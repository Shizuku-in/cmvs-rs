//! The page graph and the settings found on it.
//!
//! Nodes are settled pages, or a message wait inside the scenario. Every trial starts a
//! fresh session, replays the steps that reached a node and then tries one action there:
//! each enabled button, and cancel. Pages are identified by their signature only, so no
//! page needs a name; a control is a setting control when pressing it changes the
//! player-facing settings.
use super::{
    robustness::play_route,
    route::{Runner, scenario_entry},
};
use crate::{
    device::Action,
    json::Json,
    observe::{Observation, PageSignature, Settle, variables_digest},
    report::{CheckResult, Level, Outcome},
    scenario::{STABLE_FRAMES, Scenario, StartLog, WaitPoint, drive_to_start, follow, view_digest},
    session::{Session, Stop},
};
use std::fs;

/// Steps from a node before the graph stops expanding.
const MAX_DEPTH: usize = 3;
/// Trials per graph; each replays a path from a fresh start.
const MAX_TRIALS: usize = 400;
/// Frames an action gets to settle on a page or reach a message wait.
const STEP_FRAMES: u64 = 3600;
/// Buttons tried per page; larger pages are sampled evenly.
const MAX_BUTTONS: usize = 64;
/// The message wait the in-scenario root stands at.
const ROOT_WAIT: usize = 10;
/// Cancel presses used to back out of a settings page before saving preferences.
const BACK_OUT: usize = 3;
/// Message waits timed for the text-speed effect.
const SPEED_WAITS: usize = 10;

const P1: &str = "every reachable page is traversed from the start page and the scenario";
const P2: &str = "leaving a page and coming back restores what was shown";
const P3: &str = "quit and return-to-title controls end cleanly";
const P4: &str = "pages offer no fewer enabled buttons after a cleared route";
const C1: &str = "controls that change player-facing settings are found";
const C2: &str = "a changed setting survives a restart";
const C3: &str = "a changed text speed changes how long text takes";
const C4: &str = "a control restores the settings a page started with";

/// User-directory files (name, bytes) to start a session with.
type UserFiles = Vec<(String, Vec<u8>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Enter the scenario and stop at message wait `ROOT_WAIT`.
    Scenario,
    Press(usize, usize),
    Cancel,
}

impl Step {
    fn text(&self) -> String {
        match self {
            Self::Scenario => "scenario".into(),
            Self::Press(g, b) => format!("press {g}:{b}"),
            Self::Cancel => "cancel".into(),
        }
    }
}

/// Where one step left the session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Page(PageSignature),
    Wait(WaitPoint),
}

impl Place {
    fn key(&self) -> String {
        match self {
            // Coarse: positions and enabled states change with slider values and unlocks,
            // the group layout of a page does not.
            Self::Page(p) => {
                let groups: Vec<_> = p.groups.iter().map(|g| (g.slot, g.buttons)).collect();
                // List visibility changes while scrolling or switching categories.
                // Keep those states distinct so their newly visible controls are tried.
                let lists: Vec<_> = p
                    .groups
                    .iter()
                    .filter(|g| g.slot >= cmvs_core::buttons::BUTTON_GROUPS)
                    .map(|g| (g.slot, g.layout))
                    .collect();
                format!("page {} {groups:?} {lists:?}", p.script.escape_ascii())
            }
            Self::Wait(w) => format!("wait {} {:x} {}", w.script, w.pc, w.text),
        }
    }
    fn json(&self) -> Json {
        match self {
            Self::Page(p) => p.json(),
            Self::Wait(w) => Json::obj([("wait", w.json())]),
        }
    }
}

/// What a session looked like at a place.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    place: Place,
    view: String,
    variables: String,
    settings: String,
}

fn capture(session: &Session<'_>, place: Place) -> State {
    let o = Observation::of(session.startup(), session.yields());
    State {
        place,
        view: view_digest(session, &o),
        variables: variables_digest(session.startup()),
        settings: session.settings_fingerprint(),
    }
}

#[derive(Debug, Clone)]
enum Arrival {
    State(State),
    Quit,
    Timeout,
    Stopped(Stop),
}

fn stop_outcome(stop: Stop) -> Outcome {
    match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    }
}

/// Applies one step and waits for a settled page or a message wait.
fn apply(session: &mut Session<'_>, entry: (usize, usize), step: &Step) -> Arrival {
    let result = (|| -> Result<Arrival, Stop> {
        let action = match step {
            Step::Scenario => {
                // Settles the start page again, presses the entry and plays to the root wait.
                let mut scenario = Scenario::begin(session, entry, false)?;
                let points = follow(session, &mut scenario, ROOT_WAIT)?;
                return Ok(match points.last() {
                    Some(point) => Arrival::State(capture(session, Place::Wait(point.clone()))),
                    None => Arrival::Timeout,
                });
            }
            Step::Press(g, b) => Action::Press {
                group: *g,
                button: *b,
            },
            Step::Cancel => Action::Cancel,
        };
        session.act(action).map_err(Stop::Error)?;
        let mut settle = Settle::new(STABLE_FRAMES);
        let from = session.yields();
        while session.yields() - from < STEP_FRAMES {
            let o = session.frame()?;
            if session.busy() {
                continue;
            }
            if o.waiting_for_advance() {
                let point = WaitPoint::of(session, &o);
                return Ok(Arrival::State(capture(session, Place::Wait(point))));
            }
            if let Some(page) = settle.feed(&o) {
                let page = page.clone();
                return Ok(Arrival::State(capture(session, Place::Page(page))));
            }
        }
        Ok(Arrival::Timeout)
    })();
    match result {
        Ok(arrival) => arrival,
        Err(Stop::Quit) => Arrival::Quit,
        Err(stop) => Arrival::Stopped(stop),
    }
}

/// A fresh session at the start page with `path` replayed; `None` when the replay no
/// longer reaches the recorded places.
fn replay<'t>(
    runner: &Runner<'t>,
    files: &[(&str, &[u8])],
    entry: (usize, usize),
    path: &[(Step, String)],
) -> Result<Option<(Session<'t>, State)>, Stop> {
    let mut session = runner.session_with("page", files).map_err(Stop::Error)?;
    let start = drive_to_start(&mut session, &mut StartLog::default())?;
    let mut state = capture(&session, Place::Page(start));
    for (step, expected) in path {
        match apply(&mut session, entry, step) {
            Arrival::State(next) if next.place.key() == *expected => state = next,
            Arrival::Stopped(stop) => return Err(stop),
            _ => return Ok(None),
        }
    }
    Ok(Some((session, state)))
}

#[derive(Debug, Clone)]
struct Node {
    /// Readable name from the manifest, if it labels this page.
    label: Option<String>,
    path: Vec<(Step, String)>,
    state: State,
    /// Reached from the in-scenario root.
    in_scenario: bool,
}

#[derive(Debug, Clone)]
struct Edge {
    from: usize,
    step: Step,
    to: Option<usize>,
    arrival: Arrival,
    /// For a press that stays on its page: the settings after leaving it with cancel, since
    /// many pages apply their values on the way out.
    left: Option<String>,
}

#[derive(Debug, Default)]
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    trials: usize,
    diverged: usize,
    truncated: bool,
    stop: Option<Stop>,
}

fn actions_at(session: &Session<'_>) -> Vec<Step> {
    let mut steps: Vec<Step> = crate::controls::enabled(session.startup())
        .into_iter()
        .map(|(g, b)| Step::Press(g, b))
        .collect();
    if steps.len() > MAX_BUTTONS {
        let total = steps.len();
        steps = (0..MAX_BUTTONS)
            .map(|i| steps[i * total / MAX_BUTTONS].clone())
            .collect();
    }
    steps.push(Step::Cancel);
    steps
}

/// Breadth-first traversal from the start page and, with `scenario`, from the root wait.
fn build(
    runner: &Runner<'_>,
    files: &[(&str, &[u8])],
    entry: (usize, usize),
    scenario: bool,
    depth_limit: usize,
) -> Graph {
    let mut graph = Graph::default();
    let start = match replay(runner, files, entry, &[]) {
        Ok(Some((_, state))) => state,
        Ok(None) => return graph,
        Err(stop) => {
            graph.stop = Some(stop);
            return graph;
        }
    };
    graph.nodes.push(Node {
        label: None,
        path: Vec::new(),
        state: start,
        in_scenario: false,
    });
    if scenario {
        let root = replay(runner, files, entry, &[]).map(|r| {
            r.and_then(
                |(mut session, _)| match apply(&mut session, entry, &Step::Scenario) {
                    Arrival::State(state) => Some(state),
                    _ => None,
                },
            )
        });
        match root {
            Ok(Some(state)) => graph.nodes.push(Node {
                label: None,
                path: vec![(Step::Scenario, state.place.key())],
                state,
                in_scenario: true,
            }),
            Ok(None) => {}
            Err(stop) => graph.stop = Some(stop),
        }
    }
    let mut next = 0;
    while next < graph.nodes.len() {
        let node = graph.nodes[next].clone();
        let index = next;
        next += 1;
        if node.path.len() >= depth_limit + usize::from(node.in_scenario) {
            continue;
        }
        let steps = match replay(runner, files, entry, &node.path) {
            Ok(Some((session, _))) => actions_at(&session),
            Ok(None) => {
                graph.diverged += 1;
                continue;
            }
            Err(stop) => {
                graph.stop.get_or_insert(stop);
                continue;
            }
        };
        for step in steps {
            if graph.trials >= MAX_TRIALS {
                graph.truncated = true;
                return graph;
            }
            graph.trials += 1;
            let (arrival, left) = match replay(runner, files, entry, &node.path) {
                Ok(Some((mut session, _))) => {
                    let arrival = apply(&mut session, entry, &step);
                    let stays = matches!((&arrival, &step), (Arrival::State(s), Step::Press(..))
                        if s.place.key() == node.state.place.key());
                    let left = if stays {
                        match apply(&mut session, entry, &Step::Cancel) {
                            Arrival::State(after) => Some(after.settings),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    (arrival, left)
                }
                Ok(None) => {
                    graph.diverged += 1;
                    continue;
                }
                Err(stop) => (Arrival::Stopped(stop), None),
            };
            let to = match &arrival {
                Arrival::State(state) => {
                    let key = state.place.key();
                    match graph.nodes.iter().position(|n| n.state.place.key() == key) {
                        Some(found) => Some(found),
                        None => {
                            let mut path = node.path.clone();
                            path.push((step.clone(), key));
                            graph.nodes.push(Node {
                                label: None,
                                path,
                                state: state.clone(),
                                in_scenario: node.in_scenario,
                            });
                            Some(graph.nodes.len() - 1)
                        }
                    }
                }
                _ => None,
            };
            if let Arrival::Stopped(stop) = &arrival {
                graph.stop.get_or_insert(stop.clone());
            }
            graph.edges.push(Edge {
                from: index,
                step,
                to,
                arrival,
                left,
            });
        }
    }
    graph
}

impl Graph {
    fn json(&self) -> Json {
        Json::obj([
            ("pages", Json::from(self.nodes.len())),
            ("edges", self.edges.len().into()),
            ("trials", self.trials.into()),
            ("replays_diverged", self.diverged.into()),
            ("truncated", self.truncated.into()),
            (
                "nodes",
                Json::arr(self.nodes.iter().enumerate().map(|(i, n)| {
                    Json::obj([
                        ("index", Json::from(i)),
                        ("in_scenario", n.in_scenario.into()),
                        (
                            "path",
                            Json::arr(n.path.iter().map(|(s, _)| Json::from(s.text()))),
                        ),
                        ("place", n.state.place.json()),
                        ("label", n.label.clone().into()),
                    ])
                })),
            ),
            (
                "edges",
                Json::arr(self.edges.iter().map(|e| {
                    let to = match (&e.to, &e.arrival) {
                        (Some(to), _) => Json::from(*to),
                        (None, Arrival::Quit) => "quit".into(),
                        (None, Arrival::Timeout) => "timeout".into(),
                        (None, Arrival::Stopped(stop)) => {
                            format!("stopped: {}", stop.describe()).into()
                        }
                        (None, Arrival::State(_)) => Json::Null,
                    };
                    Json::obj([
                        ("from", Json::from(e.from)),
                        ("step", e.step.text().into()),
                        ("to", to),
                    ])
                })),
            ),
        ])
    }
    fn arrived<'e>(&self, edge: &'e Edge) -> Option<&'e State> {
        match &edge.arrival {
            Arrival::State(state) => Some(state),
            _ => None,
        }
    }
}

/// Names pages the manifest labels, by script and group shape.
fn label_pages(graph: &mut Graph, manifest: Option<&crate::manifest::Manifest>) {
    let Some(manifest) = manifest else {
        return;
    };
    for node in &mut graph.nodes {
        if let Place::Page(page) = &node.state.place {
            let groups: Vec<_> = page.groups.iter().map(|g| (g.slot, g.buttons)).collect();
            let shape = crate::manifest::shape_text(&groups);
            node.label = manifest.page_label(&page.script.escape_ascii().to_string(), &shape);
        }
    }
}

fn graph_outcome(graph: &Graph) -> Option<Outcome> {
    graph.stop.clone().map(|stop| match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    })
}

/// P1 to P3 and C1 to C4 share one graph built from the game's own system save.
pub fn pages_and_settings(runner: &Runner<'_>, wanted: &[&str]) -> Vec<CheckResult> {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => {
            return wanted
                .iter()
                .map(|id| CheckResult::new(static_id(id), title(id), outcome.clone()))
                .collect();
        }
    };
    let mut graph = build(runner, &[], entry, true, MAX_DEPTH);
    label_pages(&mut graph, runner.manifest);
    let mut results = Vec::new();
    for id in wanted {
        let result = match *id {
            "P1" => traversal(&graph),
            "P2" => returns(&graph),
            "P3" => destructive(&graph),
            "C1" => controls(&graph),
            "C2" => persistence(runner, entry, &graph),
            "C3" => text_speed(runner, entry, &graph),
            "C4" => defaults(runner, entry, &graph),
            _ => continue,
        };
        results.push(result);
    }
    results
}

fn static_id(id: &str) -> &'static str {
    ["P1", "P2", "P3", "P4", "C1", "C2", "C3", "C4"]
        .into_iter()
        .find(|c| *c == id)
        .unwrap_or("P1")
}

fn title(id: &str) -> &'static str {
    match id {
        "P1" => P1,
        "P2" => P2,
        "P3" => P3,
        "P4" => P4,
        "C1" => C1,
        "C2" => C2,
        "C3" => C3,
        _ => C4,
    }
}

fn traversal(graph: &Graph) -> CheckResult {
    let outcome = graph_outcome(graph).unwrap_or_else(|| {
        if graph.nodes.len() < 2 {
            Outcome::Inconclusive("no page beyond the start page was reached".into())
        } else if graph.truncated {
            Outcome::Inconclusive(format!("the trial budget of {MAX_TRIALS} ran out"))
        } else {
            Outcome::Pass(Level::L0)
        }
    });
    CheckResult::new("P1", P1, outcome).with(graph.json())
}

fn returns(graph: &Graph) -> CheckResult {
    let mut checked = Vec::new();
    let mut broken = 0;
    for edge in &graph.edges {
        let (Some(to), Some(_)) = (edge.to, graph.arrived(edge)) else {
            continue;
        };
        if to == edge.from {
            continue;
        }
        // An edge back from `to` to `from` closes the round trip.
        for back in graph
            .edges
            .iter()
            .filter(|b| b.from == to && b.to == Some(edge.from))
        {
            let Some(arrived) = graph.arrived(back) else {
                continue;
            };
            let original = &graph.nodes[edge.from].state;
            let view = arrived.view == original.view;
            let variables = arrived.variables == original.variables;
            broken += usize::from(!view);
            checked.push(Json::obj([
                ("from", Json::from(edge.from)),
                ("to", to.into()),
                ("there", edge.step.text().into()),
                ("back", back.step.text().into()),
                ("view_restored", view.into()),
                ("variables_restored", variables.into()),
            ]));
        }
    }
    let outcome = if checked.is_empty() {
        Outcome::Inconclusive("no page could be left and returned to".into())
    } else if broken == 0 {
        Outcome::Pass(Level::L1)
    } else {
        Outcome::Fail(format!("{broken} round trips changed what was shown"))
    };
    CheckResult::new("P2", P2, outcome).with(Json::obj([("round_trips", Json::Arr(checked))]))
}

fn destructive(graph: &Graph) -> CheckResult {
    let found: Vec<Json> = graph
        .edges
        .iter()
        .filter(|e| {
            matches!(e.arrival, Arrival::Quit)
                || (graph.nodes[e.from].in_scenario && e.to == Some(0))
        })
        .map(|e| {
            Json::obj([
                ("from", Json::from(e.from)),
                ("step", e.step.text().into()),
                (
                    "effect",
                    if matches!(e.arrival, Arrival::Quit) {
                        "quit"
                    } else {
                        "start page"
                    }
                    .into(),
                ),
            ])
        })
        .collect();
    let outcome = graph_outcome(graph).unwrap_or(if found.is_empty() {
        Outcome::Inconclusive("no quit or return-to-title control was found".into())
    } else {
        Outcome::Pass(Level::L0)
    });
    CheckResult::new("P3", P3, outcome).with(Json::obj([("controls", Json::Arr(found))]))
}

/// The settings an edge changed to: at arrival, or after leaving the page it stayed on.
fn changed_settings<'g>(graph: &'g Graph, edge: &'g Edge) -> Option<&'g str> {
    let before = &graph.nodes[edge.from].state.settings;
    graph
        .arrived(edge)
        .map(|s| s.settings.as_str())
        .filter(|s| *s != before)
        .or(edge.left.as_deref().filter(|s| *s != before))
}

/// Edges that changed the player-facing settings.
fn setting_edges(graph: &Graph) -> Vec<&Edge> {
    graph
        .edges
        .iter()
        .filter(|e| changed_settings(graph, e).is_some())
        .collect()
}

fn changed_fields(before: &str, after: &str) -> String {
    let split = |s: &str| -> Vec<String> {
        s.split([',', '{', '}', ' '])
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (a, b) = (split(before), split(after));
    let mut changes = Vec::new();
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        if x != y {
            let field = a[..i]
                .iter()
                .rev()
                .find(|t| t.ends_with(':'))
                .map_or("?", |t| t.as_str());
            changes.push(format!("{field} {x}->{y}"));
        }
    }
    changes.join(", ")
}

fn controls(graph: &Graph) -> CheckResult {
    let edges = setting_edges(graph);
    let rows: Vec<Json> = edges
        .iter()
        .map(|e| {
            let after = changed_settings(graph, e).expect("filtered");
            Json::obj([
                ("page", Json::from(e.from)),
                ("step", e.step.text().into()),
                (
                    "changed",
                    changed_fields(&graph.nodes[e.from].state.settings, after).into(),
                ),
            ])
        })
        .collect();
    let outcome = graph_outcome(graph).unwrap_or(if rows.is_empty() {
        Outcome::Inconclusive("no control changed a player-facing setting".into())
    } else {
        Outcome::Pass(Level::L0)
    });
    CheckResult::new("C1", C1, outcome).with(Json::obj([("controls", Json::Arr(rows))]))
}

/// Replays to a setting control, applies it, backs out and saves preferences as a graceful
/// exit would; returns the changed fingerprint and the user files to restart with.
fn apply_and_save(
    runner: &Runner<'_>,
    entry: (usize, usize),
    graph: &Graph,
    edge: &Edge,
) -> Result<(String, UserFiles), Outcome> {
    let path = &graph.nodes[edge.from].path;
    let (mut session, _) = replay(runner, &[], entry, path)
        .map_err(stop_outcome)?
        .ok_or_else(|| Outcome::Inconclusive("the settings page no longer replays".into()))?;
    if !matches!(apply(&mut session, entry, &edge.step), Arrival::State(_)) {
        return Err(Outcome::Inconclusive(
            "the control no longer settles".into(),
        ));
    }
    for _ in 0..BACK_OUT {
        if let Arrival::Stopped(stop) = apply(&mut session, entry, &Step::Cancel) {
            return Err(stop_outcome(stop));
        }
    }
    let changed = session.settings_fingerprint();
    session.save_preferences().map_err(Outcome::Fail)?;
    let files = ["system.dat", "cmvs.cfg"]
        .into_iter()
        .filter_map(|name| {
            fs::read(session.user_dir().join(name))
                .ok()
                .map(|b| (name.to_owned(), b))
        })
        .collect();
    Ok((changed, files))
}

fn persistence(runner: &Runner<'_>, entry: (usize, usize), graph: &Graph) -> CheckResult {
    let Some(edge) = setting_edges(graph).into_iter().next() else {
        return CheckResult::new(
            "C2",
            C2,
            graph_outcome(graph)
                .unwrap_or(Outcome::Inconclusive("no setting control was found".into())),
        );
    };
    let result = (|| -> Result<CheckResult, Outcome> {
        let (changed, files) = apply_and_save(runner, entry, graph, edge)?;
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let mut restarted = runner
            .session_with("settings-restart", &borrowed)
            .map_err(Outcome::Fail)?;
        drive_to_start(&mut restarted, &mut StartLog::default()).map_err(stop_outcome)?;
        let after = restarted.settings_fingerprint();
        let original = &graph.nodes[edge.from].state.settings;
        let outcome = if after == changed {
            Outcome::Pass(Level::L1)
        } else if after == *original {
            Outcome::Fail("the setting reverted after the restart".into())
        } else {
            Outcome::Fail("the settings after the restart match neither value".into())
        };
        Ok(CheckResult::new("C2", C2, outcome).with(Json::obj([
            ("control", Json::from(edge.step.text())),
            ("changed", changed_fields(original, &changed).into()),
            ("after_restart", changed_fields(original, &after).into()),
            (
                "saved_files",
                Json::arr(files.iter().map(|(n, _)| Json::from(n.as_str()))),
            ),
        ])))
    })();
    result.unwrap_or_else(|outcome| CheckResult::new("C2", C2, outcome))
}

/// Frames from entering the scenario to its `SPEED_WAITS`-th message wait.
fn frames_to_waits(
    runner: &Runner<'_>,
    entry: (usize, usize),
    files: &[(&str, &[u8])],
) -> Result<u64, Outcome> {
    let mut session = runner
        .session_with("text-speed", files)
        .map_err(Outcome::Fail)?;
    let mut scenario = Scenario::begin(&mut session, entry, false).map_err(stop_outcome)?;
    let from = session.yields();
    let points = follow(&mut session, &mut scenario, SPEED_WAITS).map_err(stop_outcome)?;
    if points.len() < SPEED_WAITS {
        return Err(Outcome::Inconclusive(
            "too few message waits to time".into(),
        ));
    }
    Ok(session.yields() - from)
}

fn text_speed(runner: &Runner<'_>, entry: (usize, usize), graph: &Graph) -> CheckResult {
    let speed = setting_edges(graph).into_iter().find(|e| {
        let after = changed_settings(graph, e).expect("filtered");
        changed_fields(&graph.nodes[e.from].state.settings, after).contains("speed:")
    });
    let Some(edge) = speed else {
        return CheckResult::new(
            "C3",
            C3,
            graph_outcome(graph).unwrap_or(Outcome::Inconclusive(
                "no control changed the text speed".into(),
            )),
        );
    };
    let result = (|| -> Result<CheckResult, Outcome> {
        let (_, files) = apply_and_save(runner, entry, graph, edge)?;
        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let base = frames_to_waits(runner, entry, &[])?;
        let changed = frames_to_waits(runner, entry, &borrowed)?;
        let outcome = if base != changed {
            Outcome::Pass(Level::L1)
        } else {
            Outcome::Fail("the text speed control did not change how long text takes".into())
        };
        Ok(CheckResult::new("C3", C3, outcome).with(Json::obj([
            ("control", Json::from(edge.step.text())),
            ("frames_default", base.into()),
            ("frames_changed", changed.into()),
        ])))
    })();
    result.unwrap_or_else(|outcome| CheckResult::new("C3", C3, outcome))
}

fn defaults(runner: &Runner<'_>, entry: (usize, usize), graph: &Graph) -> CheckResult {
    let Some(edge) = setting_edges(graph).into_iter().next() else {
        return CheckResult::new(
            "C4",
            C4,
            graph_outcome(graph)
                .unwrap_or(Outcome::Inconclusive("no setting control was found".into())),
        );
    };
    let original = graph.nodes[edge.from].state.settings.clone();
    let path = graph.nodes[edge.from].path.clone();
    let candidates: Vec<Step> = graph
        .edges
        .iter()
        .filter(|e| e.from == edge.from && matches!(e.step, Step::Press(..)))
        .map(|e| e.step.clone())
        .collect();
    let mut tried = 0;
    for step in candidates {
        tried += 1;
        let Ok(Some((mut session, _))) = replay(runner, &[], entry, &path) else {
            continue;
        };
        if !matches!(apply(&mut session, entry, &edge.step), Arrival::State(_)) {
            continue;
        }
        let Arrival::State(after) = apply(&mut session, entry, &step) else {
            continue;
        };
        let restored = after.settings == original
            || matches!(apply(&mut session, entry, &Step::Cancel), Arrival::State(left) if left.settings == original);
        if restored {
            return CheckResult::new("C4", C4, Outcome::Pass(Level::L1)).with(Json::obj([
                ("changed_by", Json::from(edge.step.text())),
                ("restored_by", step.text().into()),
            ]));
        }
    }
    CheckResult::new(
        "C4",
        C4,
        Outcome::Inconclusive("no other control on the settings page restored the settings".into()),
    )
    .with(Json::obj([
        ("changed_by", Json::from(edge.step.text())),
        ("tried", tried.into()),
    ]))
}

/// P4: the start page graph from the game's template and from the system save of a cleared
/// route; matching pages must not lose enabled buttons.
pub fn unlocks(runner: &Runner<'_>) -> CheckResult {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("P4", P4, outcome),
    };
    let cleared = match play_route(runner, entry, None, "unlock-route") {
        Ok(play) => play.system_save,
        Err(outcome) => return CheckResult::new("P4", P4, outcome),
    };
    let Some(cleared) = cleared else {
        return CheckResult::new(
            "P4",
            P4,
            Outcome::Fail("the cleared route wrote no system save".into()),
        );
    };
    let before = build(runner, &[], entry, false, 2);
    let after = build(runner, &[("system.dat", &cleared)], entry, false, 2);
    // Groups match by script, slot and size; a page's other groups may grow with unlocks.
    type Groups = std::collections::BTreeMap<(String, usize, usize), usize>;
    let groups = |graph: &Graph| -> Groups {
        let mut map = Groups::new();
        for node in &graph.nodes {
            if let Place::Page(p) = &node.state.place {
                let script = p.script.escape_ascii().to_string();
                for g in &p.groups {
                    let enabled = map.entry((script.clone(), g.slot, g.buttons)).or_default();
                    *enabled = (*enabled).max(g.enabled);
                }
            }
        }
        map
    };
    let (a, b) = (groups(&before), groups(&after));
    // The largest group per script and slot, to see groups that grew.
    let sizes = |m: &Groups| -> std::collections::BTreeMap<(String, usize), usize> {
        let mut sizes = std::collections::BTreeMap::new();
        for (script, slot, buttons) in m.keys() {
            let size = sizes.entry((script.clone(), *slot)).or_insert(0);
            *size = (*size).max(*buttons);
        }
        sizes
    };
    let (sa, sb) = (sizes(&a), sizes(&b));
    let mut rows = Vec::new();
    let mut lost = 0;
    for ((script, slot), size) in &sa {
        let Some(later) = sb.get(&(script.clone(), *slot)) else {
            continue;
        };
        let enabled_before = a.get(&(script.clone(), *slot, *size)).copied().unwrap_or(0);
        let enabled_after = b
            .get(&(script.clone(), *slot, *later))
            .copied()
            .unwrap_or(0);
        let shrank = later < size || (later == size && enabled_after < enabled_before);
        lost += usize::from(shrank);
        rows.push(Json::obj([
            ("script", Json::from(script.as_str())),
            ("group", (*slot).into()),
            ("buttons_before", (*size).into()),
            ("buttons_after", (*later).into()),
            ("enabled_before", enabled_before.into()),
            ("enabled_after", enabled_after.into()),
        ]));
    }
    let outcome = graph_outcome(&before)
        .or_else(|| graph_outcome(&after))
        .unwrap_or(if rows.is_empty() {
            Outcome::Inconclusive(
                "no button group appears both before and after the cleared route".into(),
            )
        } else if lost == 0 {
            Outcome::Pass(Level::L1)
        } else {
            Outcome::Fail(format!("{lost} button groups lost buttons"))
        });
    CheckResult::new("P4", P4, outcome).with(Json::obj([
        ("pages_before", Json::from(before.nodes.len())),
        ("pages_after", after.nodes.len().into()),
        ("groups", Json::Arr(rows)),
    ]))
}

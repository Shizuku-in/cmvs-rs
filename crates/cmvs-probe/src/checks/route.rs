//! Route exploration over the choice tree, and coverage of executed script addresses.
//!
//! Every route is replayed from a fresh start with its recorded choice prefix; no state is
//! copied between runs. Nothing names a script: the scenario entry is the first start-page
//! button after which a message wait appears, and a choice is a settled button group that
//! was never visible during a message wait.
use super::{
    startup::{STABLE_FRAMES, StartLog, drive_to_start},
    static_checks::ScriptInventory,
};
use crate::{
    device::Action,
    json::Json,
    observe::{GroupShape, Observation, PageSignature, Settle, variables_digest},
    report::{CheckResult, Level, Outcome},
    session::{Coverage, Options, Session, Stop},
    target::Target,
};
use cmvs_core::ps3::Script;
use std::{
    cell::Cell,
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::Path,
};

/// Virtual frames a start-page button gets to show its effect.
const ENTRY_FRAMES: u64 = 3600;
/// Frames a choice candidate must stay unchanged before it is taken.
const CHOICE_FRAMES: u32 = 30;
/// Quiet frames after which a confirm-to-continue loop gets one advance.
const NUDGE_FRAMES: u64 = 300;

/// Creates sessions with fresh user directories under the output directory.
pub struct Runner<'t> {
    pub target: &'t Target,
    pub script: &'t Script,
    pub options: &'t Options,
    pub font: Option<&'t cmvs_desktop::font::FontRenderer>,
    pub out: &'t Path,
    count: Cell<usize>,
}

impl<'t> Runner<'t> {
    pub fn new(
        target: &'t Target,
        script: &'t Script,
        options: &'t Options,
        font: Option<&'t cmvs_desktop::font::FontRenderer>,
        out: &'t Path,
    ) -> Self {
        Self {
            target,
            script,
            options,
            font,
            out,
            count: Cell::new(0),
        }
    }
    pub fn session(&self, label: &str) -> Result<Session<'t>, String> {
        let n = self.count.get();
        self.count.set(n + 1);
        let user = self.out.join("users").join(format!("{n:03}-{label}"));
        fs::create_dir_all(&user).map_err(|e| e.to_string())?;
        Session::start(self.target, self.script, &user, self.options, self.font)
    }
}

/// What pressing one start-page button led to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Effect {
    Scenario,
    Page(String),
    Unchanged,
    Quit,
    Stopped(Stop),
    Timeout,
}

impl Effect {
    fn json(&self) -> Json {
        match self {
            Self::Scenario => "scenario".into(),
            Self::Page(digest) => format!("page {digest}").into(),
            Self::Unchanged => "unchanged".into(),
            Self::Quit => "quit".into(),
            Self::Stopped(stop) => format!("stopped: {}", stop.describe()).into(),
            Self::Timeout => "timeout".into(),
        }
    }
}

fn enabled_buttons(page: &PageSignature, session: &Session<'_>) -> Vec<(usize, usize)> {
    page.groups
        .iter()
        .flat_map(|g| {
            let group = session.startup().buttons().group(g.slot);
            group
                .map(|grp| {
                    grp.buttons
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| b.enabled)
                        .map(|(i, _)| (g.slot, i))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

/// Presses `button` on a fresh start page and classifies the result.
fn try_button(runner: &Runner<'_>, button: (usize, usize)) -> Result<Effect, String> {
    let mut session = runner.session("start-button")?;
    let start = match drive_to_start(&mut session, &mut StartLog::default()) {
        Ok(page) => page,
        Err(stop) => return Ok(Effect::Stopped(stop)),
    };
    session.act(Action::Press {
        group: button.0,
        button: button.1,
    })?;
    let mut settle = Settle::new(STABLE_FRAMES);
    let from = session.yields();
    while session.yields() - from < ENTRY_FRAMES {
        let observation = match session.frame() {
            Ok(o) => o,
            Err(Stop::Quit) => return Ok(Effect::Quit),
            Err(stop) => return Ok(Effect::Stopped(stop)),
        };
        if observation.waiting_for_advance() {
            return Ok(Effect::Scenario);
        }
        if session.busy() {
            continue;
        }
        if let Some(page) = settle.feed(&observation) {
            return Ok(if *page == start {
                Effect::Unchanged
            } else {
                Effect::Page(page.digest())
            });
        }
    }
    Ok(Effect::Timeout)
}

#[derive(Debug, Clone)]
struct ChoicePoint {
    script: String,
    pc: usize,
    key: String,
    slot: usize,
    buttons: usize,
    enabled: Vec<usize>,
    picked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    /// The start page settled again.
    Start,
    /// A choice point already expanded by route `n` (same script, PC and variables).
    Merged(usize),
    Stopped(Stop),
}

#[derive(Debug)]
struct Route {
    path: Vec<usize>,
    choices: Vec<ChoicePoint>,
    end: End,
    steps: u64,
    yields: u64,
    advances: u64,
    nudges: u64,
    /// Prefixes discovered by this route, still to run.
    branches: Vec<Vec<usize>>,
    events: Vec<Json>,
    end_page: Option<PageSignature>,
}

impl Route {
    fn json(&self) -> Json {
        let end = match &self.end {
            End::Start => Json::from("start"),
            End::Merged(n) => format!("merged with route {n}").into(),
            End::Stopped(stop) => stop.describe().into(),
        };
        Json::obj([
            (
                "path",
                Json::from(
                    self.path
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join("."),
                ),
            ),
            ("end", end),
            (
                "end_page",
                self.end_page
                    .as_ref()
                    .map_or(Json::Null, PageSignature::json),
            ),
            ("steps", self.steps.into()),
            ("yields", self.yields.into()),
            ("advances", self.advances.into()),
            ("nudges", self.nudges.into()),
            (
                "choices",
                Json::arr(self.choices.iter().map(|c| {
                    Json::obj([
                        ("script", Json::from(c.script.as_str())),
                        ("pc", Json::hex(c.pc as u64)),
                        ("group", c.slot.into()),
                        ("buttons", c.buttons.into()),
                        ("enabled", Json::arr(c.enabled.iter().map(|&i| i.into()))),
                        ("picked", c.picked.into()),
                        ("key", c.key.as_str().into()),
                    ])
                })),
            ),
        ])
    }
}

/// Interactive groups never seen during a message wait.
fn candidates(o: &Observation, ambient: &HashSet<(usize, [u8; 32])>) -> Vec<GroupShape> {
    o.page
        .groups
        .iter()
        .filter(|g| g.enabled > 0 && !ambient.contains(&(g.slot, g.layout)))
        .cloned()
        .collect()
}

fn run_route(
    runner: &Runner<'_>,
    entry: (usize, usize),
    prefix: &[usize],
    known: &mut BTreeMap<String, usize>,
    index: usize,
    coverage: &mut Coverage,
) -> Result<Route, String> {
    let mut session = runner.session("route")?;
    session.record_coverage();
    let mut route = Route {
        path: Vec::new(),
        choices: Vec::new(),
        end: End::Start,
        steps: 0,
        yields: 0,
        advances: 0,
        nudges: 0,
        branches: Vec::new(),
        events: Vec::new(),
        end_page: None,
    };
    let end = explore(&mut session, entry, prefix, known, index, &mut route);
    route.end = end;
    route.steps = session.startup().steps();
    route.yields = session.yields();
    if let Some(c) = session.take_coverage() {
        coverage.merge(c);
    }
    route.events = session.take_events();
    Ok(route)
}

fn explore(
    session: &mut Session<'_>,
    entry: (usize, usize),
    prefix: &[usize],
    known: &mut BTreeMap<String, usize>,
    index: usize,
    route: &mut Route,
) -> End {
    let start = match drive_to_start(session, &mut StartLog::default()) {
        Ok(page) => page,
        Err(stop) => return End::Stopped(stop),
    };
    let press = Action::Press {
        group: entry.0,
        button: entry.1,
    };
    if let Err(e) = session.act(press) {
        return End::Stopped(Stop::Error(e));
    }
    let mut ambient: HashSet<(usize, [u8; 32])> = HashSet::new();
    // Choice shapes already answered; cleared once no candidate is visible.
    let mut answered: HashSet<(usize, [u8; 32])> = HashSet::new();
    let (mut pending, mut pending_frames) = (Vec::<GroupShape>::new(), 0u32);
    let mut settle = Settle::new(STABLE_FRAMES);
    let mut quiet_since = session.yields();
    let mut in_scenario = false;
    loop {
        let o = match session.frame() {
            Ok(o) => o,
            Err(stop) => return End::Stopped(stop),
        };
        if o.waiting_for_advance() {
            if !in_scenario && session_skip(session) {
                session.note("skip-held", Json::Null);
            }
            in_scenario = true;
            for g in &o.page.groups {
                ambient.insert((g.slot, g.layout));
            }
            if !session.busy() {
                let _ = session.act(Action::Advance);
                route.advances += 1;
            }
            quiet_since = o.yields;
            pending.clear();
            continue;
        }
        if session.busy() {
            continue;
        }
        // The route ends when a page settles in the start page's script. Its shape may
        // differ: finishing a route can unlock start-page buttons.
        if in_scenario
            && let Some(page) = settle.feed(&o)
            && page.script == start.script
        {
            route.end_page = Some(page.clone());
            session.note(
                "route-end",
                Json::obj([("changed", Json::from(*page != start))]),
            );
            return End::Start;
        }
        // Inside the start page's script the route is ending: wait for the page to settle
        // instead of answering its buttons as a choice (identity comparison, no literal).
        if !in_scenario || o.transition || o.script == start.script {
            continue;
        }
        let found: Vec<GroupShape> = candidates(&o, &ambient)
            .into_iter()
            .filter(|g| !answered.contains(&(g.slot, g.layout)))
            .collect();
        if candidates(&o, &ambient).is_empty() {
            answered.clear();
        }
        if !found.is_empty() && found == pending {
            pending_frames += 1;
        } else {
            pending_frames = 0;
            pending = found;
        }
        if pending_frames >= CHOICE_FRAMES {
            if pending.len() > 1 {
                let slots: Vec<_> = pending.iter().map(|g| g.slot).collect();
                return End::Stopped(Stop::Error(format!(
                    "ambiguous-choice: candidate groups {slots:?}"
                )));
            }
            let shape = pending[0].clone();
            let Some(group) = session.startup().buttons().group(shape.slot) else {
                return End::Stopped(Stop::Error("choice group vanished".into()));
            };
            let enabled: Vec<usize> = group
                .buttons
                .iter()
                .enumerate()
                .filter(|(_, b)| b.enabled)
                .map(|(i, _)| i)
                .collect();
            let buttons = group.buttons.len();
            let key = variables_digest(session.startup());
            let depth = route.choices.len();
            let picked = if let Some(&want) = prefix.get(depth) {
                if !enabled.contains(&want) {
                    return End::Stopped(Stop::Error(format!(
                        "replay diverged: choice {depth} has no enabled button {want}"
                    )));
                }
                want
            } else {
                if let Some(&first) = known.get(&key) {
                    return End::Merged(first);
                }
                known.insert(key.clone(), index);
                for &alternative in enabled.iter().skip(1) {
                    let mut branch = route.path.clone();
                    branch.push(alternative);
                    route.branches.push(branch);
                }
                enabled[0]
            };
            session.note(
                "choice",
                Json::obj([
                    ("depth", Json::from(depth)),
                    ("group", shape.slot.into()),
                    ("buttons", buttons.into()),
                    ("picked", picked.into()),
                ]),
            );
            route.choices.push(ChoicePoint {
                script: o.script.escape_ascii().to_string(),
                pc: o.pc,
                key,
                slot: shape.slot,
                buttons,
                enabled,
                picked,
            });
            route.path.push(picked);
            answered.insert((shape.slot, shape.layout));
            pending.clear();
            pending_frames = 0;
            if let Err(e) = session.act(Action::Press {
                group: shape.slot,
                button: picked,
            }) {
                return End::Stopped(Stop::Error(e));
            }
            quiet_since = o.yields;
            continue;
        }
        let movie = session.startup().movies().player().is_some();
        if pending.is_empty() && !movie && o.yields - quiet_since >= NUDGE_FRAMES {
            let _ = session.act(Action::Advance);
            route.nudges += 1;
            session.note("nudge", Json::Null);
            quiet_since = o.yields;
        }
    }
}

/// Starts holding skip once, at the first message wait, when the options ask for it.
fn session_skip(session: &mut Session<'_>) -> bool {
    if !session.route_skip() {
        return false;
    }
    match crate::device::skip_code(session.bindings()) {
        Some(code) => {
            session.hold(vec![code]);
            true
        }
        None => false,
    }
}

pub struct RouteRun {
    pub result: CheckResult,
    pub coverage: Coverage,
    pub events: Vec<Json>,
}

const R2_TITLE: &str = "choice tree explored by replay to the start page";

/// Surveys the start page, enters the scenario and explores the choice tree depth-first.
pub fn explore_routes(runner: &Runner<'_>) -> RouteRun {
    let mut coverage = Coverage::default();
    let fail = |why: String, details: Json| RouteRun {
        result: CheckResult::new("R2", R2_TITLE, Outcome::Fail(why)).with(details),
        coverage: Coverage::default(),
        events: Vec::new(),
    };
    let mut survey_session = match runner.session("start-survey") {
        Ok(s) => s,
        Err(e) => return fail(e, Json::Null),
    };
    let start = match drive_to_start(&mut survey_session, &mut StartLog::default()) {
        Ok(page) => page,
        Err(Stop::Blocked(b)) => {
            return RouteRun {
                result: CheckResult::new("R2", R2_TITLE, Outcome::Blocked(b)),
                coverage,
                events: Vec::new(),
            };
        }
        Err(stop) => return fail(stop.describe(), Json::Null),
    };
    let buttons = enabled_buttons(&start, &survey_session);
    drop(survey_session);
    let mut survey = Vec::new();
    let mut entry = None;
    let mut first_block = None;
    for &button in &buttons {
        let effect = match try_button(runner, button) {
            Ok(effect) => effect,
            Err(e) => return fail(e, Json::Null),
        };
        if let Effect::Stopped(Stop::Blocked(b)) = &effect {
            first_block.get_or_insert_with(|| b.clone());
        }
        survey.push(Json::obj([
            ("group", Json::from(button.0)),
            ("button", button.1.into()),
            ("effect", effect.json()),
        ]));
        if effect == Effect::Scenario {
            entry = Some(button);
            break;
        }
    }
    let mut details = Json::obj([
        ("start_page", start.json()),
        ("start_buttons", Json::Arr(survey)),
    ]);
    let Some(entry) = entry else {
        if let Some(blocked) = first_block {
            // A gate stop behind a start-page button hides whether it enters the scenario.
            return RouteRun {
                result: CheckResult::new("R2", R2_TITLE, Outcome::Blocked(blocked)).with(details),
                coverage,
                events: Vec::new(),
            };
        }
        return fail(
            "no start-page button leads to a message wait".into(),
            details,
        );
    };
    details.set(
        "entry",
        Json::obj([("group", Json::from(entry.0)), ("button", entry.1.into())]),
    );
    let mut frontier: Vec<Vec<usize>> = vec![Vec::new()];
    let mut known = BTreeMap::new();
    let mut routes: Vec<Route> = Vec::new();
    let mut events = Vec::new();
    while let Some(prefix) = frontier.pop() {
        if routes.len() >= runner.options.max_routes {
            frontier.push(prefix);
            break;
        }
        let route = match run_route(
            runner,
            entry,
            &prefix,
            &mut known,
            routes.len(),
            &mut coverage,
        ) {
            Ok(route) => route,
            Err(e) => return fail(e, details),
        };
        let mut route = route;
        for mut event in std::mem::take(&mut route.events) {
            event.set("route", routes.len().into());
            events.push(event);
        }
        events.push(Json::obj([
            ("kind", Json::from("route")),
            ("index", routes.len().into()),
            ("route", route.json()),
        ]));
        // Depth-first: the last discovered branch runs next.
        frontier.extend(route.branches.iter().rev().cloned());
        routes.push(route);
    }
    let stopped: Vec<&Route> = routes
        .iter()
        .filter(|r| matches!(r.end, End::Stopped(_)))
        .collect();
    let outcome = if let Some(blocked) = stopped.iter().find_map(|r| match &r.end {
        End::Stopped(Stop::Blocked(b)) => Some(b.clone()),
        _ => None,
    }) {
        Outcome::Blocked(blocked)
    } else if let Some(r) = stopped.first() {
        Outcome::Fail(match &r.end {
            End::Stopped(stop) => stop.describe(),
            _ => unreachable!(),
        })
    } else if !frontier.is_empty() {
        Outcome::Inconclusive(format!(
            "route budget reached with {} prefixes unexplored",
            frontier.len()
        ))
    } else {
        Outcome::Pass(Level::L0)
    };
    details.set("routes_run", routes.len().into());
    details.set("choice_points", known.len().into());
    details.set(
        "returned_to_start",
        routes.iter().filter(|r| r.end == End::Start).count().into(),
    );
    details.set(
        "unexplored",
        Json::arr(
            frontier
                .iter()
                .map(|p| Json::from(p.iter().map(usize::to_string).collect::<Vec<_>>().join("."))),
        ),
    );
    details.set("routes", Json::arr(routes.iter().map(Route::json)));
    RouteRun {
        result: CheckResult::new("R2", R2_TITLE, outcome).with(details),
        coverage,
        events,
    }
}

const R3_TITLE: &str = "executed addresses against statically decoded instructions";

/// Joins the addresses executed during route exploration with the statically decoded
/// instruction starts and writes `coverage.tsv`.
pub fn coverage_result(
    inventory: &ScriptInventory,
    coverage: &Coverage,
    out: &Path,
) -> CheckResult {
    let mut rows = Vec::new();
    let (mut total, mut served) = (0usize, 0usize);
    let mut unmatched = Vec::new();
    for name in coverage.scripts.keys() {
        let label = name.escape_ascii().to_string();
        if !inventory.instruction_starts.contains_key(&label) {
            unmatched.push(label);
        }
    }
    for (name, starts) in &inventory.instruction_starts {
        let hits = coverage
            .scripts
            .get(name.as_bytes())
            .map_or(0, |pcs| starts.iter().filter(|pc| pcs.contains(pc)).count());
        total += starts.len();
        served += hits;
        rows.push((name.clone(), starts.len(), hits));
    }
    let mut tsv = String::from("script\tinstructions\texecuted\n");
    for (name, count, hits) in &rows {
        tsv.push_str(&format!("{name}\t{count}\t{hits}\n"));
    }
    let written = fs::File::create_new(out.join("coverage.tsv"))
        .and_then(|mut f| f.write_all(tsv.as_bytes()));
    let outcome = match written {
        Err(e) => Outcome::Fail(format!("coverage.tsv: {e}")),
        Ok(()) if coverage.scripts.is_empty() => {
            Outcome::Inconclusive("no executed address was recorded".into())
        }
        Ok(()) => Outcome::Pass(Level::L0),
    };
    let per_mille = (served * 1000).checked_div(total).unwrap_or(0);
    CheckResult::new("R3", R3_TITLE, outcome).with(Json::obj([
        ("instructions", Json::from(total)),
        ("executed", served.into()),
        ("per_mille", per_mille.into()),
        (
            "scripts_touched",
            rows.iter().filter(|r| r.2 > 0).count().into(),
        ),
        ("scripts", rows.len().into()),
        (
            "unmatched_scripts",
            Json::arr(unmatched.into_iter().map(Json::from)),
        ),
        ("table", "coverage.tsv".into()),
    ]))
}

//! Route exploration over the choice tree, unlock rounds that carry the system save from
//! route to route, and coverage of executed script addresses.
//!
//! Every route is replayed from the start page with its recorded choice prefix; no VM
//! state is copied between runs. Nothing names a script: a scenario entry is a start-page
//! button after which a message wait appears, and a choice is a settled button group that
//! was never visible during a message wait.
use super::static_checks::ScriptInventory;
use crate::{
    device::Action,
    json::Json,
    observe::{PageSignature, Settle, variables_digest},
    report::{Blocked, CheckResult, Level, Outcome},
    scenario::{Event, STABLE_FRAMES, Scenario, StartLog, drive_to_start},
    session::{Coverage, Options, Session, Stop},
    target::Target,
};
use cmvs_core::{
    digest::{sha256, to_hex},
    ps3::Script,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

/// Virtual frames a start-page button gets to show its effect.
const ENTRY_FRAMES: u64 = 3600;
/// The engine's system save inside a user directory.
const SYSTEM_SAVE: &str = "system.dat";

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
    /// A new session in a fresh user directory. With `seed`, the directory starts with
    /// that system save; without it the engine falls back to the game's own template.
    pub fn session(&self, label: &str, seed: Option<&[u8]>) -> Result<Session<'t>, String> {
        let n = self.count.get();
        self.count.set(n + 1);
        let user = self.out.join("users").join(format!("{n:03}-{label}"));
        fs::create_dir_all(&user).map_err(|e| e.to_string())?;
        if let Some(seed) = seed {
            fs::write(user.join(SYSTEM_SAVE), seed).map_err(|e| e.to_string())?;
        }
        Session::start(self.target, self.script, &user, self.options, self.font)
    }
}

fn seed_json(seed: Option<&[u8]>) -> Json {
    seed.map_or(Json::from("game template"), |s| {
        to_hex(&sha256(s))[..16].to_owned().into()
    })
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
fn try_button(
    runner: &Runner<'_>,
    seed: Option<&[u8]>,
    button: (usize, usize),
) -> Result<Effect, String> {
    let mut session = runner.session("start-button", seed)?;
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

/// The start page and the buttons that enter the scenario.
struct Survey {
    start: PageSignature,
    entries: Vec<(usize, usize)>,
    /// The first gate stop behind a start-page button, if no button entered the scenario.
    blocked: Option<Blocked>,
    details: Json,
}

/// Presses every start-page button, each on a fresh start, and keeps those that lead to a
/// message wait. Buttons unlocked by earlier play often start further scenarios.
fn survey(runner: &Runner<'_>, seed: Option<&[u8]>) -> Result<Survey, Stop> {
    let mut session = runner.session("start-survey", seed).map_err(Stop::Error)?;
    let start = drive_to_start(&mut session, &mut StartLog::default())?;
    let buttons = enabled_buttons(&start, &session);
    drop(session);
    let (mut results, mut entries, mut blocked) = (Vec::new(), Vec::new(), None);
    for button in buttons {
        let effect = try_button(runner, seed, button).map_err(Stop::Error)?;
        if let Effect::Stopped(Stop::Blocked(b)) = &effect {
            blocked.get_or_insert_with(|| b.clone());
        }
        results.push(Json::obj([
            ("group", Json::from(button.0)),
            ("button", button.1.into()),
            ("effect", effect.json()),
        ]));
        if effect == Effect::Scenario {
            entries.push(button);
        }
    }
    let mut details = Json::obj([
        ("start_page", start.json()),
        ("start_buttons", Json::Arr(results)),
    ]);
    details.set("entries", Json::arr(entries.iter().map(|e| entry_json(*e))));
    if !entries.is_empty() {
        blocked = None;
    }
    Ok(Survey {
        start,
        entries,
        blocked,
        details,
    })
}

fn entry_json(entry: (usize, usize)) -> Json {
    Json::obj([("group", Json::from(entry.0)), ("button", entry.1.into())])
}

/// One answered choice of a route prefix, located by where the choice appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pick {
    script: String,
    pc: usize,
    button: usize,
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
    /// The start page's script showed a settled page again.
    Start,
    /// A choice point already expanded by route `n` (same script, PC and variables).
    Merged(usize),
    Stopped(Stop),
}

#[derive(Debug)]
struct Route {
    entry: (usize, usize),
    picks: Vec<Pick>,
    choices: Vec<ChoicePoint>,
    end: End,
    /// Depth at which the replayed prefix no longer matched; later choices were new.
    diverged: Option<usize>,
    steps: u64,
    yields: u64,
    advances: u64,
    nudges: u64,
    /// Prefixes discovered by this route, still to run.
    branches: Vec<Vec<Pick>>,
    events: Vec<Json>,
    end_page: Option<PageSignature>,
}

fn path_text(picks: &[Pick]) -> String {
    picks
        .iter()
        .map(|p| p.button.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

impl Route {
    fn json(&self) -> Json {
        let end = match &self.end {
            End::Start => Json::from("start"),
            End::Merged(n) => format!("merged with route {n}").into(),
            End::Stopped(stop) => stop.describe().into(),
        };
        Json::obj([
            ("entry", entry_json(self.entry)),
            ("path", Json::from(path_text(&self.picks))),
            ("end", end),
            (
                "end_page",
                self.end_page
                    .as_ref()
                    .map_or(Json::Null, PageSignature::json),
            ),
            ("diverged_at", self.diverged.into()),
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

/// Every choice point and enabled button seen so far in this probe run, by location.
#[derive(Debug, Default)]
struct Seen {
    points: BTreeMap<(String, usize), BTreeSet<usize>>,
    end_pages: BTreeSet<String>,
    start_pages: BTreeSet<String>,
}

/// What one tree exploration added to `Seen`.
#[derive(Debug, Default, Clone, Copy)]
struct Discoveries {
    points: usize,
    buttons: usize,
    end_pages: usize,
    start_pages: usize,
}

impl Discoveries {
    fn any(self) -> bool {
        self.points + self.buttons + self.end_pages + self.start_pages > 0
    }
    fn json(self) -> Json {
        Json::obj([
            ("choice_points", Json::from(self.points)),
            ("buttons", self.buttons.into()),
            ("end_pages", self.end_pages.into()),
            ("start_pages", self.start_pages.into()),
        ])
    }
}

impl Seen {
    fn choice(&mut self, script: &str, pc: usize, enabled: &[usize], found: &mut Discoveries) {
        let entry = self.points.entry((script.to_owned(), pc));
        if let std::collections::btree_map::Entry::Vacant(_) = entry {
            found.points += 1;
        }
        let set = entry.or_default();
        for &button in enabled {
            if set.insert(button) {
                found.buttons += 1;
            }
        }
    }
}

/// Per-tree state shared by its routes.
struct TreeState<'s> {
    known: BTreeMap<String, usize>,
    seen: &'s mut Seen,
    found: Discoveries,
}

fn explore(
    session: &mut Session<'_>,
    entry: (usize, usize),
    prefix: &[Pick],
    tree: &mut TreeState<'_>,
    index: usize,
    route: &mut Route,
) -> End {
    match explore_scenario(session, entry, prefix, tree, index, route) {
        Ok(end) => end,
        Err(stop) => End::Stopped(stop),
    }
}

fn explore_scenario(
    session: &mut Session<'_>,
    entry: (usize, usize),
    prefix: &[Pick],
    tree: &mut TreeState<'_>,
    index: usize,
    route: &mut Route,
) -> Result<End, Stop> {
    let skip = session.route_skip();
    let mut scenario = Scenario::begin(session, entry, skip)?;
    let end = loop {
        let choice = match scenario.next(session)? {
            Event::Wait(_) => {
                scenario.advance(session)?;
                continue;
            }
            Event::End(page) => {
                route.end_page = Some(page);
                break End::Start;
            }
            Event::Choice(choice) => choice,
        };
        let o = &choice.observation;
        let script = o.script.escape_ascii().to_string();
        let key = variables_digest(session.startup());
        let depth = route.choices.len();
        tree.seen
            .choice(&script, o.pc, &choice.enabled, &mut tree.found);
        // Replay the prefix while it matches where choices appear. A choice at another
        // location, or a recorded button that is no longer enabled, ends the replay and the
        // route continues as new exploration.
        let replay = if route.diverged.is_none() {
            prefix.get(depth).and_then(|want| {
                let matches = want.script == script && want.pc == o.pc;
                (matches && choice.enabled.contains(&want.button)).then_some(want.button)
            })
        } else {
            None
        };
        if replay.is_none() && route.diverged.is_none() && depth < prefix.len() {
            route.diverged = Some(depth);
            session.note("replay-diverged", Json::obj([("depth", Json::from(depth))]));
        }
        let picked = match replay {
            Some(button) => button,
            None => {
                if let Some(&first) = tree.known.get(&key) {
                    break End::Merged(first);
                }
                tree.known.insert(key.clone(), index);
                for &alternative in choice.enabled.iter().skip(1) {
                    let mut branch = route.picks.clone();
                    branch.push(Pick {
                        script: script.clone(),
                        pc: o.pc,
                        button: alternative,
                    });
                    route.branches.push(branch);
                }
                choice.enabled[0]
            }
        };
        session.note(
            "choice",
            Json::obj([
                ("depth", Json::from(depth)),
                ("group", choice.shape.slot.into()),
                ("buttons", choice.buttons.into()),
                ("picked", picked.into()),
            ]),
        );
        route.picks.push(Pick {
            script: script.clone(),
            pc: o.pc,
            button: picked,
        });
        route.choices.push(ChoicePoint {
            script,
            pc: o.pc,
            key,
            slot: choice.shape.slot,
            buttons: choice.buttons,
            enabled: choice.enabled.clone(),
            picked,
        });
        scenario.answer(session, &choice, picked)?;
    };
    route.advances = scenario.advances;
    route.nudges = scenario.nudges;
    Ok(end)
}

/// One explored choice tree.
struct Tree {
    routes: Vec<Route>,
    unexplored: Vec<Vec<Pick>>,
    choice_points: usize,
    found: Discoveries,
    /// The system save after the last route, when routes carry it forward.
    final_seed: Option<Vec<u8>>,
    events: Vec<Json>,
}

/// Explores the tree depth-first from `seed`. With `carry`, each route starts from the
/// system save the previous route left, as one player playing the routes in order.
fn explore_tree(
    runner: &Runner<'_>,
    entry: (usize, usize),
    seed: Option<Vec<u8>>,
    carry: bool,
    seen: &mut Seen,
    coverage: &mut Coverage,
) -> Result<Tree, String> {
    let mut state = TreeState {
        known: BTreeMap::new(),
        seen,
        found: Discoveries::default(),
    };
    let mut current = seed;
    let mut frontier: Vec<Vec<Pick>> = vec![Vec::new()];
    let (mut routes, mut events) = (Vec::<Route>::new(), Vec::new());
    while let Some(prefix) = frontier.pop() {
        if routes.len() >= runner.options.max_routes {
            frontier.push(prefix);
            break;
        }
        let mut session = runner.session("route", current.as_deref())?;
        session.record_coverage();
        let mut route = Route {
            entry,
            picks: Vec::new(),
            choices: Vec::new(),
            end: End::Start,
            diverged: None,
            steps: 0,
            yields: 0,
            advances: 0,
            nudges: 0,
            branches: Vec::new(),
            events: Vec::new(),
            end_page: None,
        };
        route.end = explore(
            &mut session,
            entry,
            &prefix,
            &mut state,
            routes.len(),
            &mut route,
        );
        route.steps = session.startup().steps();
        route.yields = session.yields();
        if let Some(c) = session.take_coverage() {
            coverage.merge(c);
        }
        route.events = session.take_events();
        let user = session.user_dir().to_path_buf();
        drop(session);
        if carry && let Ok(saved) = fs::read(user.join(SYSTEM_SAVE)) {
            current = Some(saved);
        }
        if let Some(page) = &route.end_page
            && state.seen.end_pages.insert(page.digest())
        {
            state.found.end_pages += 1;
        }
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
    Ok(Tree {
        routes,
        unexplored: frontier,
        choice_points: state.known.len(),
        found: state.found,
        final_seed: if carry { current } else { None },
        events,
    })
}

/// Explores one tree per scenario entry and joins them. With `carry`, the system save also
/// flows from one entry's routes into the next.
fn explore_entries(
    runner: &Runner<'_>,
    entries: &[(usize, usize)],
    seed: Option<Vec<u8>>,
    carry: bool,
    seen: &mut Seen,
    coverage: &mut Coverage,
) -> Result<Tree, String> {
    let mut joined = Tree {
        routes: Vec::new(),
        unexplored: Vec::new(),
        choice_points: 0,
        found: Discoveries::default(),
        final_seed: seed.clone(),
        events: Vec::new(),
    };
    for &entry in entries {
        let start_seed = if carry {
            joined.final_seed.clone()
        } else {
            seed.clone()
        };
        let tree = explore_tree(runner, entry, start_seed, carry, seen, coverage)?;
        let offset = joined.routes.len();
        for mut event in tree.events {
            if let Json::Obj(fields) = &mut event
                && let Some((_, Json::Int(index))) = fields
                    .iter_mut()
                    .find(|(k, _)| k == "route" || k == "index")
            {
                *index += offset as i64;
            }
            event.set("entry", entry_json(entry));
            joined.events.push(event);
        }
        joined.routes.extend(tree.routes);
        joined.unexplored.extend(tree.unexplored);
        joined.choice_points += tree.choice_points;
        joined.found.points += tree.found.points;
        joined.found.buttons += tree.found.buttons;
        joined.found.end_pages += tree.found.end_pages;
        if carry {
            joined.final_seed = tree.final_seed;
        }
    }
    if !carry {
        joined.final_seed = None;
    }
    Ok(joined)
}

impl Tree {
    /// The worst route result: a gate stop, then any other stop.
    fn stop(&self) -> Option<Outcome> {
        let stopped = || {
            self.routes.iter().filter_map(|r| match &r.end {
                End::Stopped(stop) => Some(stop),
                _ => None,
            })
        };
        stopped()
            .find_map(|s| match s {
                Stop::Blocked(b) => Some(Outcome::Blocked(b.clone())),
                _ => None,
            })
            .or_else(|| stopped().next().map(|s| Outcome::Fail(s.describe())))
    }
    fn details(&self, into: &mut Json) {
        into.set("routes_run", self.routes.len().into());
        into.set("choice_points", self.choice_points.into());
        into.set(
            "returned_to_start",
            self.routes
                .iter()
                .filter(|r| r.end == End::Start)
                .count()
                .into(),
        );
        into.set(
            "unexplored",
            Json::arr(self.unexplored.iter().map(|p| Json::from(path_text(p)))),
        );
        into.set("routes", Json::arr(self.routes.iter().map(Route::json)));
    }
}

pub struct RouteRun {
    pub result: CheckResult,
    pub events: Vec<Json>,
}

const TREE_TITLE: &str = "choice tree explored by replay to the start page";
const ROUNDS_TITLE: &str = "unlock rounds carrying the system save converge";

fn survey_failure(id: &'static str, title: &'static str, survey: &Survey) -> CheckResult {
    let outcome = match &survey.blocked {
        // A gate stop behind a start-page button hides whether it enters the scenario.
        Some(blocked) => Outcome::Blocked(blocked.clone()),
        None => Outcome::Fail("no start-page button leads to a message wait".into()),
    };
    CheckResult::new(id, title, outcome).with(survey.details.clone())
}

fn stop_result(id: &'static str, title: &'static str, stop: Stop) -> CheckResult {
    let outcome = match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    };
    CheckResult::new(id, title, outcome)
}

/// Surveys the start page, enters the scenario and explores the choice tree depth-first,
/// every route from the game's own system save.
pub fn explore_routes(runner: &Runner<'_>, coverage: &mut Coverage) -> RouteRun {
    let survey = match survey(runner, None) {
        Ok(survey) => survey,
        Err(stop) => {
            return RouteRun {
                result: stop_result("R2", TREE_TITLE, stop),
                events: Vec::new(),
            };
        }
    };
    if survey.entries.is_empty() {
        return RouteRun {
            result: survey_failure("R2", TREE_TITLE, &survey),
            events: Vec::new(),
        };
    }
    let mut seen = Seen::default();
    let tree = match explore_entries(runner, &survey.entries, None, false, &mut seen, coverage) {
        Ok(tree) => tree,
        Err(e) => {
            return RouteRun {
                result: CheckResult::new("R2", TREE_TITLE, Outcome::Fail(e)),
                events: Vec::new(),
            };
        }
    };
    let outcome = tree.stop().unwrap_or_else(|| {
        if tree.unexplored.is_empty() {
            Outcome::Pass(Level::L0)
        } else {
            Outcome::Inconclusive(format!(
                "route budget reached with {} prefixes unexplored",
                tree.unexplored.len()
            ))
        }
    });
    let mut details = survey.details;
    tree.details(&mut details);
    RouteRun {
        result: CheckResult::new("R2", TREE_TITLE, outcome).with(details),
        events: tree.events,
    }
}

/// Explores the tree in rounds, each route carrying the system save forward and each
/// round starting from the save the previous round ended with. Converges when a round
/// after the first adds no choice point, enabled button, end page or start page.
pub fn unlock_rounds(runner: &Runner<'_>, coverage: &mut Coverage) -> RouteRun {
    let mut seen = Seen::default();
    let mut seed: Option<Vec<u8>> = None;
    let mut rounds = Vec::new();
    let mut events = Vec::new();
    let mut outcome = None;
    // A stopped route is reported, but the other routes may still unlock content, so the
    // rounds go on and the stop becomes the result once they end.
    let mut first_stop = None;
    for round in 1..=runner.options.max_rounds {
        let survey = match survey(runner, seed.as_deref()) {
            Ok(survey) => survey,
            Err(stop) => {
                outcome = Some(stop_result("R4", ROUNDS_TITLE, stop).outcome);
                break;
            }
        };
        let mut found = Discoveries::default();
        if seen.start_pages.insert(survey.start.digest()) {
            found.start_pages += 1;
        }
        if survey.entries.is_empty() {
            outcome = Some(survey_failure("R4", ROUNDS_TITLE, &survey).outcome);
            rounds.push(survey.details);
            break;
        }
        let mut tree = match explore_entries(
            runner,
            &survey.entries,
            seed.clone(),
            true,
            &mut seen,
            coverage,
        ) {
            Ok(tree) => tree,
            Err(e) => {
                outcome = Some(Outcome::Fail(e));
                break;
            }
        };
        found.points += tree.found.points;
        found.buttons += tree.found.buttons;
        found.end_pages += tree.found.end_pages;
        let mut details = survey.details;
        details.set("round", round.into());
        details.set("seed", seed_json(seed.as_deref()));
        details.set("discovered", found.json());
        tree.details(&mut details);
        rounds.push(details);
        for mut event in std::mem::take(&mut tree.events) {
            event.set("round", round.into());
            events.push(event);
        }
        if first_stop.is_none() {
            first_stop = tree.stop();
        }
        if !tree.unexplored.is_empty() {
            outcome = Some(Outcome::Inconclusive(format!(
                "round {round} reached the route budget with {} prefixes unexplored",
                tree.unexplored.len()
            )));
            break;
        }
        if round > 1 && !found.any() {
            outcome = Some(Outcome::Pass(Level::L1));
            break;
        }
        seed = tree.final_seed;
    }
    let converged = matches!(outcome, Some(Outcome::Pass(_)));
    let outcome = match (outcome, first_stop) {
        // Convergence or the route budget still yields to a route that stopped.
        (Some(Outcome::Pass(_)) | Some(Outcome::Inconclusive(_)) | None, Some(stop)) => stop,
        (Some(outcome), _) => outcome,
        (None, None) => Outcome::Inconclusive(format!(
            "still discovering after {} rounds",
            runner.options.max_rounds
        )),
    };
    let details = Json::obj([
        ("converged", Json::from(converged)),
        ("rounds", Json::Arr(rounds)),
        (
            "choice_points_seen",
            Json::arr(seen.points.iter().map(|((script, pc), buttons)| {
                Json::obj([
                    ("script", Json::from(script.as_str())),
                    ("pc", Json::hex(*pc as u64)),
                    ("enabled", Json::arr(buttons.iter().map(|&b| b.into()))),
                ])
            })),
        ),
    ]);
    RouteRun {
        result: CheckResult::new("R4", ROUNDS_TITLE, outcome).with(details),
        events,
    }
}

const COVERAGE_TITLE: &str = "executed addresses against statically decoded instructions";

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
    CheckResult::new("R3", COVERAGE_TITLE, outcome).with(Json::obj([
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_counts_new_points_and_newly_enabled_buttons_only() {
        let mut seen = Seen::default();
        let mut first = Discoveries::default();
        seen.choice("a", 0x10, &[0], &mut first);
        seen.choice("b", 0x10, &[0, 1], &mut first);
        assert_eq!((first.points, first.buttons), (2, 3));
        // Revisiting finds nothing; an unlocked second button is one discovery.
        let mut again = Discoveries::default();
        seen.choice("a", 0x10, &[0], &mut again);
        seen.choice("b", 0x10, &[1, 0], &mut again);
        assert!(!again.any());
        seen.choice("a", 0x10, &[0, 1], &mut again);
        assert_eq!((again.points, again.buttons), (0, 1));
        assert!(again.any());
    }

    #[test]
    fn path_text_lists_buttons_in_order() {
        let pick = |button| Pick {
            script: "s".into(),
            pc: 0,
            button,
        };
        assert_eq!(path_text(&[pick(1), pick(0), pick(2)]), "1.0.2");
        assert_eq!(path_text(&[]), "");
    }
}

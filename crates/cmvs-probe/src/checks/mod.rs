//! All checks. Their report IDs stay the same for every game, so results can be compared
//! across games.
pub mod equivalence;
pub mod input;
pub mod pages;
pub mod robustness;
pub mod route;
pub mod saves;
pub mod startup;
pub mod static_checks;

use crate::{
    json::Json,
    report::{CheckResult, Outcome},
    session::{Coverage, Options},
    target::Target,
};
use std::{fs, path::Path};

/// Every implemented check in report order, with its one-line description.
pub const CATALOG: [(&str, &str); 32] = [
    ("S1", "engine profile, file hashes and capabilities"),
    (
        "S2",
        "all scripts decode with valid labels and jump targets",
    ),
    ("S3", "system commands in scripts classified by gate status"),
    ("S4", "resource archives verify and resource headers decode"),
    ("S5", "saves shipped with the game decode"),
    ("R1", "fresh start reaches a stable interactive page"),
    (
        "D1",
        "identical input reproduces the frame trace and final state",
    ),
    (
        "D2",
        "frame times of 16 and 33 ms pass the same message waits",
    ),
    ("D3", "damaged system and gameplay saves fail cleanly"),
    (
        "I1",
        "every input action at a message wait runs without error",
    ),
    ("I2", "mouse and keyboard pass the same message waits"),
    (
        "I3",
        "cancel and skip round trips return to the same message wait",
    ),
    (
        "I4",
        "the backlog opens on wheel up and closes back to the same wait",
    ),
    ("I5", "seeded random input keeps the engine running"),
    (
        "P1",
        "every reachable page is traversed from the start page and the scenario",
    ),
    (
        "P2",
        "leaving a page and coming back restores what was shown",
    ),
    ("P3", "quit and return-to-title controls end cleanly"),
    (
        "P4",
        "pages offer no fewer enabled buttons after a cleared route",
    ),
    (
        "C1",
        "controls that change player-facing settings are found",
    ),
    ("C2", "a changed setting survives a restart"),
    ("C3", "a changed text speed changes how long text takes"),
    ("C4", "a control restores the settings a page started with"),
    (
        "V1",
        "a save loaded after a restart resumes the same message waits",
    ),
    (
        "V2",
        "save, load and save again gives the same snapshot bytes",
    ),
    (
        "V3",
        "reloading in the same session resumes the same message waits",
    ),
    (
        "V4",
        "overwriting and loading keep the engine's backup slots",
    ),
    (
        "V5",
        "the system save carries the start page across a restart",
    ),
    ("R2", "choice tree explored by replay to the start page"),
    ("R4", "unlock rounds carrying the system save converge"),
    (
        "R3",
        "executed addresses against statically decoded instructions",
    ),
    (
        "R5",
        "a second pass with the first pass's system save is no slower",
    ),
    ("L1", "live resource counts stay bounded over a whole route"),
];
/// Checks that run when `--checks` is not given; everything else plays the scenario and
/// is opt-in.
pub const DEFAULT: [&str; 7] = ["S1", "S2", "S3", "S4", "S5", "R1", "D1"];

pub fn known(id: &str) -> bool {
    CATALOG.iter().any(|(c, _)| *c == id)
}

fn title(id: &str) -> &'static str {
    CATALOG
        .iter()
        .find(|(c, _)| *c == id)
        .map_or("unknown check", |(_, t)| t)
}

fn skipped(id: &'static str, why: String) -> CheckResult {
    CheckResult::new(id, title(id), Outcome::Skipped(why))
}

pub struct Run {
    pub results: Vec<CheckResult>,
    pub events: Vec<Json>,
}

/// Runs the selected checks. `out` is the existing session output directory; each VM
/// session gets a new user directory below it.
pub fn run(
    target: &Target,
    selected: &[&str],
    options: &Options,
    out: &Path,
    font: Option<&cmvs_desktop::font::FontRenderer>,
    manifest: Option<&crate::manifest::Manifest>,
) -> Run {
    let wants = |id: &str| selected.contains(&id);
    let mut results = Vec::new();
    let mut events = Vec::new();
    if wants("S1") {
        results.push(static_checks::identify(target));
    }
    if wants("S4") {
        results.push(static_checks::resources(target));
    }
    if wants("S5") {
        results.push(static_checks::bundled_saves(target));
    }
    let inventory = if wants("S2") || wants("S3") || wants("R3") {
        let (s2, inventory) = static_checks::scripts(target);
        if wants("S2") {
            results.push(s2);
        }
        if wants("S3") {
            results.push(static_checks::gate_inventory(target, inventory.as_ref()));
        }
        inventory
    } else {
        None
    };
    let running: Vec<&'static str> = CATALOG
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| !id.starts_with('S') && wants(id))
        .collect();
    if running.is_empty() {
        return finish(results, events);
    }
    let script = match target.missing(&startup::RUN_CAPABILITIES) {
        Some(missing) => Err(format!("missing capability {missing}")),
        None => startup::load_script(target),
    };
    let script = match script {
        Ok(script) => script,
        Err(why) => {
            for id in running {
                results.push(skipped(id, why.clone()));
            }
            return finish(results, events);
        }
    };
    if wants("R1") || wants("D1") {
        let session = |label: &str| {
            let user = out.join("users").join(label);
            fs::create_dir_all(&user)
                .map_err(|e| e.to_string())
                .and_then(|()| startup::run_to_start(target, &script, &user, options, font))
        };
        let mut first = session("start-1");
        if let Ok(run) = &mut first {
            events.append(&mut run.events);
        }
        if wants("R1") {
            results.push(startup::start_page_result(&first));
        }
        if wants("D1") {
            let second = session("start-2");
            results.push(startup::determinism_result(&first, &second));
        }
    }
    let mut runner = route::Runner::new(target, &script, options, font, out);
    runner.manifest = manifest;
    if wants("D2") {
        results.push(equivalence::frame_times(&runner));
    }
    if wants("D3") {
        results.push(robustness::damaged_saves(&runner));
    }
    if wants("I1") {
        results.push(input::actions_at_wait(&runner));
    }
    if wants("I2") {
        results.push(equivalence::devices(&runner));
    }
    if wants("I3") {
        results.push(input::round_trips(&runner));
    }
    if wants("I4") {
        results.push(input::backlog(&runner));
    }
    if wants("I5") {
        results.push(input::random_input(&runner));
    }
    let saves: Vec<&str> = ["V1", "V2", "V3", "V4"]
        .into_iter()
        .filter(|id| wants(id))
        .collect();
    if !saves.is_empty() {
        results.extend(saves::gameplay_saves(&runner, &saves));
    }
    let page_checks: Vec<&str> = ["P1", "P2", "P3", "C1", "C2", "C3", "C4"]
        .into_iter()
        .filter(|id| wants(id))
        .collect();
    if !page_checks.is_empty() {
        results.extend(pages::pages_and_settings(&runner, &page_checks));
    }
    if wants("P4") {
        results.push(pages::unlocks(&runner));
    }
    if wants("V5") {
        results.push(saves::system_save(&runner));
    }
    let mut coverage = Coverage::default();
    // Coverage alone still needs routes; the fresh tree is the cheaper source.
    if wants("R2") || (wants("R3") && !wants("R4")) {
        let mut routes = route::explore_routes(&runner, &mut coverage);
        events.append(&mut routes.events);
        if wants("R2") {
            results.push(routes.result);
        }
    }
    if wants("R4") {
        let mut rounds = route::unlock_rounds(&runner, &mut coverage);
        events.append(&mut rounds.events);
        results.push(rounds.result);
    }
    if wants("R5") {
        results.push(robustness::read_skip(&runner));
    }
    if wants("L1") {
        results.push(robustness::resources(&runner));
    }
    if wants("R3") {
        results.push(match &inventory {
            Some(inventory) => route::coverage_result(inventory, &coverage, out),
            None => skipped("R3", "script inventory unavailable".into()),
        });
    }
    finish(results, events)
}

/// Orders results as the catalog lists them.
fn finish(mut results: Vec<CheckResult>, events: Vec<Json>) -> Run {
    let order = |id: &str| {
        CATALOG
            .iter()
            .position(|(c, _)| *c == id)
            .unwrap_or(usize::MAX)
    };
    results.sort_by_key(|r| order(r.id));
    Run { results, events }
}

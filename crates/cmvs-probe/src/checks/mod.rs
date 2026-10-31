//! All checks. Their report IDs stay the same for every game, so results can be compared
//! across games.
pub mod route;
pub mod startup;
pub mod static_checks;

use crate::{
    device::Device,
    json::Json,
    report::{CheckResult, Outcome},
    session::Options,
    target::Target,
};
use std::{fs, path::Path};

/// Checks implemented so far, in run order.
pub const IMPLEMENTED: [&str; 7] = ["S1", "S2", "S3", "R1", "D1", "R2", "R3"];
/// Checks that run when `--checks` is not given; route exploration is long and opt-in.
pub const DEFAULT: [&str; 5] = ["S1", "S2", "S3", "R1", "D1"];

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
) -> Run {
    let wants = |id: &str| selected.contains(&id);
    let mut results = Vec::new();
    let mut events = Vec::new();
    if wants("S1") {
        results.push(static_checks::identify(target));
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
    let running: Vec<&str> = ["R1", "D1", "R2", "R3"]
        .into_iter()
        .filter(|id| wants(id))
        .collect();
    if running.is_empty() {
        return Run { results, events };
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
            return Run { results, events };
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
    if wants("R2") || wants("R3") {
        if options.device == Device::Keyboard {
            let why = "keyboard button navigation is not implemented".to_owned();
            for id in ["R2", "R3"].into_iter().filter(|id| wants(id)) {
                results.push(skipped(id, why.clone()));
            }
            return Run { results, events };
        }
        let runner = route::Runner::new(target, &script, options, font, out);
        let mut routes = route::explore_routes(&runner);
        events.append(&mut routes.events);
        if wants("R2") {
            results.push(routes.result);
        }
        if wants("R3") {
            results.push(match &inventory {
                Some(inventory) => route::coverage_result(inventory, &routes.coverage, out),
                None => skipped("R3", "script inventory unavailable".into()),
            });
        }
    }
    Run { results, events }
}

fn skipped(id: &'static str, why: String) -> CheckResult {
    let title = match id {
        "R1" | "D1" => return startup::skipped(id, why),
        "R2" => "choice tree explored by replay to the start page",
        _ => "executed addresses against statically decoded instructions",
    };
    CheckResult::new(id, title, Outcome::Skipped(why))
}

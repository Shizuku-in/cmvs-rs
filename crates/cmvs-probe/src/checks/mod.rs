//! Check families (design section 6). IDs are stable across games.
pub mod startup;
pub mod static_checks;

use crate::{json::Json, report::CheckResult, session::Options, target::Target};
use std::{fs, path::Path};

/// Checks implemented so far, in run order.
pub const IMPLEMENTED: [&str; 5] = ["S1", "S2", "S3", "R1", "D1"];

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
    if wants("S2") || wants("S3") {
        let (s2, inventory) = static_checks::scripts(target);
        if wants("S2") {
            results.push(s2);
        }
        if wants("S3") {
            results.push(static_checks::gate_inventory(target, inventory.as_ref()));
        }
    }
    if wants("R1") || wants("D1") {
        let script = match target.missing(&startup::RUN_CAPABILITIES) {
            Some(missing) => Err(format!("missing capability {missing}")),
            None => startup::load_script(target),
        };
        match script {
            Err(why) => {
                for id in ["R1", "D1"].into_iter().filter(|id| wants(id)) {
                    results.push(startup::skipped(id, why.clone()));
                }
            }
            Ok(script) => {
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
                    results.push(startup::r1(&first));
                }
                if wants("D1") {
                    let second = session("start-2");
                    results.push(startup::d1(&first, &second));
                }
            }
        }
    }
    Run { results, events }
}

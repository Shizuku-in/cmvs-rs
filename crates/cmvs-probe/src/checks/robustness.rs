//! Whole-route behaviour and damaged inputs: read-skip on a second pass, resources that
//! must not keep growing, and corrupt or missing saves that must fail cleanly.
use super::route::{Runner, scenario_entry};
use crate::{
    json::Json,
    observe::PageSignature,
    report::{CheckResult, Level, Outcome},
    scenario::{Event, Scenario, StartLog, drive_to_start},
    session::{Session, Stop},
};
use std::{fs, panic::AssertUnwindSafe};

/// Frames between resource samples on a long route.
const SAMPLE_FRAMES: u64 = 500;
/// Step budget for damaged-input sessions; the start page needs far fewer.
const DAMAGED_STEPS: u64 = 20_000_000;
/// The slot the damaged-save cases try to load.
const DAMAGED_SLOT: u16 = 500;
/// Resource names in `Session::resource_counts` order.
const RESOURCES: [&str; 5] = [
    "scene objects",
    "SE channels",
    "button groups",
    "sequences",
    "backlog",
];

/// A damaged-input case: name, system save seed, and bytes for the gameplay slot.
type DamagedCase = (&'static str, Option<Vec<u8>>, Option<Vec<u8>>);

const L1: &str = "live resource counts stay bounded over a whole route";
const R5: &str = "a second pass with the first pass's system save is no slower";
const D3: &str = "damaged system and gameplay saves fail cleanly";

fn stop_outcome(stop: Stop) -> Outcome {
    match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    }
}

/// One route played to its end with the first enabled button at every choice.
pub struct RoutePlay {
    pub yields: u64,
    pub steps: u64,
    pub choices: Vec<(String, usize, usize)>,
    pub end_page: PageSignature,
    pub system_save: Option<Vec<u8>>,
    pub samples: Vec<[usize; 5]>,
}

pub fn play_route(
    runner: &Runner<'_>,
    entry: (usize, usize),
    seed: Option<&[u8]>,
    label: &str,
) -> Result<RoutePlay, Outcome> {
    let mut session = runner.session(label, seed).map_err(Outcome::Fail)?;
    let skip = session.route_skip();
    let mut scenario = Scenario::begin(&mut session, entry, skip).map_err(stop_outcome)?;
    let mut choices = Vec::new();
    let mut samples = Vec::new();
    let mut next_sample = 0;
    loop {
        if session.yields() >= next_sample {
            samples.push(session.resource_counts());
            next_sample = session.yields() + SAMPLE_FRAMES;
        }
        match scenario
            .next_within(&mut session, SAMPLE_FRAMES)
            .map_err(stop_outcome)?
        {
            None => {}
            Some(Event::Wait(_)) => scenario.advance(&mut session).map_err(stop_outcome)?,
            Some(Event::Choice(choice)) => {
                let first = choice.enabled[0];
                choices.push((
                    choice.observation.script.escape_ascii().to_string(),
                    choice.observation.pc,
                    first,
                ));
                scenario
                    .answer(&mut session, &choice, first)
                    .map_err(stop_outcome)?;
            }
            Some(Event::End(end_page)) => {
                samples.push(session.resource_counts());
                return Ok(RoutePlay {
                    yields: session.yields(),
                    steps: session.startup().steps(),
                    choices,
                    end_page,
                    system_save: fs::read(session.user_dir().join("system.dat")).ok(),
                    samples,
                });
            }
        }
    }
}

/// Resources still rising at the end: the last third's maximum exceeds one and a half
/// times the middle third's plus a small margin. Counts that fill up to a cap early, such
/// as the backlog, are not flagged.
fn growth(samples: &[[usize; 5]]) -> Vec<&'static str> {
    if samples.len() < 6 {
        return Vec::new();
    }
    let third = samples.len() / 3;
    let max = |part: &[[usize; 5]], i: usize| part.iter().map(|s| s[i]).max().unwrap_or(0);
    (0..RESOURCES.len())
        .filter(|&i| {
            let middle = max(&samples[third..samples.len() - third], i);
            let late = max(&samples[samples.len() - third..], i);
            late * 2 > middle * 3 + 16
        })
        .map(|i| RESOURCES[i])
        .collect()
}

pub fn resources(runner: &Runner<'_>) -> CheckResult {
    let play =
        scenario_entry(runner, None).and_then(|entry| play_route(runner, entry, None, "resources"));
    let play = match play {
        Ok(play) => play,
        Err(outcome) => return CheckResult::new("L1", L1, outcome),
    };
    let grown = growth(&play.samples);
    let outcome = if play.samples.len() < 6 {
        Outcome::Inconclusive("the route was too short to compare early and late counts".into())
    } else if grown.is_empty() {
        Outcome::Pass(Level::L0)
    } else {
        Outcome::Fail(format!(
            "still growing at the end of the route: {}",
            grown.join(", ")
        ))
    };
    let peaks: Vec<(String, Json)> = RESOURCES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let peak = play.samples.iter().map(|s| s[i]).max().unwrap_or(0);
            let last = play.samples.last().map_or(0, |s| s[i]);
            (
                (*name).to_owned(),
                Json::obj([("peak", Json::from(peak)), ("last", last.into())]),
            )
        })
        .collect();
    CheckResult::new("L1", L1, outcome).with(Json::obj([
        ("samples", Json::from(play.samples.len())),
        ("frames", play.yields.into()),
        ("resources", Json::Obj(peaks)),
    ]))
}

pub fn read_skip(runner: &Runner<'_>) -> CheckResult {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("R5", R5, outcome),
    };
    let first = match play_route(runner, entry, None, "read-first") {
        Ok(play) => play,
        Err(outcome) => return CheckResult::new("R5", R5, outcome),
    };
    let Some(saved) = first.system_save.clone() else {
        return CheckResult::new(
            "R5",
            R5,
            Outcome::Fail("the first pass wrote no system save".into()),
        );
    };
    let second = match play_route(runner, entry, Some(&saved), "read-second") {
        Ok(play) => play,
        Err(outcome) => return CheckResult::new("R5", R5, outcome),
    };
    let outcome = if first.choices != second.choices {
        Outcome::Fail("the second pass met different choices".into())
    } else if second.yields > first.yields {
        Outcome::Fail("the second pass took more frames than the first".into())
    } else {
        Outcome::Pass(Level::L1)
    };
    CheckResult::new("R5", R5, outcome).with(Json::obj([
        ("first_frames", Json::from(first.yields)),
        ("second_frames", second.yields.into()),
        ("first_steps", first.steps.into()),
        ("second_steps", second.steps.into()),
        ("choices", first.choices.len().into()),
    ]))
}

/// Deterministic filler bytes for damaged files.
fn garbage(length: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// Reaches the start page; a clean error is acceptable, a hang (budget) is not.
fn start_or_error(session: &mut Session<'_>) -> Result<String, String> {
    match drive_to_start(session, &mut StartLog::default()) {
        Ok(_) => Ok("reached the start page".into()),
        Err(Stop::Budget) => Err("no start page within the step budget".into()),
        Err(Stop::Quit) => Ok("the script quit".into()),
        Err(stop) => Ok(format!("stopped cleanly: {}", stop.describe())),
    }
}

/// Loads a slot that is missing or damaged; the load must be refused and the session must
/// keep running.
fn refused_load(session: &mut Session<'_>) -> Result<String, String> {
    let reached = start_or_error(session)?;
    if reached != "reached the start page" {
        return Ok(reached);
    }
    match session.load_slot(DAMAGED_SLOT) {
        Ok(()) => Err("the engine accepted the slot".into()),
        Err(e) => {
            for _ in 0..60 {
                if let Err(stop) = session.frame() {
                    return Err(format!(
                        "the session stopped after the refused load: {}",
                        stop.describe()
                    ));
                }
            }
            Ok(format!("refused: {e}"))
        }
    }
}

pub fn damaged_saves(runner: &Runner<'_>) -> CheckResult {
    let mut options = runner.options.clone();
    options.max_steps = DAMAGED_STEPS;
    let damaged = runner.with_options(&options);
    let template = fs::read(runner.target.root.join("save/system.dat")).ok();
    let mut cases: Vec<DamagedCase> = vec![
        ("corrupt system save", Some(garbage(4096)), None),
        ("missing gameplay slot", None, None),
        ("corrupt gameplay slot", None, Some(garbage(8192))),
    ];
    if let Some(template) = &template {
        cases.insert(
            1,
            (
                "truncated system save",
                Some(template[..template.len().min(64)].to_vec()),
                None,
            ),
        );
    }
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (name, seed, slot) in cases {
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<String, String> {
            let mut session = match damaged.session("damaged", seed.as_deref()) {
                Ok(session) => session,
                Err(e) => return Ok(format!("refused at startup: {e}")),
            };
            if let Some(bytes) = &slot {
                let path = session
                    .user_dir()
                    .join(format!("save{DAMAGED_SLOT:03}.dat"));
                fs::write(path, bytes).map_err(|e| e.to_string())?;
            }
            if name.contains("slot") {
                refused_load(&mut session)
            } else {
                start_or_error(&mut session)
            }
        }));
        let text = match outcome {
            Ok(Ok(text)) => text,
            Ok(Err(why)) => {
                failures.push(format!("{name}: {why}"));
                why
            }
            Err(_) => {
                failures.push(format!("{name}: panicked"));
                "panicked".into()
            }
        };
        rows.push(Json::obj([
            ("case", Json::from(name)),
            ("result", text.into()),
        ]));
    }
    let outcome = if failures.is_empty() {
        Outcome::Pass(Level::L0)
    } else {
        Outcome::Fail(failures.join("; "))
    };
    CheckResult::new("D3", D3, outcome).with(Json::obj([("cases", Json::Arr(rows))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_flags_only_counts_that_keep_rising() {
        let mut samples = vec![[3, 1, 2, 0, 10]; 3];
        samples.extend(vec![[4, 1, 2, 0, 128]; 3]);
        samples.extend(vec![[40, 1, 2, 0, 128]; 3]);
        assert_eq!(growth(&samples), ["scene objects"]);
        assert!(growth(&samples[..5]).is_empty());
    }
}

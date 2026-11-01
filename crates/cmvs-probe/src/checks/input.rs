//! Input at message waits: every abstract action, round trips that must restore what the
//! player sees (cancel, skip, backlog), and seeded random input.
use super::route::{Runner, scenario_entry};
use crate::{
    device::{Action, Device, skip_code},
    json::Json,
    observe::{Observation, Settle},
    report::{CheckResult, Level, Outcome},
    scenario::{Event, Scenario, WaitPoint, follow, view_digest},
    session::{Session, Stop},
};

/// The message wait each input check starts from.
const START_WAIT: usize = 10;
/// Frames an action gets to show an effect.
const EFFECT_FRAMES: u64 = 600;
/// Frames waited after an action before looking at the view.
const SETTLE_FRAMES: u64 = 90;
/// Frames skip is held in the skip round trip.
const SKIP_FRAMES: usize = 60;
/// Frames of random input, and frames between random actions.
const MONKEY_FRAMES: u64 = 3000;
const MONKEY_STEP: u64 = 15;

const I1: &str = "every input action at a message wait runs without error";
const I3: &str = "cancel and skip round trips return to the same message wait";
const I4: &str = "the backlog opens on wheel up and closes back to the same wait";
const I5: &str = "seeded random input keeps the engine running";

fn stop_outcome(stop: Stop) -> Outcome {
    match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    }
}

/// A fresh session waiting at message wait `START_WAIT`, not yet advanced.
struct AtWait<'t> {
    session: Session<'t>,
    scenario: Scenario,
    point: WaitPoint,
    view: String,
}

fn at_wait<'t>(
    runner: &Runner<'t>,
    entry: (usize, usize),
    label: &str,
) -> Result<AtWait<'t>, Outcome> {
    let mut session = runner.session(label, None).map_err(Outcome::Fail)?;
    let mut scenario = Scenario::begin(&mut session, entry, false).map_err(stop_outcome)?;
    let points = follow(&mut session, &mut scenario, START_WAIT).map_err(stop_outcome)?;
    let Some(point) = points.last().cloned() else {
        return Err(Outcome::Inconclusive(
            "the scenario shows no message wait".into(),
        ));
    };
    let view = current_view(&session);
    Ok(AtWait {
        session,
        scenario,
        point,
        view,
    })
}

fn current_view(session: &Session<'_>) -> String {
    view_digest(
        session,
        &Observation::of(session.startup(), session.yields()),
    )
}

/// Runs plain frames; the scenario driver is not consulted.
fn frames(session: &mut Session<'_>, count: u64) -> Result<(), Stop> {
    for _ in 0..count {
        session.frame()?;
    }
    Ok(())
}

/// What the scenario did after an action.
fn effect(at: &mut AtWait<'_>) -> Result<String, Stop> {
    // An input can open a page with several control groups, such as the backlog.
    // Observe it directly: scenario choice detection would reject that page as
    // ambiguous and can inject unrelated confirmation input while waiting.
    frames(&mut at.session, SETTLE_FRAMES)?;
    let mut settle = Settle::new(crate::scenario::STABLE_FRAMES);
    for _ in 0..EFFECT_FRAMES {
        let o = at.session.frame()?;
        if at.session.busy() {
            continue;
        }
        if o.in_message_wait() {
            return Ok(if WaitPoint::of(&at.session, &o) == at.point {
                "same wait".into()
            } else {
                "advanced".into()
            });
        }
        if let Some(page) = settle.feed(&o) {
            return Ok(format!("page with {} groups", page.groups.len()));
        }
    }
    Ok(if current_view(&at.session) == at.view {
        "no visible change".into()
    } else {
        "view changed".into()
    })
}

fn actions(device: Device) -> Vec<(&'static str, Option<Action>)> {
    let mut list = vec![
        ("advance", Some(Action::Advance)),
        ("cancel", Some(Action::Cancel)),
    ];
    if device == Device::Mouse {
        list.push(("wheel up", Some(Action::WheelUp)));
        list.push(("wheel down", Some(Action::WheelDown)));
    }
    // Skip is held through key.cfg whatever the device.
    list.push(("skip", None));
    list
}

/// Holds skip for `SKIP_FRAMES` frames, then releases it.
fn hold_skip(session: &mut Session<'_>) -> Result<bool, Stop> {
    let Some(code) = skip_code(session.bindings()) else {
        return Ok(false);
    };
    session.hold(vec![code]);
    frames(session, SKIP_FRAMES as u64)?;
    session.hold(Vec::new());
    Ok(true)
}

pub fn actions_at_wait(runner: &Runner<'_>) -> CheckResult {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("I1", I1, outcome),
    };
    let mut rows = Vec::new();
    let mut worst = None;
    for (name, action) in actions(runner.options.device) {
        let mut at = match at_wait(runner, entry, "input-action") {
            Ok(at) => at,
            Err(outcome) => return CheckResult::new("I1", I1, outcome),
        };
        let result = (|| -> Result<String, Stop> {
            match action {
                Some(action) => at.session.act(action).map_err(Stop::Error)?,
                None => {
                    if !hold_skip(&mut at.session)? {
                        return Ok("no skip key bound".into());
                    }
                }
            }
            effect(&mut at)
        })();
        let text = match result {
            Ok(text) => text,
            Err(Stop::Quit) => "quit".into(),
            Err(stop) => {
                let description = stop.describe();
                worst.get_or_insert(stop_outcome(stop));
                format!("stopped: {description}")
            }
        };
        rows.push(Json::obj([
            ("action", Json::from(name)),
            ("effect", text.into()),
        ]));
    }
    CheckResult::new("I1", I1, worst.unwrap_or(Outcome::Pass(Level::L0)))
        .with(Json::obj([("actions", Json::Arr(rows))]))
}

/// Cancel opens something and a second cancel closes it again; skip held and released
/// leaves the scenario waiting for input again.
pub fn round_trips(runner: &Runner<'_>) -> CheckResult {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("I3", I3, outcome),
    };
    let cancel = (|| -> Result<(String, bool), Outcome> {
        let mut at = at_wait(runner, entry, "input-cancel")?;
        at.session.act(Action::Cancel).map_err(Outcome::Fail)?;
        frames(&mut at.session, SETTLE_FRAMES).map_err(stop_outcome)?;
        if current_view(&at.session) == at.view {
            return Ok(("cancel changed nothing visible".into(), true));
        }
        at.session.act(Action::Cancel).map_err(Outcome::Fail)?;
        let back = at
            .scenario
            .next_within(&mut at.session, EFFECT_FRAMES)
            .map_err(stop_outcome)?;
        let restored = matches!(&back, Some(Event::Wait(o))
            if WaitPoint::of(&at.session, o) == at.point && current_view(&at.session) == at.view);
        Ok((
            if restored {
                "second cancel restored the wait".into()
            } else {
                "second cancel did not restore the wait".into()
            },
            restored,
        ))
    })();
    let skip = (|| -> Result<(String, bool), Outcome> {
        let mut at = at_wait(runner, entry, "input-skip")?;
        if !hold_skip(&mut at.session).map_err(stop_outcome)? {
            return Ok(("no skip key bound".into(), true));
        }
        let next = at
            .scenario
            .next_within(&mut at.session, EFFECT_FRAMES * 2)
            .map_err(stop_outcome)?;
        Ok(match next {
            Some(Event::Wait(_) | Event::Choice(_) | Event::End(_)) => {
                ("released skip waits for input again".into(), true)
            }
            None => ("no message wait after releasing skip".into(), false),
        })
    })();
    match (cancel, skip) {
        (Ok((c, c_ok)), Ok((s, s_ok))) => CheckResult::new(
            "I3",
            I3,
            if c_ok && s_ok {
                Outcome::Pass(Level::L1)
            } else {
                Outcome::Fail(if c_ok { s.clone() } else { c.clone() })
            },
        )
        .with(Json::obj([("cancel", Json::from(c)), ("skip", s.into())])),
        (Err(outcome), _) | (_, Err(outcome)) => CheckResult::new("I3", I3, outcome),
    }
}

/// Wheel up opens the backlog; cancel, then wheel down, close it again.
pub fn backlog(runner: &Runner<'_>) -> CheckResult {
    if runner.options.device != Device::Mouse {
        return CheckResult::new(
            "I4",
            I4,
            Outcome::Skipped("the keyboard device has no wheel".into()),
        );
    }
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("I4", I4, outcome),
    };
    let result = (|| -> Result<CheckResult, Outcome> {
        let mut at = at_wait(runner, entry, "input-backlog")?;
        let entries = at.session.startup().history().entries().len();
        if entries == 0 {
            return Ok(CheckResult::new(
                "I4",
                I4,
                Outcome::Fail("no backlog entry after ten message waits".into()),
            ));
        }
        at.session.act(Action::WheelUp).map_err(Outcome::Fail)?;
        frames(&mut at.session, SETTLE_FRAMES).map_err(stop_outcome)?;
        if current_view(&at.session) == at.view {
            return Ok(CheckResult::new(
                "I4",
                I4,
                Outcome::Inconclusive("wheel up showed nothing new".into()),
            )
            .with(Json::obj([("entries", Json::from(entries))])));
        }
        let mut closed_by = None;
        // Cancel first: wheel down may also be bound to advancing the text.
        for (name, action) in std::iter::once(("cancel", Action::Cancel))
            .chain(std::iter::repeat_n(("wheel down", Action::WheelDown), 10))
        {
            at.session.act(action).map_err(Outcome::Fail)?;
            frames(&mut at.session, 30).map_err(stop_outcome)?;
            if current_view(&at.session) == at.view {
                closed_by = Some(name);
                break;
            }
        }
        // Right after closing, the VM should still stand in the same message wait.
        let now = Observation::of(at.session.startup(), at.session.yields());
        let still = now
            .in_message_wait()
            .then(|| WaitPoint::of(&at.session, &now));
        let back = match (closed_by, &still) {
            (Some(_), None) => at
                .scenario
                .next_within(&mut at.session, EFFECT_FRAMES)
                .map_err(stop_outcome)?
                .and_then(|event| match event {
                    Event::Wait(o) => Some(WaitPoint::of(&at.session, &o)),
                    _ => None,
                }),
            _ => still.clone(),
        };
        let restored = back.as_ref() == Some(&at.point);
        Ok(CheckResult::new(
            "I4",
            I4,
            if restored {
                Outcome::Pass(Level::L1)
            } else {
                Outcome::Fail("the backlog did not close back to the same message wait".into())
            },
        )
        .with(Json::obj([
            ("entries", Json::from(entries)),
            ("closed_by", closed_by.map(str::to_owned).into()),
            ("before", at.point.json()),
            ("after", back.as_ref().map_or(Json::Null, WaitPoint::json)),
        ])))
    })();
    result.unwrap_or_else(|outcome| CheckResult::new("I4", I4, outcome))
}

/// Seeded random actions in the scenario; only errors and gate stops fail.
pub fn random_input(runner: &Runner<'_>) -> CheckResult {
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return CheckResult::new("I5", I5, outcome),
    };
    let mut at = match at_wait(runner, entry, "input-random") {
        Ok(at) => at,
        Err(outcome) => return CheckResult::new("I5", I5, outcome),
    };
    let mut state = u64::from(runner.options.seed) ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        // xorshift64: a fixed sequence for a fixed seed.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut counts = std::collections::BTreeMap::<&str, u64>::new();
    let mut ended = "frame budget".to_owned();
    let mut outcome = Outcome::Pass(Level::L0);
    let from = at.session.yields();
    while at.session.yields() - from < MONKEY_FRAMES {
        if !at.session.busy() && (at.session.yields() - from).is_multiple_of(MONKEY_STEP) {
            let buttons = crate::controls::enabled(at.session.startup());
            let pick = next() % 5;
            let (name, action) = match pick {
                0 if !buttons.is_empty() => {
                    let (group, button) = buttons[(next() % buttons.len() as u64) as usize];
                    ("press", Action::Press { group, button })
                }
                1 => ("cancel", Action::Cancel),
                2 if runner.options.device == Device::Mouse => ("wheel up", Action::WheelUp),
                3 if runner.options.device == Device::Mouse => ("wheel down", Action::WheelDown),
                _ => ("advance", Action::Advance),
            };
            if at.session.act(action).is_ok() {
                *counts.entry(name).or_default() += 1;
            }
        }
        match at.session.frame() {
            Ok(_) => {}
            Err(Stop::Quit) => {
                ended = "the script quit".into();
                break;
            }
            Err(stop) => {
                ended = stop.describe();
                outcome = stop_outcome(stop);
                break;
            }
        }
    }
    CheckResult::new("I5", I5, outcome).with(Json::obj([
        ("ended", Json::from(ended)),
        ("frames", (at.session.yields() - from).into()),
        (
            "actions",
            Json::Obj(
                counts
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), Json::from(v)))
                    .collect(),
            ),
        ),
    ]))
}

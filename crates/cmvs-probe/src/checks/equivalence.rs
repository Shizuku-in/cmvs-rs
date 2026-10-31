//! The same scenario stretch under different run conditions must pass the same message
//! waits with the same text: another virtual frame time, or the other input device.
use super::route::{Runner, scenario_entry};
use crate::{
    device::Device,
    json::Json,
    report::{CheckResult, Level, Outcome},
    scenario::{Scenario, WaitPoint, describe_divergence, first_difference, follow},
    session::{Options, Stop},
    target::cap,
};

/// Message waits compared from the start of the scenario.
const WAITS: usize = 40;

const D2: &str = "frame times of 16 and 33 ms pass the same message waits";
const I2: &str = "mouse and keyboard pass the same message waits";

fn trace(runner: &Runner<'_>, options: &Options, label: &str) -> Result<Vec<WaitPoint>, Outcome> {
    let fail = |stop: Stop| match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    };
    let variant = runner.with_options(options);
    let entry = scenario_entry(&variant, None)?;
    let mut session = variant.session(label, None).map_err(Outcome::Fail)?;
    let mut scenario = Scenario::begin(&mut session, entry, false).map_err(fail)?;
    follow(&mut session, &mut scenario, WAITS).map_err(fail)
}

fn compare(
    id: &'static str,
    title: &'static str,
    runner: &Runner<'_>,
    variants: [(&str, Options); 2],
) -> CheckResult {
    let [(la, a), (lb, b)] = variants;
    let first = match trace(runner, &a, la) {
        Ok(t) => t,
        Err(outcome) => return CheckResult::new(id, title, outcome),
    };
    let second = match trace(runner, &b, lb) {
        Ok(t) => t,
        Err(outcome) => return CheckResult::new(id, title, outcome),
    };
    let outcome = match first_difference(&first, &second) {
        None if first.len() > 1 => Outcome::Pass(Level::L1),
        None => Outcome::Inconclusive("too few message waits to compare".into()),
        Some(i) => Outcome::Fail(format!(
            "{la} vs {lb}: {}",
            describe_divergence(&first, &second, i)
        )),
    };
    CheckResult::new(id, title, outcome).with(Json::obj([
        ("waits", Json::from(first.len())),
        (la, Json::arr(first.iter().map(WaitPoint::json))),
        (lb, Json::arr(second.iter().map(WaitPoint::json))),
    ]))
}

pub fn frame_times(runner: &Runner<'_>) -> CheckResult {
    let mut fast = runner.options.clone();
    fast.frame_ms = 16;
    let mut slow = runner.options.clone();
    slow.frame_ms = 33;
    compare("D2", D2, runner, [("16ms", fast), ("33ms", slow)])
}

pub fn devices(runner: &Runner<'_>) -> CheckResult {
    if !runner.target.has(cap::KEY_CONFIG) {
        return CheckResult::new(
            "I2",
            I2,
            Outcome::Skipped(format!("missing capability {}", cap::KEY_CONFIG)),
        );
    }
    let mut mouse = runner.options.clone();
    mouse.device = Device::Mouse;
    let mut keyboard = runner.options.clone();
    keyboard.device = Device::Keyboard;
    compare("I2", I2, runner, [("mouse", mouse), ("keyboard", keyboard)])
}

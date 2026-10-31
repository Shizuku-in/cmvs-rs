//! Gameplay saves and the system save, through the engine's own save and load operations
//! (the ones the script commands perform), so no save-menu layout has to be known.
//!
//! One session plays into the scenario and saves at a message wait, then plays on and
//! records which message waits follow. Loading that save, in the same session or in a new
//! one started from the start page, must lead through the same waits with the same text.
use super::route::{Runner, scenario_entry};
use crate::{
    json::Json,
    report::{CheckResult, Level, Outcome},
    scenario::{Scenario, StartLog, WaitPoint, drive_to_start, first_difference, follow},
    session::{Session, Stop},
    target::cap,
};
use cmvs_core::{save::Csv2Save, save_menu::SaveHeader, snapshot::parse_records};
use std::{fs, path::Path};

/// Message waits played before the first save attempt.
const SAVE_AFTER: usize = 30;
/// Message waits tried for a save point once `SAVE_AFTER` is reached.
const SAVE_TRIES: usize = 300;
/// Message waits compared after the save point.
const FOLLOW: usize = 20;
/// Extra message waits a load may show before the saved line again.
const LOAD_LEAD: usize = 2;
/// Slots used by the checks; slot 2 keeps the save point while slot 1 is overwritten.
const POINT: u16 = 2;
const OVERWRITTEN: u16 = 1;
const RESAVED: u16 = 3;
/// The engine keeps the previous version of an overwritten slot here, and the state before
/// a load here.
const OVERWRITE_BACKUP: u16 = 999;
const LOAD_BACKUP: u16 = 998;

const V1: &str = "a save loaded after a restart resumes the same message waits";
const V2: &str = "save, load and save again gives the same snapshot bytes";
const V3: &str = "reloading in the same session resumes the same message waits";
const V4: &str = "overwriting and loading keep the engine's backup slots";
const V5: &str = "the system save carries the start page across a restart";

fn slot_path(user: &Path, slot: u16) -> std::path::PathBuf {
    user.join(format!("save{slot:03}.dat"))
}

fn stop_outcome(stop: Stop) -> Outcome {
    match stop {
        Stop::Blocked(b) => Outcome::Blocked(b),
        other => Outcome::Fail(other.describe()),
    }
}

fn trace_json(points: &[WaitPoint]) -> Json {
    Json::arr(points.iter().map(WaitPoint::json))
}

/// Compares a resumed trace with the original; both start at the save point.
fn compare(
    id: &'static str,
    title: &'static str,
    original: &[WaitPoint],
    resumed: &[WaitPoint],
) -> CheckResult {
    // A load may stop once more before the saved line is shown again; allow a short lead.
    let aligned = (0..=LOAD_LEAD).find_map(|lead| {
        let resumed = resumed.get(lead..)?;
        let length = original.len().min(resumed.len());
        (length > 1 && first_difference(&original[..length], &resumed[..length]).is_none())
            .then_some((lead, length))
    });
    let outcome = match aligned {
        Some(_) => Outcome::Pass(Level::L1),
        None if original.len() <= 1 => {
            Outcome::Inconclusive("too few message waits after the save point".into())
        }
        None => Outcome::Fail(format!(
            "the resumed message waits do not match the original within a lead of {LOAD_LEAD}"
        )),
    };
    CheckResult::new(id, title, outcome).with(Json::obj([
        ("lead", aligned.map(|(lead, _)| lead).into()),
        ("compared", aligned.map(|(_, length)| length).into()),
        ("original", trace_json(original)),
        ("resumed", trace_json(resumed)),
    ]))
}

/// Compares two CSV2 files section by section and names the first differing snapshot
/// record.
fn compare_saves(first: &[u8], second: &[u8]) -> Result<Option<String>, String> {
    let a = Csv2Save::decode(first).map_err(|e| e.to_string())?;
    let b = Csv2Save::decode(second).map_err(|e| e.to_string())?;
    if a.snapshot != b.snapshot {
        let ra = parse_records(&a.snapshot).map_err(|e| e.to_string())?;
        let rb = parse_records(&b.snapshot).map_err(|e| e.to_string())?;
        let tag = ra
            .iter()
            .zip(&rb)
            .find(|(x, y)| {
                x.tag != y.tag || a.snapshot[x.body.clone()] != b.snapshot[y.body.clone()]
            })
            .map(|(x, _)| format!("0x{:x}", x.tag))
            .unwrap_or_else(|| format!("record count {} vs {}", ra.len(), rb.len()));
        return Ok(Some(format!("snapshot record {tag} differs")));
    }
    if a.script_image != b.script_image {
        return Ok(Some("script memory image differs".into()));
    }
    if a.auxiliary != b.auxiliary {
        return Ok(Some("auxiliary section differs".into()));
    }
    Ok(None)
}

/// Runs V1 to V4 on one save point; returns their results in order.
pub fn gameplay_saves(runner: &Runner<'_>, wanted: &[&str]) -> Vec<CheckResult> {
    let titles = [("V1", V1), ("V2", V2), ("V3", V3), ("V4", V4)];
    let all = |outcome: Outcome| -> Vec<CheckResult> {
        titles
            .iter()
            .filter(|(id, _)| wanted.contains(id))
            .map(|&(id, title)| CheckResult::new(id, title, outcome.clone()))
            .collect()
    };
    if !runner.target.has(cap::SNAPSHOT_LAYOUT) {
        return all(Outcome::Skipped(format!(
            "missing capability {}",
            cap::SNAPSHOT_LAYOUT
        )));
    }
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return all(outcome),
    };
    match run(runner, entry) {
        Ok(results) => results
            .into_iter()
            .filter(|r| wanted.contains(&r.id))
            .collect(),
        Err(outcome) => all(outcome),
    }
}

fn run(runner: &Runner<'_>, entry: (usize, usize)) -> Result<Vec<CheckResult>, Outcome> {
    // Session A plays to a save point, saves twice and records what follows.
    let mut a = runner.session("save-play", None).map_err(Outcome::Fail)?;
    let mut scenario = Scenario::begin(&mut a, entry, false).map_err(stop_outcome)?;
    let before = follow(&mut a, &mut scenario, SAVE_AFTER).map_err(stop_outcome)?;
    if before.len() < SAVE_AFTER {
        return Err(Outcome::Inconclusive(
            "the scenario ended before a save point".into(),
        ));
    }
    let mut saved = false;
    for _ in 0..SAVE_TRIES {
        if a.save_slot(POINT).map_err(Outcome::Fail)? {
            saved = true;
            break;
        }
        scenario.advance(&mut a).map_err(stop_outcome)?;
        if follow(&mut a, &mut scenario, 1)
            .map_err(stop_outcome)?
            .is_empty()
        {
            break;
        }
    }
    if !saved {
        return Err(Outcome::Inconclusive(
            "the engine built no gameplay snapshot within the tried message waits".into(),
        ));
    }
    a.save_slot(OVERWRITTEN).map_err(Outcome::Fail)?;
    let user_a = a.user_dir().to_path_buf();
    let point_bytes = fs::read(slot_path(&user_a, POINT))
        .map_err(|e| e.to_string())
        .map_err(Outcome::Fail)?;
    let original = follow(&mut a, &mut scenario, FOLLOW + 1).map_err(stop_outcome)?;

    // V4: overwrite slot 1 and load slot 2 in the same session.
    let first_version =
        fs::read(slot_path(&user_a, OVERWRITTEN)).map_err(|e| Outcome::Fail(e.to_string()))?;
    a.save_slot(OVERWRITTEN).map_err(Outcome::Fail)?;
    let backup = fs::read(slot_path(&user_a, OVERWRITE_BACKUP)).ok();
    let start = scenario.start.clone();
    a.load_slot(POINT).map_err(Outcome::Fail)?;
    let load_backup = slot_path(&user_a, LOAD_BACKUP).is_file();
    let mut issues = Vec::new();
    if backup.as_deref() != Some(first_version.as_slice()) {
        issues.push("overwriting a slot did not keep its previous version in slot 999");
    }
    if !load_backup {
        issues.push("loading a slot did not keep the state before the load in slot 998");
    }
    let headers: Vec<Json> = [POINT, OVERWRITTEN, OVERWRITE_BACKUP, LOAD_BACKUP]
        .into_iter()
        .map(|slot| {
            let parsed = fs::read(slot_path(&user_a, slot))
                .ok()
                .map(|bytes| SaveHeader::parse(&bytes));
            let state = match parsed {
                None => "missing".to_owned(),
                Some(Ok(header)) => format!("label {}", header.label.escape_ascii()),
                Some(Err(e)) => format!("unreadable: {e}"),
            };
            if !state.starts_with("label") {
                issues.push("a slot header is missing or unreadable");
            }
            Json::obj([("slot", Json::from(slot)), ("header", state.into())])
        })
        .collect();
    issues.dedup();
    let v4 = CheckResult::new(
        "V4",
        V4,
        if issues.is_empty() {
            Outcome::Pass(Level::L1)
        } else {
            Outcome::Fail(issues.join("; "))
        },
    )
    .with(Json::obj([("slots", Json::Arr(headers))]));

    // V3: the same session resumes from the reloaded point.
    let mut again = Scenario::resume(&mut a, start.clone(), false);
    let reloaded = follow(&mut a, &mut again, FOLLOW + 1).map_err(stop_outcome)?;
    let v3 = compare("V3", V3, &original, &reloaded);
    let system = fs::read(user_a.join("system.dat")).ok();
    drop(a);

    // Session B starts fresh with the save point and loads it from the start page.
    let mut b = runner
        .session("save-restart", system.as_deref())
        .map_err(Outcome::Fail)?;
    let user_b = b.user_dir().to_path_buf();
    fs::write(slot_path(&user_b, POINT), &point_bytes).map_err(|e| Outcome::Fail(e.to_string()))?;
    let start_b = drive_to_start(&mut b, &mut StartLog::default()).map_err(stop_outcome)?;
    b.load_slot(POINT).map_err(Outcome::Fail)?;
    // V2: saving right after the load must reproduce the loaded snapshot.
    let v2 = match b.save_slot(RESAVED) {
        Ok(true) => match fs::read(slot_path(&user_b, RESAVED))
            .map_err(|e| e.to_string())
            .and_then(|resaved| compare_saves(&point_bytes, &resaved))
        {
            Ok(None) => CheckResult::new("V2", V2, Outcome::Pass(Level::L1)),
            Ok(Some(difference)) => CheckResult::new("V2", V2, Outcome::Fail(difference)),
            Err(e) => CheckResult::new("V2", V2, Outcome::Fail(e)),
        },
        Ok(false) => CheckResult::new(
            "V2",
            V2,
            Outcome::Fail("no snapshot was available right after the load".into()),
        ),
        Err(e) => CheckResult::new("V2", V2, Outcome::Fail(e)),
    };
    let mut resumed_scenario = Scenario::resume(&mut b, start_b, false);
    let resumed = follow(&mut b, &mut resumed_scenario, FOLLOW + 1).map_err(stop_outcome)?;
    let v1 = compare("V1", V1, &original, &resumed);
    Ok(vec![v1, v2, v3, v4])
}

/// V5: one route from the game's template to its end, then a restart with the system save
/// it left; the restarted start page must equal the page the route ended on.
pub fn system_save(runner: &Runner<'_>) -> CheckResult {
    let result = |outcome| CheckResult::new("V5", V5, outcome);
    let entry = match scenario_entry(runner, None) {
        Ok(entry) => entry,
        Err(outcome) => return result(outcome),
    };
    let (saved, end_page) = match super::robustness::play_route(runner, entry, None, "system-route")
    {
        Ok(play) => (play.system_save, play.end_page),
        Err(outcome) => return result(outcome),
    };
    let Some(saved) = saved else {
        return result(Outcome::Fail(
            "the route ended without writing a system save".into(),
        ));
    };
    let restarted = runner
        .session("system-restart", Some(&saved))
        .map_err(Stop::Error)
        .and_then(|mut s: Session<'_>| drive_to_start(&mut s, &mut StartLog::default()));
    match restarted {
        Ok(page) => {
            let outcome = if page == end_page {
                Outcome::Pass(Level::L1)
            } else {
                Outcome::Fail("the start page after the restart differs from the route end".into())
            };
            result(outcome).with(Json::obj([
                ("route_end", end_page.json()),
                ("after_restart", page.json()),
            ]))
        }
        Err(stop) => result(stop_outcome(stop)),
    }
}

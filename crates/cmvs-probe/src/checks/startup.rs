//! Reaching the start page from a fresh start, and checking that the same input twice
//! gives the same run.
use crate::{
    device::Action,
    json::Json,
    observe::{Observation, PageSignature, Settle, state_digest},
    report::{CheckResult, Level, Outcome},
    session::{Options, Session, Stop},
    target::{Target, cap},
};
use cmvs_core::{digest::to_hex, ps3::Script, runner::load_start};
use std::path::Path;

/// Frames an interactive page must stay unchanged to count as settled.
pub const STABLE_FRAMES: u32 = 60;
pub const RUN_CAPABILITIES: [&str; 3] = [cap::PROFILE, cap::CPZ6, cap::KEY_CONFIG];

/// Result of driving one fresh session to its first stable interactive page.
pub struct StartRun {
    pub reached: Option<PageSignature>,
    pub stop: Option<Stop>,
    pub last: Option<Observation>,
    pub scripts: Vec<String>,
    pub yields: u64,
    pub steps: u64,
    pub advances: u64,
    pub trace: [u8; 32],
    pub frames: Vec<[u8; 8]>,
    pub state: String,
    pub events: Vec<Json>,
}

pub fn load_script(target: &Target) -> Result<Script, String> {
    let files = target.files.as_ref().ok_or("game files not found")?;
    load_start(&target.root, files).map_err(|e| e.to_string())
}

/// What happened on the way to the start page.
#[derive(Debug, Default)]
pub struct StartLog {
    pub scripts: Vec<String>,
    pub advances: u64,
    pub last: Option<Observation>,
}

/// Advances every message wait until the first settled page. Nothing here knows which
/// script shows the title; the first settled page is the start page by definition.
pub fn drive_to_start(
    session: &mut Session<'_>,
    log: &mut StartLog,
) -> Result<PageSignature, Stop> {
    let mut settle = Settle::new(STABLE_FRAMES);
    loop {
        let observation = session.frame()?;
        let name = observation.script.escape_ascii().to_string();
        if !log.scripts.contains(&name) {
            log.scripts.push(name);
        }
        if observation.waiting_for_advance() && !session.busy() {
            session
                .act(Action::Advance)
                .map_err(crate::session::Stop::Error)?;
            log.advances += 1;
        }
        let settled = settle.feed(&observation).cloned();
        log.last = Some(observation);
        if let Some(page) = settled {
            return Ok(page);
        }
    }
}

/// Drives a new session in a fresh user directory to the start page and records the run.
pub fn run_to_start(
    target: &Target,
    script: &Script,
    user: &Path,
    options: &Options,
    font: Option<&cmvs_desktop::font::FontRenderer>,
) -> Result<StartRun, String> {
    let mut session = Session::start(target, script, user, options, font)?;
    let mut log = StartLog::default();
    let (reached, stop) = match drive_to_start(&mut session, &mut log) {
        Ok(page) => (Some(page), None),
        Err(stop) => (None, Some(stop)),
    };
    let run = session.startup();
    Ok(StartRun {
        reached,
        stop,
        last: log.last,
        scripts: log.scripts,
        yields: session.yields(),
        steps: run.steps(),
        advances: log.advances,
        trace: session.trace_digest(),
        frames: session.frame_digests().to_vec(),
        state: state_digest(run),
        events: session.take_events(),
    })
}

impl StartRun {
    pub fn json(&self) -> Json {
        Json::obj([
            ("steps", Json::from(self.steps)),
            ("yields", self.yields.into()),
            ("advances", self.advances.into()),
            (
                "start_page",
                self.reached
                    .as_ref()
                    .map_or(Json::Null, PageSignature::json),
            ),
            ("stop", self.stop.as_ref().map(|s| s.describe()).into()),
            (
                "last",
                self.last.as_ref().map_or(Json::Null, Observation::json),
            ),
            (
                "scripts",
                Json::arr(self.scripts.iter().map(|s| s.as_str().into())),
            ),
            ("trace_sha256", to_hex(&self.trace).into()),
            ("state_sha256", self.state.as_str().into()),
        ])
    }
    fn outcome(&self) -> Outcome {
        match (&self.reached, &self.stop) {
            (Some(_), _) => Outcome::Pass(Level::L0),
            (None, Some(Stop::Blocked(b))) => Outcome::Blocked(b.clone()),
            (None, Some(stop)) => Outcome::Fail(stop.describe()),
            (None, None) => Outcome::Fail("ended without a stable page".into()),
        }
    }
}

const R1_TITLE: &str = "fresh start reaches a stable interactive page";
const D1_TITLE: &str = "identical input reproduces the frame trace and final state";

pub fn start_page_result(run: &Result<StartRun, String>) -> CheckResult {
    match run {
        Ok(run) => CheckResult::new("R1", R1_TITLE, run.outcome()).with(run.json()),
        Err(e) => CheckResult::new("R1", R1_TITLE, Outcome::Fail(e.clone())),
    }
}

pub fn skipped(id: &'static str, why: String) -> CheckResult {
    let title = if id == "R1" { R1_TITLE } else { D1_TITLE };
    CheckResult::new(id, title, Outcome::Skipped(why))
}

/// Compares two runs of the same procedure. A run that stopped early is still compared:
/// a gate stop that reproduces exactly is still deterministic behaviour.
pub fn determinism_result(
    first: &Result<StartRun, String>,
    second: &Result<StartRun, String>,
) -> CheckResult {
    let (first, second) = match (first, second) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            return CheckResult::new("D1", D1_TITLE, Outcome::Fail(e.clone()));
        }
    };
    let divergence = first
        .frames
        .iter()
        .zip(&second.frames)
        .position(|(a, b)| a != b)
        .or((first.frames.len() != second.frames.len())
            .then(|| first.frames.len().min(second.frames.len())));
    let same = divergence.is_none() && first.trace == second.trace && first.state == second.state;
    let outcome = if same && first.frames.is_empty() {
        // Equal digests over no frames compare almost nothing.
        Outcome::Inconclusive("no frame was observed before the run stopped".into())
    } else if same {
        Outcome::Pass(Level::L1)
    } else {
        Outcome::Fail(match divergence {
            Some(frame) => format!("frame traces diverge at frame {}", frame + 1),
            None => "final state digests differ".into(),
        })
    };
    CheckResult::new("D1", D1_TITLE, outcome).with(Json::obj([
        ("frames", Json::from(first.frames.len())),
        ("first_trace_sha256", to_hex(&first.trace).into()),
        ("second_trace_sha256", to_hex(&second.trace).into()),
        ("first_state_sha256", first.state.as_str().into()),
        ("second_state_sha256", second.state.as_str().into()),
    ]))
}

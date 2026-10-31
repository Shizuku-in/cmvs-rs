//! Drives a session through the scenario and reports the moments a check acts on: message
//! waits, choices and the return to the start page. Nothing here names a script: the
//! scenario starts at the first message wait after the entry button, a choice is a settled
//! button group never visible during a message wait, and the scenario ends when a page
//! settles in the start page's script.
use crate::{
    device::Action,
    json::Json,
    observe::{GroupShape, Observation, PageSignature, Settle},
    session::{Session, Stop},
};
use std::collections::HashSet;

/// Frames an interactive page must stay unchanged to count as settled.
pub const STABLE_FRAMES: u32 = 60;
/// Frames a choice candidate must stay unchanged before it is reported.
const CHOICE_FRAMES: u32 = 30;
/// Quiet frames after which a confirm-to-continue loop gets one advance.
const NUDGE_FRAMES: u64 = 300;

/// A choice the caller must answer with [`Scenario::answer`].
#[derive(Debug, Clone)]
pub struct Choice {
    pub observation: Observation,
    pub shape: GroupShape,
    pub buttons: usize,
    pub enabled: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A message wait. The caller advances with [`Scenario::advance`] or does something else;
    /// asking for the next event without acting returns the same wait again.
    Wait(Observation),
    Choice(Choice),
    /// A page settled in the start page's script.
    End(PageSignature),
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
            session.act(Action::Advance).map_err(Stop::Error)?;
            log.advances += 1;
        }
        let settled = settle.feed(&observation).cloned();
        log.last = Some(observation);
        if let Some(page) = settled {
            return Ok(page);
        }
    }
}

pub struct Scenario {
    pub start: PageSignature,
    /// Hold the skip key once the scenario starts.
    skip: bool,
    ambient: HashSet<(usize, [u8; 32])>,
    /// Choice shapes already answered; cleared once no candidate is visible.
    answered: HashSet<(usize, [u8; 32])>,
    pending: Vec<GroupShape>,
    pending_frames: u32,
    settle: Settle,
    quiet_since: u64,
    in_scenario: bool,
    pub advances: u64,
    pub nudges: u64,
}

impl Scenario {
    /// Reaches the start page and presses the entry button.
    pub fn begin(
        session: &mut Session<'_>,
        entry: (usize, usize),
        skip: bool,
    ) -> Result<Self, Stop> {
        let start = drive_to_start(session, &mut StartLog::default())?;
        session
            .act(Action::Press {
                group: entry.0,
                button: entry.1,
            })
            .map_err(Stop::Error)?;
        Ok(Self::new(session, start, skip, false))
    }
    /// Continues a session that is already inside the scenario, for example right after a
    /// gameplay load from the start page.
    pub fn resume(session: &Session<'_>, start: PageSignature, skip: bool) -> Self {
        Self::new(session, start, skip, true)
    }
    fn new(session: &Session<'_>, start: PageSignature, skip: bool, in_scenario: bool) -> Self {
        Self {
            start,
            skip,
            ambient: HashSet::new(),
            answered: HashSet::new(),
            pending: Vec::new(),
            pending_frames: 0,
            settle: Settle::new(STABLE_FRAMES),
            quiet_since: session.yields(),
            in_scenario,
            advances: 0,
            nudges: 0,
        }
    }
    pub fn in_scenario(&self) -> bool {
        self.in_scenario
    }
    /// Advances the current message wait.
    pub fn advance(&mut self, session: &mut Session<'_>) -> Result<(), Stop> {
        session.act(Action::Advance).map_err(Stop::Error)?;
        self.advances += 1;
        self.quiet_since = session.yields();
        Ok(())
    }
    /// Presses `button` of the reported choice.
    pub fn answer(
        &mut self,
        session: &mut Session<'_>,
        choice: &Choice,
        button: usize,
    ) -> Result<(), Stop> {
        self.answered
            .insert((choice.shape.slot, choice.shape.layout));
        self.pending.clear();
        self.pending_frames = 0;
        session
            .act(Action::Press {
                group: choice.shape.slot,
                button,
            })
            .map_err(Stop::Error)?;
        self.quiet_since = session.yields();
        Ok(())
    }
    /// Runs frames until the next event.
    pub fn next(&mut self, session: &mut Session<'_>) -> Result<Event, Stop> {
        loop {
            let o = session.frame()?;
            if o.waiting_for_advance() {
                if !self.in_scenario && self.skip && hold_skip(session) {
                    session.note("skip-held", Json::Null);
                }
                self.in_scenario = true;
                for g in &o.page.groups {
                    self.ambient.insert((g.slot, g.layout));
                }
                self.quiet_since = o.yields;
                self.pending.clear();
                if !session.busy() {
                    return Ok(Event::Wait(o));
                }
                continue;
            }
            if session.busy() {
                continue;
            }
            // Finishing a route can unlock start-page buttons, so only the script must match.
            if self.in_scenario
                && let Some(page) = self.settle.feed(&o)
                && page.script == self.start.script
            {
                let page = page.clone();
                session.note(
                    "route-end",
                    Json::obj([("changed", Json::from(page != self.start))]),
                );
                return Ok(Event::End(page));
            }
            // Inside the start page's script the scenario is ending: wait for the page to
            // settle instead of reporting its buttons as a choice.
            if !self.in_scenario || o.transition || o.script == self.start.script {
                continue;
            }
            let candidates: Vec<GroupShape> = o
                .page
                .groups
                .iter()
                .filter(|g| g.enabled > 0 && !self.ambient.contains(&(g.slot, g.layout)))
                .cloned()
                .collect();
            if candidates.is_empty() {
                self.answered.clear();
            }
            let found: Vec<GroupShape> = candidates
                .into_iter()
                .filter(|g| !self.answered.contains(&(g.slot, g.layout)))
                .collect();
            if !found.is_empty() && found == self.pending {
                self.pending_frames += 1;
            } else {
                self.pending_frames = 0;
                self.pending = found;
            }
            if self.pending_frames >= CHOICE_FRAMES {
                if self.pending.len() > 1 {
                    let slots: Vec<_> = self.pending.iter().map(|g| g.slot).collect();
                    return Err(Stop::Error(format!(
                        "ambiguous-choice: candidate groups {slots:?}"
                    )));
                }
                let shape = self.pending[0].clone();
                let Some(group) = session.startup().buttons().group(shape.slot) else {
                    return Err(Stop::Error("choice group vanished".into()));
                };
                let enabled = group
                    .buttons
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.enabled)
                    .map(|(i, _)| i)
                    .collect();
                let buttons = group.buttons.len();
                return Ok(Event::Choice(Choice {
                    observation: o,
                    shape,
                    buttons,
                    enabled,
                }));
            }
            let movie = session.startup().movies().player().is_some();
            if self.pending.is_empty() && !movie && o.yields - self.quiet_since >= NUDGE_FRAMES {
                session.act(Action::Advance).map_err(Stop::Error)?;
                self.nudges += 1;
                session.note("nudge", Json::Null);
                self.quiet_since = o.yields;
            }
        }
    }
}

/// Starts holding skip; false when key.cfg binds no key to it.
fn hold_skip(session: &mut Session<'_>) -> bool {
    match crate::device::skip_code(session.bindings()) {
        Some(code) => {
            session.hold(vec![code]);
            true
        }
        None => false,
    }
}

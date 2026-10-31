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
    /// Text shown with the choice, decoded as Shift-JIS, in slot order.
    pub texts: Vec<String>,
}

/// Decodes glyph codes as Shift-JIS. Double-byte pairs keep the lead byte high; single
/// bytes are stored sign-extended, so 0xff00..=0xffff is one byte.
fn decode_codes(codes: impl Iterator<Item = u16>) -> String {
    let mut bytes = Vec::new();
    for code in codes {
        if code >= 0xff00 || code <= 0xff {
            bytes.push(code as u8);
        } else {
            bytes.extend(code.to_be_bytes());
        }
    }
    encoding_rs::SHIFT_JIS.decode(&bytes).0.into_owned()
}

/// Splits glyphs into lines where their vertical position changes; choice options placed
/// in one text object usually sit on separate lines.
fn lines(glyphs: impl Iterator<Item = (u16, i32)>) -> Vec<String> {
    let mut lines: Vec<Vec<u16>> = Vec::new();
    let mut last = None;
    for (code, y) in glyphs {
        if last != Some(y) || lines.is_empty() {
            lines.push(Vec::new());
            last = Some(y);
        }
        lines.last_mut().expect("pushed").push(code);
    }
    lines
        .into_iter()
        .map(|codes| decode_codes(codes.into_iter()))
        .filter(|text| !text.trim().is_empty())
        .collect()
}

/// Text currently shown, line by line: visible text objects (where placed choice options
/// are drawn) and auxiliary texts, in slot order.
pub fn shown_texts(session: &Session<'_>) -> Vec<String> {
    let messages = session.startup().messages();
    let mut texts = Vec::new();
    for slot in 0..cmvs_core::message::TEXT_OBJECTS {
        if let Some(text) = messages.text(slot).filter(|t| t.visible) {
            texts.extend(lines(text.glyphs.iter().map(|g| (g.code, g.position[1]))));
        }
    }
    for slot in 0..cmvs_core::message::AUXILIARY_TEXTS {
        if let Some(text) = messages.auxiliary_text(slot) {
            texts.extend(lines(text.glyphs.iter().map(|g| (g.code, g.position[1]))));
        }
    }
    texts
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
    pub fn resume(session: &mut Session<'_>, start: PageSignature, skip: bool) -> Self {
        if skip && hold_skip(session) {
            session.note("skip-held", Json::Null);
        }
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
            if let Some(event) = self.next_within(session, u64::MAX)? {
                return Ok(event);
            }
        }
    }
    /// Runs at most `frames` frames; `None` when no event happened in that time.
    pub fn next_within(
        &mut self,
        session: &mut Session<'_>,
        frames: u64,
    ) -> Result<Option<Event>, Stop> {
        let from = session.yields();
        loop {
            if session.yields() - from >= frames {
                return Ok(None);
            }
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
                    return Ok(Some(Event::Wait(o)));
                }
                continue;
            }
            if session.busy() {
                continue;
            }
            // Polling a timed sequence is active execution, not a quiet confirm
            // loop. A frame-count nudge could finish it early at one frame time.
            if session.startup().sequence().header().active != 0 {
                self.quiet_since = o.yields;
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
                return Ok(Some(Event::End(page)));
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
                    let script = o.script.escape_ascii().to_string();
                    let preferred = session.manifest().and_then(|m| m.choice_group(&script));
                    match preferred
                        .and_then(|slot| self.pending.iter().position(|g| g.slot == slot))
                    {
                        Some(index) => {
                            let chosen = self.pending.remove(index);
                            self.pending = vec![chosen];
                        }
                        None => {
                            let slots: Vec<_> = self.pending.iter().map(|g| g.slot).collect();
                            return Err(Stop::Error(format!(
                                "ambiguous-choice: candidate groups {slots:?}"
                            )));
                        }
                    }
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
                let texts = shown_texts(session);
                return Ok(Some(Event::Choice(Choice {
                    observation: o,
                    shape,
                    buttons,
                    enabled,
                    texts,
                })));
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

/// What is visible: the page, the shown scene objects and message windows. Two moments
/// with the same view look the same to the player.
pub fn view_digest(session: &Session<'_>, o: &Observation) -> String {
    let run = session.startup();
    let objects: Vec<_> = run
        .scene()
        .objects()
        .iter()
        .filter(|(_, object)| object.shown)
        .map(|(slot, _)| *slot)
        .collect();
    let windows: Vec<bool> = (0..cmvs_core::message::MESSAGE_WINDOWS)
        .map(|slot| run.messages().window(slot).is_some_and(|w| w.visible))
        .collect();
    let text = format!("{} {objects:?} {windows:?}", o.page.digest());
    cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(text.as_bytes()))[..16].to_owned()
}

/// Where a message wait stood, and a digest of the newest backlog entry (its text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitPoint {
    pub script: String,
    pub pc: usize,
    pub text: String,
}

impl WaitPoint {
    pub fn of(session: &Session<'_>, o: &Observation) -> Self {
        // Only the entry's strings: its state words may change while the backlog is shown.
        let newest = session
            .startup()
            .history()
            .entries()
            .front()
            .map(|entry| entry.strings.concat());
        let text =
            cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(format!("{newest:?}").as_bytes()))
                [..12]
                .to_owned();
        Self {
            script: o.script.escape_ascii().to_string(),
            pc: o.pc,
            text,
        }
    }
    pub fn json(&self) -> Json {
        Json::obj([
            ("script", Json::from(self.script.as_str())),
            ("pc", Json::hex(self.pc as u64)),
            ("text", self.text.as_str().into()),
        ])
    }
}

/// Follows the scenario for `count` message waits, advancing each and answering every
/// choice with its first enabled button. Stops early at the end of the scenario.
pub fn follow(
    session: &mut Session<'_>,
    scenario: &mut Scenario,
    count: usize,
) -> Result<Vec<WaitPoint>, Stop> {
    let mut points = Vec::new();
    while points.len() < count {
        match scenario.next(session)? {
            Event::Wait(o) => {
                points.push(WaitPoint::of(session, &o));
                if points.len() < count {
                    scenario.advance(session)?;
                }
            }
            Event::Choice(choice) => {
                let first = choice.enabled[0];
                scenario.answer(session, &choice, first)?;
            }
            Event::End(_) => break,
        }
    }
    Ok(points)
}

/// The first index where two wait traces differ, or the shorter length when one is a
/// prefix of the other; `None` when they are equal.
pub fn first_difference(a: &[WaitPoint], b: &[WaitPoint]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or((a.len() != b.len()).then(|| a.len().min(b.len())))
}

/// Explains a divergence at `at`: one trace may hold an extra wait there while the rest
/// matches, which reads differently from a different story.
pub fn describe_divergence(a: &[WaitPoint], b: &[WaitPoint], at: usize) -> String {
    let rest_matches = |longer: &[WaitPoint], shorter: &[WaitPoint]| {
        let tail = longer.get(at + 1..).unwrap_or(&[]);
        let other = shorter.get(at..).unwrap_or(&[]);
        let n = tail.len().min(other.len());
        n > 0 && tail[..n] == other[..n]
    };
    if rest_matches(b, a) {
        format!("the second run has one extra message wait at {at}; the rest matches")
    } else if rest_matches(a, b) {
        format!("the first run has one extra message wait at {at}; the rest matches")
    } else {
        format!("message wait {at} differs and the traces do not realign")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sign_extended_single_bytes_and_splits_lines() {
        // 0x82a0 is a double-byte hiragana; 0xffb1 is the half-width katakana byte 0xb1.
        let text = decode_codes([0x82a0u16, 0xffb1, 0x0041].into_iter());
        assert_eq!(text, "\u{3042}\u{ff71}A");
        let split = lines([(0x0041u16, 0), (0x0042, 0), (0x0043, 30)].into_iter());
        assert_eq!(split, ["AB", "C"]);
    }
}

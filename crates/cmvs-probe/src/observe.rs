//! Engine-level observations. Judgements use only these values: no
//! script name or address ever decides what state the game is in.
use crate::json::Json;
use cmvs_core::{
    buttons::BUTTON_GROUPS,
    digest::{Sha256, to_hex},
    runner::Startup,
};

/// System command 0x2153 (message wait), as the machine reports it without 0x2000.
const MESSAGE_WAIT: u16 = 0x153;
/// Rectangles are compared on this grid so sub-pixel layout motion is not a new page.
const RECT_GRID: i32 = 8;

/// One visible button group, by shape only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupShape {
    pub slot: usize,
    pub buttons: usize,
    pub enabled: usize,
    /// Quantised rectangles and enabled states.
    pub layout: [u8; 32],
}

/// A page is identified by the visible button groups and the current main script. The
/// script is part of the identity only; it is never used to guess what the page is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSignature {
    pub script: Vec<u8>,
    pub groups: Vec<GroupShape>,
}

impl PageSignature {
    pub fn of(run: &Startup<'_>) -> Self {
        let groups = (0..BUTTON_GROUPS)
            .filter_map(|slot| {
                let group = run.buttons().group(slot)?;
                if group.buttons.is_empty() {
                    return None;
                }
                let mut layout = Sha256::new();
                for button in &group.buttons {
                    for v in button.rect {
                        layout.update(&v.div_euclid(RECT_GRID).to_le_bytes());
                    }
                    layout.update(&[u8::from(button.enabled)]);
                }
                Some(GroupShape {
                    slot,
                    buttons: group.buttons.len(),
                    enabled: group.buttons.iter().filter(|b| b.enabled).count(),
                    layout: layout.finish(),
                })
            })
            .collect();
        Self {
            script: run.machine().current_name().to_vec(),
            groups,
        }
    }
    /// A page offers a choice of at least one enabled button.
    pub fn interactive(&self) -> bool {
        self.groups.iter().any(|g| g.enabled > 0)
    }
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(&self.script);
        for g in &self.groups {
            h.update(&(g.slot as u32).to_le_bytes());
            h.update(&(g.buttons as u32).to_le_bytes());
            h.update(&g.layout);
        }
        to_hex(&h.finish())[..16].to_owned()
    }
    pub fn json(&self) -> Json {
        Json::obj([
            ("digest", Json::from(self.digest())),
            ("script", Json::bytes(&self.script)),
            (
                "groups",
                Json::arr(self.groups.iter().map(|g| {
                    Json::obj([
                        ("slot", Json::from(g.slot)),
                        ("buttons", g.buttons.into()),
                        ("enabled", g.enabled.into()),
                    ])
                })),
            ),
        ])
    }
}

/// State after one VM frame (one yield).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub yields: u64,
    pub steps: u64,
    pub script: Vec<u8>,
    pub pc: usize,
    pub pending: Option<u16>,
    pub repeat: u32,
    /// A scene fade is running; the page is still changing even if its buttons are not.
    pub transition: bool,
    pub page: PageSignature,
}

impl Observation {
    pub fn of(run: &Startup<'_>, yields: u64) -> Self {
        let machine = run.machine();
        Self {
            yields,
            steps: run.steps(),
            script: machine.current_name().to_vec(),
            pc: machine.pc(),
            pending: machine.pending_system().map(|(_, c)| c),
            repeat: machine.repeat_count(),
            transition: run.scene().fade().current().is_some(),
            page: PageSignature::of(run),
        }
    }
    /// The message wait has entered its waiting phase and wants a decide release.
    pub fn waiting_for_advance(&self) -> bool {
        self.pending == Some(MESSAGE_WAIT) && self.repeat == 2
    }
    /// A wait can resume after a script callback with its repeat count intact.
    /// Use this for returning from a page; `waiting_for_advance` marks only a new wait.
    pub fn in_message_wait(&self) -> bool {
        self.pending == Some(MESSAGE_WAIT) && self.repeat >= 2
    }
    /// Feeds the per-frame trace digest used by determinism checks.
    pub fn hash_into(&self, h: &mut Sha256) {
        h.update(&self.steps.to_le_bytes());
        h.update(&self.script);
        h.update(&(self.pc as u64).to_le_bytes());
        h.update(&u32::from(self.pending.unwrap_or(u16::MAX)).to_le_bytes());
        h.update(&self.repeat.to_le_bytes());
        h.update(&[u8::from(self.transition)]);
        h.update(self.page.digest().as_bytes());
    }
    pub fn json(&self) -> Json {
        Json::obj([
            ("yields", Json::from(self.yields)),
            ("steps", self.steps.into()),
            ("script", Json::bytes(&self.script)),
            ("pc", Json::hex(self.pc as u64)),
            (
                "pending",
                self.pending
                    .map(|c| Json::hex(0x2000 + u64::from(c)))
                    .unwrap_or(Json::Null),
            ),
            ("transition", self.transition.into()),
            ("page", self.page.json()),
        ])
    }
}

/// Coarse digest of VM, scene and button state for comparing two runs of the same build.
/// Volatile fields such as timers are not excluded yet, so it only compares runs that should be
/// identical step for step.
pub fn state_digest(run: &Startup<'_>) -> String {
    let mut h = Sha256::new();
    h.update(format!("{:?}", run.machine().state()).as_bytes());
    match run.machine().capture_vm() {
        Ok(image) => h.update(format!("{image:?}").as_bytes()),
        Err(e) => h.update(e.as_bytes()),
    }
    h.update(format!("{:?}", run.scene().objects()).as_bytes());
    for slot in 0..BUTTON_GROUPS {
        h.update(format!("{:?}", run.buttons().group(slot)).as_bytes());
    }
    to_hex(&h.finish())
}

/// Detects a settled page: one with an enabled button, outside message waits and scene
/// fades, unchanged for `need` consecutive frames.
#[derive(Debug, Clone)]
pub struct Settle {
    need: u32,
    frames: u32,
    candidate: Option<PageSignature>,
}

impl Settle {
    pub fn new(need: u32) -> Self {
        Self {
            need,
            frames: 0,
            candidate: None,
        }
    }
    pub fn reset(&mut self) {
        self.frames = 0;
        self.candidate = None;
    }
    pub fn feed(&mut self, o: &Observation) -> Option<&PageSignature> {
        let interactive = o.page.interactive() && !o.waiting_for_advance() && !o.transition;
        if interactive && self.candidate.as_ref() == Some(&o.page) {
            self.frames += 1;
        } else {
            self.frames = 0;
            self.candidate = interactive.then(|| o.page.clone());
        }
        if self.frames >= self.need {
            self.candidate.as_ref()
        } else {
            None
        }
    }
}

/// Digest of the VM variable state, used to recognise the same choice point reached by
/// different paths.
pub fn variables_digest(run: &Startup<'_>) -> String {
    let mut h = Sha256::new();
    h.update(run.machine().current_name());
    h.update(&(run.machine().pc() as u64).to_le_bytes());
    h.update(format!("{:?}", run.machine().state()).as_bytes());
    to_hex(&h.finish())[..32].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        buttons: usize,
        enabled: usize,
        pending: Option<u16>,
        fade: bool,
    ) -> Observation {
        Observation {
            yields: 0,
            steps: 0,
            script: b"s".to_vec(),
            pc: 0,
            pending,
            repeat: 2,
            transition: fade,
            page: PageSignature {
                script: b"s".to_vec(),
                groups: vec![GroupShape {
                    slot: 0,
                    buttons,
                    enabled,
                    layout: [buttons as u8; 32],
                }],
            },
        }
    }

    #[test]
    fn settle_needs_unchanged_interactive_frames_outside_waits_and_fades() {
        let mut settle = Settle::new(2);
        let page = observation(4, 4, None, false);
        assert!(settle.feed(&page).is_none());
        assert!(settle.feed(&page).is_none());
        assert_eq!(settle.feed(&page), Some(&page.page));
        // A fade, a message wait or a changed page restarts the count.
        for interrupt in [
            observation(4, 4, None, true),
            observation(4, 4, Some(MESSAGE_WAIT), false),
            observation(5, 5, None, false),
        ] {
            settle.feed(&interrupt);
            assert!(settle.feed(&page).is_none());
        }
        // Disabled-only pages never settle.
        let mut settle = Settle::new(0);
        assert!(settle.feed(&observation(3, 0, None, false)).is_none());
    }
}

//! Session layer (design section 4.1): one headless run with fixed seed, virtual frame time,
//! recording backends and an isolated user directory. The game root is only read.
use crate::{
    device::{Action, Device, Frame},
    json::Json,
    observe::Observation,
    report::Blocked,
    target::{Target, cap},
};
use cmvs_core::{
    digest::Sha256,
    input::{CodeInput, KeyBindings},
    platform::RecordingBackend,
    profile::{GateError, GateErrorKind},
    ps3::Script,
    runner::{SnapshotPolicy, Startup, StartupEvent},
};
use std::{collections::VecDeque, path::Path};

#[derive(Debug, Clone)]
pub struct Options {
    pub device: Device,
    pub frame_ms: u32,
    pub seed: u32,
    /// Total VM steps per session; the probe's budget, not an engine limit.
    pub max_steps: u64,
    pub allow_presumed: bool,
    pub allow_unreviewed: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            device: Device::Mouse,
            frame_ms: 16,
            seed: 0,
            max_steps: 50_000_000,
            allow_presumed: false,
            allow_unreviewed: false,
        }
    }
}

impl Options {
    pub fn json(&self) -> Json {
        Json::obj([
            ("device", Json::from(self.device.name())),
            ("frame_ms", self.frame_ms.into()),
            ("seed", self.seed.into()),
            ("max_steps", self.max_steps.into()),
            ("allow_presumed", self.allow_presumed.into()),
            ("allow_unreviewed", self.allow_unreviewed.into()),
        ])
    }
}

/// Why a session stopped before the check's own goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The script requested quit (0x2001).
    Quit,
    /// The command gate stopped, or no host handles the command.
    Blocked(Blocked),
    /// The probe's step budget ran out.
    Budget,
    /// Any other runtime error or unexpected event.
    Error(String),
}

impl Stop {
    pub fn describe(&self) -> String {
        match self {
            Self::Quit => "script requested quit".into(),
            Self::Blocked(b) => format!(
                "{} 0x{:04x} at {} / 0x{:x}",
                b.kind, b.command, b.script, b.pc
            ),
            Self::Budget => "step budget exhausted".into(),
            Self::Error(e) => e.clone(),
        }
    }
}

fn gate_kind(kind: GateErrorKind) -> &'static str {
    match kind {
        GateErrorKind::Absent => "absent",
        GateErrorKind::NoOp => "noop",
        GateErrorKind::Unreviewed => "unreviewed",
        GateErrorKind::Presumed => "presumed",
        GateErrorKind::GameIdMismatch { .. } => "game-id-mismatch",
    }
}

pub struct Session<'a> {
    run: Startup<'a>,
    backend: RecordingBackend,
    codes: CodeInput,
    bindings: KeyBindings,
    device: Device,
    frame_ms: u32,
    max_steps: u64,
    queue: VecDeque<Frame>,
    pointer: [i32; 2],
    yields: u64,
    trace: Sha256,
    /// Per-frame trace digests, kept to locate the first divergence between runs.
    frames: Vec<[u8; 8]>,
    events: Vec<Json>,
    last_script: Vec<u8>,
    last_page: Option<String>,
}

impl<'a> Session<'a> {
    /// `user` must be a new, empty directory; the engine writes system and slot saves there.
    pub fn start(
        target: &'a Target,
        script: &'a Script,
        user: &Path,
        options: &Options,
        font: Option<&cmvs_desktop::font::FontRenderer>,
    ) -> Result<Self, String> {
        let game = target.game.as_ref().ok_or("no engine profile selected")?;
        let mut run = Startup::new(script, &target.root, game, 0).map_err(|e| e.to_string())?;
        run.set_allow_presumed(options.allow_presumed);
        run.set_allow_unreviewed(options.allow_unreviewed);
        run.set_user_data_directory(user)
            .map_err(|e| e.to_string())?;
        // Data-driven: only records that verify the policy enable it.
        if target.has(cap::AUTO_RESUME_POLICY) {
            run.set_skip_auto_resume(true)?;
        }
        if !target.has(cap::SNAPSHOT_LAYOUT) {
            // Capability, not a game branch: the live-state effect still applies.
            run.set_snapshot_policy(SnapshotPolicy::ContinueWithoutImage);
        }
        run.set_record_text(true);
        if let Some(font) = font {
            run.set_glyph_metrics(font.metrics());
            if !font.has_face(run.settings().font_face()) {
                run.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)
                    .map_err(|e| e.to_string())?;
            }
        }
        run.set_random_seed(options.seed);
        Ok(Self {
            run,
            backend: RecordingBackend::default(),
            codes: CodeInput::default(),
            bindings: target.key_bindings.clone().unwrap_or_default(),
            device: options.device,
            frame_ms: options.frame_ms,
            max_steps: options.max_steps,
            queue: VecDeque::new(),
            pointer: crate::device::OFF_SCREEN,
            yields: 0,
            trace: Sha256::new(),
            frames: Vec::new(),
            events: Vec::new(),
            last_script: Vec::new(),
            last_page: None,
        })
    }
    pub fn startup(&self) -> &Startup<'a> {
        &self.run
    }
    pub fn yields(&self) -> u64 {
        self.yields
    }
    /// True while queued input frames remain.
    pub fn busy(&self) -> bool {
        !self.queue.is_empty()
    }
    /// Queues an action; it takes effect over the next frames.
    pub fn act(&mut self, action: Action) -> Result<(), String> {
        let buttons = self.run.buttons();
        let frames = self.device.frames(action, &self.bindings, |g, b| {
            let button = buttons.group(g)?.buttons.get(b)?;
            Some([
                button.rect[0] + button.rect[2] / 2,
                button.rect[1] + button.rect[3] / 2,
            ])
        })?;
        self.event(
            "action",
            Json::obj([("action", Json::from(format!("{action:?}")))]),
        );
        self.queue.extend(frames);
        Ok(())
    }
    /// Runs the VM to its next yield, then advances virtual time, applies the next input
    /// frame and observes. Events are recorded for script and page changes.
    pub fn frame(&mut self) -> Result<Observation, Stop> {
        loop {
            if self.run.steps() >= self.max_steps {
                return Err(self.stop(Stop::Budget));
            }
            let event = match self.run.tick(&mut self.backend) {
                Ok(event) => event,
                Err(error) => {
                    let stop = match error.downcast_ref::<GateError>() {
                        Some(g) => Stop::Blocked(Blocked {
                            command: 0x2000 + g.command,
                            script: g.script.clone(),
                            pc: g.pc,
                            kind: gate_kind(g.kind).into(),
                        }),
                        None => Stop::Error(error.to_string()),
                    };
                    return Err(self.stop(stop));
                }
            };
            for command in self.run.take_unreviewed_notices() {
                self.event(
                    "unreviewed-allowed",
                    Json::obj([("command", Json::hex(0x2000 + u64::from(command)))]),
                );
            }
            match event {
                StartupEvent::Yield => break,
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::MainEnteredWithoutSnapshot => {
                    self.event("main-without-snapshot", Json::Null);
                }
                StartupEvent::Quit => return Err(self.stop(Stop::Quit)),
                StartupEvent::Unhandled { pc, command } => {
                    let blocked = Blocked {
                        command: 0x2000 + command,
                        script: self.run.machine().current_name().escape_ascii().to_string(),
                        pc,
                        kind: "unhandled".into(),
                    };
                    return Err(self.stop(Stop::Blocked(blocked)));
                }
                other => return Err(self.stop(Stop::Error(format!("{other:?}")))),
            }
        }
        self.yields += 1;
        self.run
            .advance_frame(self.frame_ms)
            .map_err(|e| Stop::Error(e.to_string()))?;
        let frame = self.queue.pop_front().unwrap_or_default();
        if let Some(pointer) = frame.pointer {
            self.pointer = pointer;
        }
        let mut now = [false; 128];
        for code in frame.codes {
            now[code] = true;
        }
        self.codes
            .update(&self.bindings, &now, self.run.input_mut());
        self.run
            .input_mut()
            .set_pointer(self.pointer[0], self.pointer[1]);
        let observation = Observation::of(&self.run, self.yields);
        let mut digest = Sha256::new();
        observation.hash_into(&mut digest);
        let digest = digest.finish();
        self.trace.update(&digest);
        self.frames
            .push(digest[..8].try_into().expect("eight bytes"));
        self.note_changes(&observation);
        Ok(observation)
    }
    fn note_changes(&mut self, observation: &Observation) {
        if observation.script != self.last_script {
            self.last_script = observation.script.clone();
            self.event("script", Json::Null);
        }
        let page = observation.page.digest();
        if !observation.page.groups.is_empty() && self.last_page.as_deref() != Some(&page) {
            self.event("page", observation.page.json());
        }
        self.last_page = (!observation.page.groups.is_empty()).then_some(page);
    }
    fn stop(&mut self, stop: Stop) -> Stop {
        self.event("stop", Json::obj([("reason", Json::from(stop.describe()))]));
        stop
    }
    fn event(&mut self, kind: &str, payload: Json) {
        let machine = self.run.machine();
        let mut fields = vec![
            ("kind".to_owned(), Json::from(kind)),
            ("yields".to_owned(), self.yields.into()),
            ("steps".to_owned(), self.run.steps().into()),
            ("script".to_owned(), Json::bytes(machine.current_name())),
            ("pc".to_owned(), Json::hex(machine.pc() as u64)),
        ];
        if let Json::Obj(extra) = payload {
            // The location fields above win; payloads only add keys.
            for (key, value) in extra {
                if !fields.iter().any(|(k, _)| *k == key) {
                    fields.push((key, value));
                }
            }
        }
        self.events.push(Json::Obj(fields));
    }
    /// Digest of every observed frame so far.
    pub fn trace_digest(&self) -> [u8; 32] {
        self.trace.clone().finish()
    }
    pub fn frame_digests(&self) -> &[[u8; 8]] {
        &self.frames
    }
    pub fn take_events(&mut self) -> Vec<Json> {
        std::mem::take(&mut self.events)
    }
}

//! One headless run with fixed seed, virtual frame time, recording backends and an isolated
//! user directory. The game root is only read.
use crate::{
    device::{Action, Device, Frame},
    json::Json,
    observe::Observation,
    report::Blocked,
    target::{Target, cap},
};
use cmvs_core::{
    audio::RecordingAudio,
    digest::Sha256,
    input::{CodeInput, KeyBindings},
    platform::RecordingBackend,
    profile::{GateError, GateErrorKind},
    ps3::Script,
    runner::{SnapshotPolicy, Startup, StartupEvent},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Options {
    pub device: Device,
    pub frame_ms: u32,
    pub seed: u32,
    /// Total VM steps per session; the probe's budget, not an engine limit.
    pub max_steps: u64,
    /// Routes route exploration may run before it stops with unexplored prefixes.
    pub max_routes: usize,
    /// Unlock rounds before stopping while routes are still being discovered.
    pub max_rounds: usize,
    /// Actions tried while traversing a page graph.
    pub max_page_trials: usize,
    /// Route exploration holds the skip action inside the scenario to shorten routes.
    pub route_skip: bool,
    /// Route branches start from a saved checkpoint instead of replaying from the start.
    pub fork_saves: bool,
    /// Explicit research mode: keep snapshot commands' live effects but no save image.
    pub capture_snapshots: bool,
    /// Composite completed software frames for commands that read the device.
    pub render_frames: bool,
    pub block_presumed: bool,
    pub block_unreviewed: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            device: Device::Mouse,
            frame_ms: 16,
            seed: 0,
            max_steps: 50_000_000,
            max_routes: 16,
            max_rounds: 4,
            max_page_trials: 400,
            route_skip: true,
            fork_saves: false,
            capture_snapshots: true,
            render_frames: false,

            block_presumed: false,
            block_unreviewed: false,
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
            ("max_routes", self.max_routes.into()),
            ("max_rounds", self.max_rounds.into()),
            ("max_page_trials", self.max_page_trials.into()),
            ("route_skip", self.route_skip.into()),
            ("fork_saves", self.fork_saves.into()),
            ("capture_snapshots", self.capture_snapshots.into()),
            ("render_frames", self.render_frames.into()),
            ("block_presumed", self.block_presumed.into()),
            ("block_unreviewed", self.block_unreviewed.into()),
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
    render_frames: bool,
    font: Option<cmvs_desktop::font::FontRenderer>,
    codes: CodeInput,
    bindings: KeyBindings,
    device: Device,
    frame_ms: u32,
    max_steps: u64,
    route_skip: bool,
    fork_saves: bool,
    queue: VecDeque<Frame>,
    pointer: [i32; 2],
    yields: u64,
    trace: Sha256,
    /// Per-frame trace digests, kept to locate the first divergence between runs.
    frames: Vec<[u8; 8]>,
    events: Vec<Json>,
    last_script: Vec<u8>,
    last_page: Option<String>,
    coverage: Option<Coverage>,
    /// Codes held on every frame in addition to queued input.
    held: Vec<usize>,
    user: PathBuf,
    /// A keyboard press in progress: group, button and navigation taps so far.
    navigation: Option<(usize, usize, usize)>,
    /// Native dialogs answered so far; each is cancelled, as a player closing it would.
    dialogs: usize,
    manifest: Option<&'a crate::manifest::Manifest>,
}

/// Executed instruction addresses by script name: the PC before each VM step.
#[derive(Debug, Default, Clone)]
pub struct Coverage {
    pub scripts: HashMap<Vec<u8>, HashSet<u32>>,
}

impl Coverage {
    fn record(&mut self, script: &[u8], pc: usize) {
        let pc = pc as u32;
        match self.scripts.get_mut(script) {
            Some(set) => {
                set.insert(pc);
            }
            None => {
                self.scripts.insert(script.to_vec(), HashSet::from([pc]));
            }
        }
    }
    pub fn merge(&mut self, other: Coverage) {
        for (script, pcs) in other.scripts {
            self.scripts.entry(script).or_default().extend(pcs);
        }
    }
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
        if options.render_frames && font.is_none() {
            return Err("software frame rendering requires a font".into());
        }
        let game = target.game.as_ref().ok_or("no engine profile selected")?;
        let mut run = Startup::new(script, &target.root, game, 0).map_err(|e| e.to_string())?;
        run.set_mgv_factory(Box::new(cmvs_desktop::video::TheoraFactory));
        run.set_block_presumed(options.block_presumed);
        run.set_block_unreviewed(options.block_unreviewed);
        run.set_user_data_directory(user)
            .map_err(|e| e.to_string())?;
        // Data-driven: only records that verify the policy enable it.
        if target.has(cap::AUTO_RESUME_POLICY) {
            run.set_skip_auto_resume(true)?;
        }
        if !options.capture_snapshots || !target.has(cap::SNAPSHOT_LAYOUT) {
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
        let mut backend = RecordingBackend::default();
        backend.set_screen_size(
            cmvs_core::viewport::ScreenSize::new(run.screen_size()).map_err(|e| e.to_string())?,
        );
        Ok(Self {
            run,
            backend,
            render_frames: options.render_frames,
            font: if options.render_frames {
                font.cloned()
            } else {
                None
            },
            codes: CodeInput::default(),
            bindings: target.key_bindings.clone().unwrap_or_default(),
            device: options.device,
            frame_ms: options.frame_ms,
            max_steps: options.max_steps,
            route_skip: options.route_skip,
            // Forking needs gameplay saves, which need the snapshot layout.
            fork_saves: options.fork_saves
                && options.capture_snapshots
                && target.has(cap::SNAPSHOT_LAYOUT),

            queue: VecDeque::new(),
            pointer: crate::device::OFF_SCREEN,
            yields: 0,
            trace: Sha256::new(),
            frames: Vec::new(),
            events: Vec::new(),
            last_script: Vec::new(),
            last_page: None,
            coverage: None,
            held: Vec::new(),
            user: user.to_path_buf(),
            navigation: None,
            dialogs: 0,
            manifest: None,
        })
        .map(|mut session: Self| {
            session.backend.answer_font_dialog(None);
            session
        })
    }
    pub fn startup(&self) -> &Startup<'a> {
        &self.run
    }
    /// Starts recording executed addresses; costs one set insertion per VM step.
    pub fn record_coverage(&mut self) {
        self.coverage.get_or_insert_with(Coverage::default);
    }
    pub fn take_coverage(&mut self) -> Option<Coverage> {
        self.coverage.take()
    }
    /// Holds `codes` on every following frame until replaced.
    pub fn hold(&mut self, codes: Vec<usize>) {
        self.held = codes;
    }
    pub fn set_manifest(&mut self, manifest: Option<&'a crate::manifest::Manifest>) {
        self.manifest = manifest;
    }
    pub fn manifest(&self) -> Option<&'a crate::manifest::Manifest> {
        self.manifest
    }
    /// The isolated user directory; the engine keeps `system.dat` and slot saves there.
    pub fn user_dir(&self) -> &Path {
        &self.user
    }
    pub fn fork_saves(&self) -> bool {
        self.fork_saves
    }
    pub fn route_skip(&self) -> bool {
        self.route_skip
    }
    pub fn bindings(&self) -> &KeyBindings {
        &self.bindings
    }
    /// Records a check-level event at the current location.
    pub fn note(&mut self, kind: &str, payload: Json) {
        self.event(kind, payload);
    }
    pub fn yields(&self) -> u64 {
        self.yields
    }
    /// True while queued input frames remain or a keyboard press is still navigating.
    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || self.navigation.is_some()
    }
    pub fn device(&self) -> Device {
        self.device
    }
    /// Writes a gameplay save to `slot` as the script command would; false when the engine
    /// has not built a snapshot yet.
    pub fn save_slot(&mut self, slot: u16) -> Result<bool, String> {
        self.run.save_slot(slot).map_err(|e| e.to_string())
    }
    /// Loads a gameplay save, replacing the VM and every saved host.
    pub fn load_slot(&mut self, slot: u16) -> Result<(), String> {
        self.run
            .load_slot(slot, &mut self.backend)
            .map_err(|e| e.to_string())
    }
    /// Writes the user's preferences as a graceful exit does: the system save and cmvs.cfg.
    pub fn save_preferences(&mut self) -> Result<bool, String> {
        let display = self.run.display_preferences();
        self.run
            .save_preferences(display)
            .map_err(|e| e.to_string())
    }
    /// Player-facing settings: text, auto advance, read skip and audio mixer values.
    pub fn settings_fingerprint(&self) -> String {
        format!(
            "{:?} {:?} {:?} background_output: {}",
            self.run.settings(),
            self.run.audio().settings(),
            self.run.audio().voice_ducking(),
            self.run.audio().background_output()
        )
    }
    /// Live resource counts that a long run should not grow without bound.
    pub fn resource_counts(&self) -> [usize; 5] {
        let run = &self.run;
        let objects = run.scene().objects().len();
        let channels = (0..32)
            .filter(|&i| run.audio().channel(i).is_some())
            .count();
        let groups = (0..cmvs_core::buttons::BUTTON_GROUPS)
            .filter(|&g| run.buttons().group(g).is_some())
            .count()
            + (cmvs_core::buttons::BUTTON_GROUPS..crate::controls::GROUPS)
                .filter(|&g| {
                    run.buttons()
                        .list_group(g - cmvs_core::buttons::BUTTON_GROUPS)
                        .is_some()
                })
                .count();
        let sequences = run.sequence().entries().iter().flatten().count();
        let history = run.history().entries().len();
        [objects, channels, groups, sequences, history]
    }
    /// Queues an action; it takes effect over the next frames.
    pub fn act(&mut self, action: Action) -> Result<(), String> {
        if let (Device::Keyboard, Action::Press { group, button }) = (self.device, action) {
            let present =
                crate::controls::group(&self.run, group).is_some_and(|g| button < g.buttons.len());
            if !present {
                return Err(format!("button {group}:{button} is not present"));
            }
            if group
                .checked_sub(cmvs_core::buttons::BUTTON_GROUPS)
                .and_then(|slot| self.run.buttons().list_group(slot))
                .is_some_and(|list| list.disable_navigation)
            {
                return Err(format!("keyboard navigation is disabled for group {group}"));
            }
            self.event(
                "action",
                Json::obj([("action", Json::from(format!("{action:?}")))]),
            );
            self.navigation = Some((group, button, 0));
            return Ok(());
        }
        let frames = self.device.frames(action, &self.bindings, |g, b| {
            crate::controls::group(&self.run, g)?.point(b)
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
            if let Some(coverage) = &mut self.coverage {
                let machine = self.run.machine();
                coverage.record(machine.current_name(), machine.pc());
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
            if self.backend.font_dialogs().len() > self.dialogs {
                self.dialogs = self.backend.font_dialogs().len();
                self.backend.answer_font_dialog(None);
                self.event("font-dialog-cancelled", Json::Null);
            }
            for command in self.run.take_unreviewed_notices() {
                self.event(
                    "unreviewed-allowed",
                    Json::obj([
                        ("command", Json::hex(0x2000 + u64::from(command))),
                        (
                            "semantics",
                            cmvs_core::dispatch::semantics_name(
                                self.run.gate().selection().profile,
                                command,
                            )
                            .into(),
                        ),
                    ]),
                );
            }
            match event {
                StartupEvent::Yield => break,
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::MainEnteredWithoutSnapshot => {
                    self.event("main-without-snapshot", Json::Null);
                }
                StartupEvent::Quit => return Err(self.stop(Stop::Quit)),
                // The event already carries the full 0x2000-based opcode.
                StartupEvent::Unhandled { pc, command } => {
                    let blocked = Blocked {
                        command,
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
        // The recording backend keeps a bounded request log; a fresh one per hundred frames
        // keeps long runs under its limit. Audio state itself lives in the core host.
        if self.yields.is_multiple_of(100) {
            self.run
                .set_audio_backend(Box::new(RecordingAudio::default()))
                .map_err(|e| Stop::Error(e.to_string()))?;
        }
        let advanced = if self.render_frames {
            self.run.advance_decoded_frame(self.frame_ms)
        } else {
            self.run.advance_frame(self.frame_ms)
        };
        advanced.map_err(|e| self.stop(Stop::Error(e.to_string())))?;
        // Match the desktop: update after a VM yield, then publish the complete
        // canvas before the next VM slice can request device readback.
        if self.render_frames {
            let image = self
                .completed_frame()
                .map_err(|e| self.stop(Stop::Error(e)))?;
            self.backend.present_frame(image);
        }
        // Keyboard navigation moves the system cursor to the new button; the host takes it.
        if let Some(warp) = self.run.input_mut().take_warp() {
            self.pointer = warp;
        }
        self.navigate()?;
        let frame = self.queue.pop_front().unwrap_or_default();
        if let Some(pointer) = frame.pointer {
            self.pointer = pointer;
        }
        let mut now = [false; 128];
        for &code in frame.codes.iter().chain(&self.held) {
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

    fn completed_frame(&mut self) -> Result<cmvs_core::raster::RgbaImage, String> {
        if let Some(movie) = self.run.movie_frame() {
            use cmvs_core::platform::PlatformBackend;
            return cmvs_desktop::movie_canvas(
                movie,
                self.run.screen_size(),
                self.backend.render_mode().map_err(|e| e.to_string())? == 1,
            )
            .map_err(|e| e.to_string());
        }
        self.font
            .as_mut()
            .ok_or("software frame rendering requires a font")?
            .render(self.run.scene(), self.run.camera(), self.run.messages())
            .map_err(|e| e.to_string())
    }
    /// One step of a keyboard press: tap "next" until the target is current, then decide.
    fn navigate(&mut self) -> Result<(), Stop> {
        let Some((group, button, taps)) = self.navigation else {
            return Ok(());
        };
        if !self.queue.is_empty() {
            return Ok(());
        }
        let Some(buttons) = crate::controls::group(&self.run, group) else {
            self.navigation = None;
            return Err(self.stop(Stop::Error(format!(
                "keyboard navigation lost button group {group}"
            ))));
        };
        let (current, count) = (buttons.current, buttons.buttons.len());
        let action = if current == Some(button) {
            self.navigation = None;
            crate::device::engine_action::DECIDE
        } else if taps > count + 1 {
            self.navigation = None;
            return Err(self.stop(Stop::Error(format!(
                "keyboard navigation could not reach button {group}:{button}"
            ))));
        } else {
            self.navigation = Some((group, button, taps + 1));
            crate::device::engine_action::NEXT
        };
        let frames = crate::device::key_tap(&self.bindings, action).map_err(Stop::Error)?;
        self.queue.extend(frames);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::{platform::PlatformBackend, runner::load_start};

    #[test]
    #[ignore = "requires CMVS_TEST_FONT; uses only synthetic scripts and archives"]
    fn yielded_frames_enable_device_capture_on_the_next_vm_slice() {
        let font = cmvs_desktop::font::FontRenderer::read(
            Path::new(&std::env::var("CMVS_TEST_FONT").expect("set CMVS_TEST_FONT")),
            0,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!("cmvs-probe-frames-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let game_root = root.join("game");
        std::fs::create_dir_all(game_root.join("data/pack")).unwrap();
        // Yield, create a parent object, capture the previous device frame, yield.
        let mut code = 0x2000u16.to_le_bytes().to_vec();
        for (op, args) in [(0x2020u16, vec![3u32]), (0x203d, vec![3, u32::MAX])] {
            for arg in args.into_iter().rev() {
                code.extend(0x440u16.to_le_bytes());
                code.extend(4u16.to_le_bytes());
                code.extend(arg.to_le_bytes());
            }
            code.extend(op.to_le_bytes());
        }
        code.extend(0x2000u16.to_le_bytes());
        let mut raw = b"PS2A".to_vec();
        for word in [
            48,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        let start = game_root.join(["data", "pack", "start"].join("/") + ".ps3");
        std::fs::write(start, raw).unwrap();
        let archive: Vec<u8> = include_str!("../../../tests/fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        std::fs::write(game_root.join("data/pack/script.cpz"), archive).unwrap();
        let target = Target::open(&game_root, Some("cmvs-20160201"));
        let script = load_start(&game_root, &target.game.as_ref().unwrap().files).unwrap();
        for render_frames in [false, true] {
            let options = Options {
                render_frames,
                capture_snapshots: false,
                ..Options::default()
            };
            let user = root.join(if render_frames {
                "rendered"
            } else {
                "recorded"
            });
            std::fs::create_dir(&user).unwrap();
            let mut session =
                Session::start(&target, &script, &user, &options, Some(&font)).unwrap();
            assert!(session.backend.read_device_frame().is_err());
            session.frame().unwrap();
            if render_frames {
                let previous = session.backend.read_device_frame().unwrap();
                assert_eq!(
                    [previous.width(), previous.height()],
                    session.run.screen_size()
                );
                session.frame().unwrap();
                let captured = session
                    .run
                    .scene()
                    .object(3)
                    .unwrap()
                    .image
                    .as_ref()
                    .unwrap();
                assert_eq!(captured.pixels.pixels(), previous.pixels());
            } else {
                assert!(session.backend.read_device_frame().is_err());
                assert!(
                    session
                        .frame()
                        .unwrap_err()
                        .describe()
                        .contains("no completed frame")
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

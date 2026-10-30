//! Bounded real-data startup orchestration for the researched Amatsutsumi layout.
//! Snapshots remain barriers unless the caller explicitly continues without an image.
//! Every system command first passes the selected engine profile's gate.
use crate::{
    MAX_PAYLOAD,
    audio::{AudioBackend, AudioHost, RecordingAudio},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    camera::CameraHost,
    capture::ScreenCapture,
    fade::SkipGate,
    gesture::PointerGesture,
    history::HistoryHost,
    image::ImageArchives,
    input::{self, ActionInput},
    machine::{Machine, Step},
    message::MessageHost,
    message_flags::MessageFlags,
    movie::{DirectoryMovies, MovieHost},
    platform::{DirectoryAssets, PlatformBackend, PlatformHost},
    profile::{CommandGate, Game, GameFiles},
    ps3::Script,
    random::CrtRandom,
    resources::{ResourceArchives, ResourceKind},
    save_menu::{DirectorySaves, SaveMenuHost},
    scene::SceneHost,
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    sequence::SequenceHost,
    snapshot::{SaveImage, SaveInfo, SaveTitle},
    system_save::{
        FileSystemSave, NewSystemSave, ProfileSystemSave, SystemSaveEvent, SystemSaveHost,
        SystemSaveSink,
    },
    text,
    timers::VmTimers,
};
use std::{
    error::Error,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

mod lifecycle;
mod preferences;
mod save;
pub use save::{SaveTime, thumbnail_bmp};

enum SystemOutput {
    New(NewSystemSave),
    Profile(ProfileSystemSave),
}
impl SystemOutput {
    fn path(&self) -> PathBuf {
        match self {
            Self::New(output) => output.path().to_owned(),
            Self::Profile(profile) => profile.path(),
        }
    }
}
impl SystemSaveSink for SystemOutput {
    fn write(&mut self, raw: &[u8]) -> std::io::Result<()> {
        match self {
            Self::New(output) => output.write(raw),
            Self::Profile(profile) => profile.write(raw),
        }
    }
}

pub fn load_start(root: &Path, files: &GameFiles) -> Result<Script, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(root.join(&files.start_script))?
        .take(MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAYLOAD + 48 {
        return Err("startup script exceeds research limit".into());
    }
    Ok(Script::decode(&bytes)?)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEvent {
    /// 0x2001: playback stopped; the host must close its window and end execution.
    Quit,
    /// Host is unfocused and ACTIVE_DRAW is off; no VM instruction was executed.
    Inactive,
    Progress,
    Yield,
    Unhandled {
        pc: usize,
        command: u16,
    },
    MainPreparedNeedsSnapshot,
    /// Main entry reached; the original snapshot was not built and saving stays unavailable.
    MainEnteredWithoutSnapshot,
    /// 0x2080 or 0x22b2 built the engine snapshot that slot writes use.
    SnapshotCaptured,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SnapshotPolicy {
    /// Build the engine snapshot (0x140069270) at 0x2080 and 0x22b2.
    #[default]
    Capture,
    /// Stop at the first snapshot point (research).
    Barrier,
    /// Apply only the live-state effect and continue without an image (research).
    ContinueWithoutImage,
}
pub struct Startup<'a> {
    quit: bool,
    focused: bool,
    machine: Machine<'a>,
    gate: CommandGate<'a>,
    /// Unreviewed commands first let through since the last `take_unreviewed_notices`.
    notices: Vec<u16>,
    bootstrap: Bootstrap,
    control: ScriptControl,
    source: ArchiveScripts<File>,
    saves: DirectorySaves,
    save_menu: SaveMenuHost,
    system_source: FileSystemSave,
    system_host: SystemSaveHost,
    system_output: Option<SystemOutput>,
    assets: DirectoryAssets,
    platform: PlatformHost,
    scene: SceneHost,
    buttons: ButtonHost,
    movies: MovieHost,
    scene_movies: ResourceArchives,
    root: PathBuf,
    sequence: SequenceHost,
    camera: CameraHost,
    capture: ScreenCapture,
    input: ActionInput,
    effects: bool,
    timers: VmTimers,
    resume_run: bool,
    frame_delta: u32,
    images: ImageArchives,
    save_title: SaveTitle,
    save_info: SaveInfo,
    gesture: PointerGesture,
    message_flags: MessageFlags,
    settings: crate::settings::SettingsHost,
    configuration: crate::settings::Configuration,
    messages: MessageHost,
    history: HistoryHost,
    random: CrtRandom,
    audio: AudioHost,
    audio_backend: Box<dyn AudioBackend>,
    steps: u64,
    /// Steps since the last yield, and the watchdog limit for them.
    since_yield: u64,
    yield_watchdog: u64,
    main_barrier: bool,
    snapshot_policy: SnapshotPolicy,
    save_image: SaveImage,
    /// Recorded (game ID, integer) of the verified preview policy, once enabled.
    skip_auto_resume: Option<(u32, u32)>,
    auto_resume_checked: bool,
    /// Engine +11192: the 0x22b8 thumbnail BMP.
    thumbnail: Option<Vec<u8>>,
    save_clock: Box<dyn FnMut() -> SaveTime>,
    auto_resume_skipped: bool,
}
impl<'a> Startup<'a> {
    pub fn new(
        script: &'a Script,
        root: &Path,
        game: &Game<'a>,
        virtual_tick: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let mut machine = Machine::new(script)?;
        machine.set_current_name(game.files.start_name());
        // 0x14000a3c0 reads the startup settings; cmvs.cfg is optional and read only.
        let configuration = crate::settings::Configuration::read(&root.join("cmvs.cfg"))?
            .unwrap_or(crate::settings::Configuration::new(Vec::new())?);
        let text = configuration.bytes();
        let mut save_menu = SaveMenuHost::default();
        save_menu.set_confirmations(
            crate::save_menu::profile_int(text, "SYSTEM_INFO", "DLG_LOAD", 0) != 0,
            crate::save_menu::profile_int(text, "SYSTEM_INFO", "DLG_SAVE", 0) != 0,
        );
        let audio = AudioHost::from_ini(text);
        let settings = crate::settings::SettingsHost::from_ini(text);
        settings.check_font_face()?;
        let message_flags = MessageFlags::from_ini(text);
        let effects = crate::save_menu::profile_int(text, "SYSTEM_INFO", "EFFECT_USE", 1) != 0;
        Ok(Self {
            quit: false,
            machine,
            gate: CommandGate::new(game.selection.clone()),
            notices: Vec::new(),
            configuration,
            focused: true,
            bootstrap: Bootstrap::default(),
            control: ScriptControl::new(virtual_tick),
            source: ArchiveScripts::new(File::open(root.join(&game.files.script_archive))?)?,
            saves: DirectorySaves::new(root.join("save"))?,
            save_menu,
            system_source: FileSystemSave::new(root.join("save/system.dat")),
            system_host: SystemSaveHost::default(),
            system_output: None,
            assets: DirectoryAssets::new(root)?,
            platform: PlatformHost::default(),
            scene: SceneHost::default(),
            buttons: ButtonHost::default(),
            movies: MovieHost::default(),
            scene_movies: ResourceArchives::new(root)?,
            root: root.canonicalize()?,
            sequence: SequenceHost::default(),
            camera: CameraHost::default(),
            capture: ScreenCapture::default(),
            // No verified key bindings yet: no action is active (see `input`).
            input: ActionInput::default(),
            effects,
            timers: VmTimers::default(),
            resume_run: true,
            frame_delta: 0,
            images: ImageArchives::new(root)?,
            save_title: SaveTitle::default(),
            save_info: SaveInfo::default(),
            gesture: PointerGesture::default(),
            message_flags,
            settings,
            messages: MessageHost::default(),
            history: HistoryHost::default(),
            // Explicit research seed, like the virtual tick; the original seeds from timeGetTime.
            random: CrtRandom::new(0),
            audio,
            audio_backend: Box::new(RecordingAudio::default()),
            steps: 0,
            since_yield: 0,
            yield_watchdog: 1_000_000,
            main_barrier: false,
            snapshot_policy: SnapshotPolicy::default(),
            save_image: SaveImage::default(),
            skip_auto_resume: None,
            auto_resume_checked: false,
            thumbnail: None,
            // A fixed research time keeps runs repeatable; hosts supply a wall clock.
            save_clock: Box::new(|| [2000, 1, 1, 0, 0, 0]),
            auto_resume_skipped: false,
        })
    }
    pub fn machine(&self) -> &Machine<'a> {
        &self.machine
    }
    /// Explicit preview policy, not a VM command: after the first system-save read, clear
    /// the recorded integer (2198 for Amatsutsumi) so main.ps3 does not attempt startup
    /// auto-load. Other persistent values and every file stay unchanged. Configure before
    /// ticking; only games whose record verifies the policy accept it.
    pub fn set_skip_auto_resume(&mut self, enabled: bool) -> Result<(), &'static str> {
        if self.steps != 0 {
            return Err("skip-auto-resume must be configured before startup");
        }
        self.skip_auto_resume = match (enabled, self.gate.selection().record) {
            (false, _) => None,
            (true, Some(record)) => Some((
                record.game_id,
                record
                    .skip_auto_resume
                    .ok_or("skip-auto-resume is not verified for this game")?,
            )),
            (true, None) => return Err("skip-auto-resume needs a recorded game"),
        };
        Ok(())
    }
    /// Research only: let unreviewed commands run with the base profile's semantics.
    pub fn set_allow_unreviewed(&mut self, allow: bool) {
        self.gate.set_allow_unreviewed(allow);
    }
    pub fn gate(&self) -> &CommandGate<'a> {
        &self.gate
    }
    /// Unreviewed commands let through for the first time since the previous call.
    pub fn take_unreviewed_notices(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.notices)
    }
    pub fn auto_resume_skipped(&self) -> bool {
        self.auto_resume_skipped
    }
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }
    pub fn control(&self) -> &ScriptControl {
        &self.control
    }
    pub fn platform(&self) -> &PlatformHost {
        &self.platform
    }
    pub fn scene(&self) -> &SceneHost {
        &self.scene
    }
    pub fn camera(&self) -> &CameraHost {
        &self.camera
    }
    pub fn sequence(&self) -> &SequenceHost {
        &self.sequence
    }
    pub fn movies(&self) -> &MovieHost {
        &self.movies
    }
    pub fn movie_frame(&self) -> Option<&crate::raster::RgbaImage> {
        self.movies.player().and_then(|m| m.image.as_ref())
    }
    pub fn set_decode_movie_frames(&mut self, enabled: bool) {
        self.movies.set_decode_frames(enabled);
    }
    pub fn render_frame(&self) -> Result<crate::raster::RgbaImage, crate::FormatError> {
        match self.movie_frame() {
            Some(image) => Ok(image.clone()),
            None => {
                crate::scene_render::render_scene(&self.scene, Some(&self.camera), &[], 1280, 720)
            }
        }
    }
    pub fn advance_decoded_frame(&mut self, delta_ms: u32) -> Result<(), Box<dyn Error>> {
        if self.quit {
            return Ok(());
        }
        self.audio_backend.update()?;
        let clock = self.audio_backend.movie_clock()?;
        let had_movie = self.movies.player().is_some();
        self.movies.advance_with_clock(
            delta_ms,
            clock,
            &mut DirectoryMovies {
                root: &self.root,
                directory: self.bootstrap.paths().get(&(0x2015, 0)).map(Vec::as_slice),
            },
        )?;
        if had_movie && self.movies.player().is_none() && clock.is_some() {
            self.audio_backend.stop_movie_audio()?;
        }
        self.movies.advance_scene(delta_ms, &mut self.scene)?;
        if !self.frame_running() {
            self.frame_delta = 0;
            return Ok(());
        }
        self.sequence.advance_frame_with_images(
            delta_ms,
            &mut self.scene,
            &mut self.images.with_paths(self.bootstrap.paths()),
        )?;
        self.scene.advance_frame(delta_ms);
        self.messages.advance_frame(delta_ms);
        self.messages
            .advance_auxiliary_motion(delta_ms, &mut self.scene)?;
        self.camera.advance_frame(delta_ms);
        self.frame_delta = delta_ms;
        self.take_capture()
    }
    /// The main loop 0x14000e400 draws after the updates (0x140015900, then 0x140009c90);
    /// the draw 0x140015fc0 ends with 0x1400157b0, which serves a pending 0x2304 request.
    /// The core draws the scene and visible message windows on the device-sized research
    /// viewport. Glyphs belong to the backend and full-screen movies to their player, so
    /// a capture over either fails explicitly.
    fn take_capture(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.capture.requested() {
            return Ok(());
        }
        if self.movies.player().is_some() {
            return Err("screen capture during a full-screen movie is not verified".into());
        }
        let texts = (0..crate::message::TEXT_OBJECTS).filter_map(|i| self.messages.text(i));
        if texts
            .filter(|text| text.visible)
            .any(|text| text.glyphs.iter().any(|g| g.shown && g.opacity != 0))
        {
            return Err("screen capture over visible glyphs needs the font backend".into());
        }
        if (0..crate::message::AUXILIARY_TEXTS)
            .filter_map(|i| self.messages.auxiliary_text(i))
            .any(|group| group.attached_glyphs().next().is_some())
        {
            return Err("screen capture over auxiliary glyphs needs the font backend".into());
        }
        let windows: Vec<_> = (0..crate::message::MESSAGE_WINDOWS)
            .filter_map(|i| self.messages.window(i))
            .filter(|window| window.visible && !window.hidden)
            .map(|window| &window.object)
            .collect();
        let [width, height] = crate::message::RESEARCH_SCREEN;
        let frame = crate::scene_render::render_scene(
            &self.scene,
            Some(&self.camera),
            &windows,
            width,
            height,
        )?;
        self.capture.take(frame)?;
        Ok(())
    }
    pub fn capture(&self) -> &ScreenCapture {
        &self.capture
    }
    pub fn buttons(&self) -> &ButtonHost {
        &self.buttons
    }
    pub fn input(&self) -> &ActionInput {
        &self.input
    }
    /// Host input is refreshed once at a frame boundary, and remains consumable across
    /// cooperative budget slices. Core does not interpret physical window events.
    pub fn input_mut(&mut self) -> &mut ActionInput {
        &mut self.input
    }
    /// Update the scene and supply the latest delta for the next VM run entry.
    /// The timer delta is consumed once after yield, never per instruction/budget slice.
    /// Callback registration time remains a separate research clock.
    pub fn advance_frame(&mut self, delta_ms: u32) -> Result<(), Box<dyn Error>> {
        if self.quit {
            return Ok(());
        }
        self.movies.advance_scene(delta_ms, &mut self.scene)?;
        self.movies.advance(delta_ms);
        if !self.frame_running() {
            self.frame_delta = 0;
            return Ok(());
        }
        self.sequence.advance_frame_with_images(
            delta_ms,
            &mut self.scene,
            &mut self.images.with_paths(self.bootstrap.paths()),
        )?;
        self.scene.advance_frame(delta_ms);
        self.messages.advance_frame(delta_ms);
        self.messages
            .advance_auxiliary_motion(delta_ms, &mut self.scene)?;
        self.camera.advance_frame(delta_ms);
        self.frame_delta = delta_ms;
        self.take_capture()
    }
    pub fn timers(&self) -> &VmTimers {
        &self.timers
    }
    pub fn save_title(&self) -> &SaveTitle {
        &self.save_title
    }
    pub fn save_info(&self) -> SaveInfo {
        self.save_info
    }
    pub fn gesture(&self) -> &PointerGesture {
        &self.gesture
    }
    pub fn message_flags(&self) -> MessageFlags {
        self.message_flags
    }
    pub fn set_focused(&mut self, focused: bool) {
        let was_running = self.frame_running();
        self.focused = focused;
        if was_running != self.frame_running() {
            self.frame_delta = 0;
        }
    }
    pub fn frame_running(&self) -> bool {
        !self.quit && (self.focused || self.settings.active_draw())
    }
    pub fn settings(&self) -> &crate::settings::SettingsHost {
        &self.settings
    }
    /// Startup check 0x14000b666: the host replaces a configured face it cannot find
    /// with the built-in default before any font object exists.
    pub fn set_font_face(&mut self, face: &[u8]) -> Result<(), Box<dyn Error>> {
        self.settings.set_font_face(face)?;
        self.messages.set_default_face(face);
        Ok(())
    }
    pub fn messages(&self) -> &MessageHost {
        &self.messages
    }
    pub fn history(&self) -> &HistoryHost {
        &self.history
    }
    /// Explicit single-line message recording; no font pixels are generated.
    pub fn set_record_text(&mut self, enabled: bool) {
        self.messages.set_record_output(enabled);
    }
    pub fn set_glyph_metrics(&mut self, metrics: Box<dyn crate::message::GlyphMetrics>) {
        self.messages.set_glyph_metrics(metrics);
    }
    pub fn save_menu(&self) -> &SaveMenuHost {
        &self.save_menu
    }
    pub fn steps(&self) -> u64 {
        self.steps
    }
    pub fn save_image(&self) -> &SaveImage {
        &self.save_image
    }
    pub fn audio(&self) -> &AudioHost {
        &self.audio
    }
    /// Defaults to a recording backend that reports nothing started.
    pub fn set_audio_backend(
        &mut self,
        mut backend: Box<dyn AudioBackend>,
    ) -> Result<(), Box<dyn Error>> {
        backend.configure_voice_ducking(self.audio.voice_ducking())?;
        self.audio_backend = backend;
        Ok(())
    }
    pub fn set_random_seed(&mut self, seed: u32) {
        self.random = CrtRandom::new(seed);
    }
    /// Opt into one CSS1 export to a new file outside the original game directory.
    /// Subsequent 0x2128 reads use the exported file after the write succeeds.
    pub fn set_system_save_output(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        if self.steps != 0 || self.system_output.is_some() {
            return Err("select one system-save output mode before VM execution".into());
        }
        self.system_output = Some(SystemOutput::New(NewSystemSave::new(path, &self.root)?));
        Ok(())
    }
    /// Use a locked, independent user directory for CSS1 reads, repeated writes and
    /// gameplay-slot scanning. Only an absent initial CSS1 falls back to the game template.
    pub fn set_user_data_directory(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        if self.steps != 0 || self.system_output.is_some() {
            return Err("select one system-save output mode before VM execution".into());
        }
        let profile = ProfileSystemSave::open(path, &self.root)?;
        let saves = DirectorySaves::new(profile.directory())?;
        let configuration = profile.read_configuration()?;
        let primary = profile.path();
        let source = if primary.try_exists()? {
            primary
        } else {
            self.root.join("save/system.dat")
        };
        if let Some(configuration) = configuration {
            self.install_configuration(configuration)?;
        }
        self.system_source = FileSystemSave::new(source);
        self.saves = saves;
        self.system_output = Some(SystemOutput::Profile(profile));
        Ok(())
    }
    /// Host watchdog, not an engine limit: a tick fails without stepping once this many VM
    /// steps ran since the last yield (default 1,000,000; the longest stretch measured on
    /// the replayed path from startup into the first scenario is 9970). Total steps are
    /// unbounded; hosts bound long runs themselves.
    pub fn set_yield_watchdog(&mut self, steps: u64) {
        self.yield_watchdog = steps;
    }
    pub fn set_snapshot_policy(&mut self, policy: SnapshotPolicy) {
        self.snapshot_policy = policy;
    }
    pub fn tick(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        if self.quit {
            return Ok(StartupEvent::Quit);
        }
        if !self.frame_running() {
            return Ok(StartupEvent::Inactive);
        }
        if self.since_yield >= self.yield_watchdog {
            return Err(format!(
                "research watchdog: {} VM steps without a yield",
                self.since_yield
            )
            .into());
        }
        let before = self.steps;
        let event = self.step_once(backend)?;
        if self.steps != before {
            if event == StartupEvent::Yield {
                self.since_yield = 0;
            } else {
                self.since_yield += 1;
            }
        }
        Ok(event)
    }
    /// 0x213d / 0x140081520: the modal font dialog starts from the global face (+3160).
    /// On OK the choice becomes global (0x140007f20) and the twelve text objects that
    /// request the default face reselect it (0x14005ab40). Returns 0x4000 either way.
    fn choose_font(&mut self, backend: &mut impl PlatformBackend) -> Result<(), Box<dyn Error>> {
        let (pc, _) = self.machine.pending_system().expect("checked by caller");
        let chosen = backend.choose_font(self.settings.font_face())?;
        let mut settings = self.settings.clone();
        if let Some(face) = &chosen {
            settings
                .set_font_face(face)
                .map_err(|why| crate::evaluate::error(pc, 0x213d, why))?;
        }
        self.machine
            .complete_system(crate::vm::SystemResult(0x4000))?;
        if chosen.is_some() {
            self.settings = settings;
            self.messages.set_default_face(self.settings.font_face());
            self.messages.refresh_default_fonts();
        }
        Ok(())
    }
    fn step_once(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        self.messages.set_text_settings(self.settings.text_output());
        self.messages.set_default_face(self.settings.font_face());
        if self.main_barrier {
            return Ok(StartupEvent::MainPreparedNeedsSnapshot);
        }
        if self.resume_run {
            // 0x140019430: the gesture update follows the input refresh and precedes the VM.
            let delta = std::mem::take(&mut self.frame_delta);
            self.timers.begin_run(delta);
            self.input.latch_pointer();
            self.gesture.frame(&mut self.input, self.timers.clock())?;
            // 0x140067930 runs the automatic callback dispatch 0x140064f50 after the timers.
            let shown =
                std::array::from_fn(|slot| self.messages.window(slot).is_some_and(|w| w.visible));
            let due = self.control.due_callbacks(shown);
            self.machine.dispatch_callbacks(&due)?;
            self.resume_run = false;
        }
        let event = self.machine.step()?;
        self.steps += 1;
        let Step::System { pc, command } = event else {
            self.resume_run = event == Step::Yield;
            return Ok(if event == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        };
        if let Some(first) = self.gate.check(&self.machine, pc, command)? {
            self.notices.push(first);
        }
        if let Some(event) = self.try_lifecycle()? {
            return Ok(event);
        }
        let id = self.bootstrap.settings().get(&0x201d).copied();
        if command == 0x129
            && let Some(output) = &mut self.system_output
            && self
                .system_host
                .try_write(&mut self.machine, output, &mut self.random, id)?
        {
            self.system_source = FileSystemSave::new(output.path());
            return Ok(StartupEvent::Progress);
        }
        if matches!(command, 0xa0 | 0xa4 | 0xa6 | 0xb0 | 0xba | 0x2d6 | 0x2d7) {
            self.audio_backend.configure_paths(self.bootstrap.paths())?;
        }
        let gate = SkipGate {
            effects: self.effects,
            skip: self.message_flags.skip(),
        };
        if let Some(step) =
            self.scene
                .fade_mut()
                .try_wait(&mut self.machine, gate, &mut self.input)?
        {
            self.resume_run = step == Step::Yield;
            return Ok(if step == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        }
        let waited = match self
            .scene
            .try_wait_mask(&mut self.machine, gate, &mut self.input)?
        {
            Some(step) => Some(step),
            None => self.scene.try_wait_quake(&mut self.machine)?,
        };
        let waited = match waited {
            Some(step) => Some(step),
            None => self
                .scene
                .try_wait_change(&mut self.machine, gate, &mut self.input)?,
        };
        let waited = match waited {
            Some(step) => Some(step),
            None => self
                .scene
                .try_wait_strip(&mut self.machine, gate, &mut self.input)?,
        };
        let waited = match waited {
            Some(step) => Some(step),
            None => self
                .camera
                .try_wait_move(&mut self.machine, gate, &mut self.input)?,
        };
        if let Some(step) = waited {
            self.resume_run = step == Step::Yield;
            return Ok(if step == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        }
        let (message_flags, read_eligible) = if matches!(command, 0x152 | 0x153) {
            let read_id = self
                .machine
                .stack()
                .peek_u32(8)
                .map_err(|e| crate::evaluate::error(pc, command + 0x2000, e.0))?
                as i32;
            self.message_flags
                .prepare_read(self.machine.state(), read_id, self.settings.read_skip())
                .map_err(|why| crate::evaluate::error(pc, command + 0x2000, why))?
        } else {
            (self.message_flags, true)
        };
        if let Some(step) = self.messages.try_wait_with_auto(
            &mut self.machine,
            &mut self.input,
            message_flags,
            crate::message::AutoAdvanceContext {
                settings: self.settings.auto_advance(),
                delta_ms: self.timers.delta_ms(),
                gesture_result: self
                    .gesture
                    .field(88)
                    .is_some_and(|value| value as i32 >= 0),
                read_eligible,
                audio: self.audio_backend.as_ref(),
            },
        )? {
            self.message_flags = message_flags;
            self.resume_run = step == Step::Yield;
            return Ok(if step == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        }
        if let Some(step) =
            self.messages
                .try_record_output(&mut self.machine, &mut self.input, message_flags)?
        {
            self.message_flags = message_flags;
            self.resume_run = step == Step::Yield;
            return Ok(StartupEvent::Yield);
        }
        if command == 0x128
            && let Some((game_id, _)) = self.skip_auto_resume
            && !self.auto_resume_checked
            && id != Some(game_id)
        {
            return Err(format!(
                "skip-auto-resume is only verified for recorded game ID 0x{game_id:08x}"
            )
            .into());
        }
        if let Some(event) =
            self.system_host
                .try_handle(&mut self.machine, &mut self.system_source, id)?
        {
            if !self.auto_resume_checked {
                if let Some((_, integer)) = self.skip_auto_resume
                    && event == SystemSaveEvent::Applied
                {
                    self.machine.state_mut().set_integer(integer, 0)?;
                    self.auto_resume_skipped = true;
                }
                self.auto_resume_checked = true;
            }
            return Ok(StartupEvent::Progress);
        }
        if matches!(self.machine.pending_system(), Some((_, 0x13d))) {
            self.choose_font(backend)?;
            return Ok(StartupEvent::Progress);
        }
        if let Some(event) = self.platform.try_handle(
            &mut self.machine,
            backend,
            &mut self.assets,
            self.bootstrap.cursor_names(),
        )? {
            let yields = event == crate::platform::PlatformEvent::Yield;
            self.resume_run = yields;
            return Ok(if yields {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        }
        if let Some(event) = self
            .control
            .try_handle(&mut self.machine, &mut self.source)?
        {
            if event == ControlEvent::MainPreparedNeedsSnapshot {
                match self.snapshot_policy {
                    SnapshotPolicy::Barrier => {
                        self.main_barrier = true;
                        return Ok(StartupEvent::MainPreparedNeedsSnapshot);
                    }
                    SnapshotPolicy::ContinueWithoutImage => {
                        self.save_image.capture_unavailable(&mut self.machine)?;
                        return Ok(StartupEvent::MainEnteredWithoutSnapshot);
                    }
                    SnapshotPolicy::Capture => {
                        // Main is already loaded; a failed capture keeps the barrier sticky
                        // instead of running main without an image.
                        let captured = match self.machine.capture_vm() {
                            Ok(vm) => self.capture_snapshot(vm),
                            Err(why) => Err(why.into()),
                        };
                        if let Err(why) = captured {
                            self.main_barrier = true;
                            return Err(why);
                        }
                        return Ok(StartupEvent::SnapshotCaptured);
                    }
                }
            }
            return Ok(StartupEvent::Progress);
        }
        if self.snapshot_policy == SnapshotPolicy::Capture
            && let Some((pc, 0x2b2)) = self.machine.pending_system()
        {
            // 0x14007c9a0 serializes with PC + 2 and repeat 0, which completing the
            // no-argument command then produces; a failed capture keeps it pending.
            let mut vm = self
                .machine
                .capture_vm()
                .map_err(|why| crate::evaluate::error(pc, 0x22b2, why))?;
            vm.pc = pc as u32 + 2;
            vm.repeat_count = 0;
            self.capture_snapshot(vm)?;
            self.machine
                .complete_system(crate::vm::SystemResult(0x4000))?;
            return Ok(StartupEvent::SnapshotCaptured);
        }
        if self.try_load_slot(backend)? {
            return Ok(StartupEvent::Progress);
        }
        if self.try_save_slot()?
            || self.try_thumbnail()?
            || self.try_latest_slot()?
            || self.try_slot_info()?
        {
            return Ok(StartupEvent::Progress);
        }
        if self
            .save_menu
            .try_handle(&mut self.machine, &mut self.saves)?
            || self.save_image.try_handle(
                &mut self.machine,
                self.snapshot_policy == SnapshotPolicy::ContinueWithoutImage,
            )?
            || self.timers.try_handle(&mut self.machine)?
            || self.bootstrap.try_handle(&mut self.machine)?.is_some()
            || self.scene.try_handle(&mut self.machine)?
            || self.scene.try_handle_quake(&mut self.machine)?
            || self.scene.try_handle_strip(&mut self.machine)?
            || self.scene.try_handle_image(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.scene.try_change_image(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.scene.try_handle_mask(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.save_title.try_handle(&mut self.machine)?
            || self.save_info.try_handle(&mut self.machine)?
            || self.gesture.try_handle_drag_press(
                &mut self.machine,
                &self.input,
                self.timers.clock(),
            )?
            || self.gesture.try_handle(&mut self.machine)?
            || self.save_menu.try_handle_popup(&mut self.machine)?
            || self.message_flags.try_handle(&mut self.machine)?
            || self.settings.try_handle(&mut self.machine)?
            || self
                .messages
                .try_record_poll(&mut self.machine, self.message_flags)?
            || self
                .messages
                .try_handle_scene(&mut self.machine, &mut self.scene)?
            || self.messages.try_handle(&mut self.machine)?
            || self.history.try_handle(&mut self.machine)?
            || self
                .messages
                .try_handle_pointer(&mut self.machine, &self.input)?
            || self
                .messages
                .try_handle_buttons(&mut self.machine, &mut self.input)?
            || self.messages.try_handle_image(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self
                .buttons
                .try_handle(&mut self.machine, &mut self.scene)?
            || self.buttons.try_handle_list_input(
                &mut self.machine,
                &mut self.scene,
                &mut self.input,
                &self.gesture,
            )?
            || self
                .buttons
                .try_handle_input(&mut self.machine, &mut self.scene, &mut self.input)?
            || self.movies.try_create_scene(
                &mut self.machine,
                &mut self.scene,
                &mut self
                    .scene_movies
                    .with_paths(self.bootstrap.paths(), ResourceKind::SceneMovie),
            )?
            || self.movies.try_start_scene(&mut self.machine)?
            || self.movies.try_handle(&mut self.machine)?
            || self.movies.try_handle_player(
                &mut self.machine,
                &mut DirectoryMovies {
                    root: &self.root,
                    directory: self.bootstrap.paths().get(&(0x2015, 0)).map(Vec::as_slice),
                },
                self.audio.settings(),
                self.audio_backend.as_mut(),
            )?
            || input::try_handle(&mut self.machine, &mut self.input)?
            || crate::fade::try_handle_effects(&mut self.machine, &mut self.effects)?
            || input::try_handle_skipping(
                &mut self.machine,
                &self.input,
                self.message_flags.skip(),
            )?
            || self.sequence.try_handle_with_images(
                &mut self.machine,
                &mut self.scene,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.camera.try_handle(&mut self.machine)?
            || self
                .capture
                .try_handle(&mut self.machine, &mut self.scene)?
            || self.random.try_handle(&mut self.machine)?
            || crate::math::try_handle(&mut self.machine)?
            || text::try_handle(&mut self.machine)?
            || crate::effect_objects::try_handle(&mut self.machine)?
            || self
                .audio
                .try_handle(&mut self.machine, self.audio_backend.as_mut())?
        {
            return Ok(StartupEvent::Progress);
        }
        Ok(StartupEvent::Unhandled {
            pc,
            command: command + 0x2000,
        })
    }
}

//! Game startup and the host frame loop.
//!
//! [`Startup`] connects the VM to resources, scene state, input, audio, and saves.
//! Keep the decoded startup script alive for the session and supply a
//! [`PlatformBackend`] for window and dialog services. Every system command passes
//! the selected profile's gate before dispatch.
//!
//! # Create a session
//!
//! ```no_run
//! use std::path::Path;
//! use cmvs_core::{profile::Registry, runner::{load_start, Startup}};
//!
//! let root = Path::new("game");
//! let registry = Registry::builtin();
//! let game = registry.identify_game(root, None)?;
//! let script = load_start(root, &game.files)?;
//! let mut session = Startup::new(&script, root, &game, 0)?;
//! session.set_user_data_directory(Path::new("user-data"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Drive the host loop
//!
//! 1. Refresh [`Startup::input_mut`] once per frame.
//! 2. Call [`Startup::tick`] until [`StartupEvent::Yield`] or a stop event. If the
//!    instruction budget runs out first, resume later with the same input; do not
//!    advance frame time or sample new input yet.
//! 3. On yield, call [`Startup::advance_decoded_frame`] to advance movies and scene
//!    state. Enable CMV decoding with [`Startup::set_decode_movie_frames`] if needed.
//!    [`Startup::advance_frame`] advances timing without reading full-screen CMV frames.
//! 4. Compose scene, message, and glyph objects for presentation. If host capture is
//!    enabled, service [`Startup::capture_due`] with [`Startup::supply_capture`].
//!
//! [`Startup::render_frame`] returns a movie frame or renders the scene without text.
//! Configure font metrics, audio output, and MGV decoding before commands need them.
//! The default audio backend records requests without playing them.
//!
//! # Stops and errors
//!
//! Stop on [`StartupEvent::Unhandled`] rather than retrying the same unsupported
//! command. [`StartupEvent::Quit`] ends the session. On [`StartupEvent::Inactive`],
//! wait for host events and service audio with [`Startup::poll_audio_output`].
//! [`SnapshotPolicy`] controls gameplay snapshots. The default captures state and
//! returns an error if the state includes unsupported records.
//! Errors from VM execution, resources, and host services propagate to the caller.
use crate::filesystem as fs;
use crate::filesystem::File;
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
    input::ActionInput,
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
    timers::VmTimers,
};
use std::{
    error::Error,
    io::Read,
    path::{Path, PathBuf},
};

mod lifecycle;
mod preferences;
mod save;
mod slot_edit;
pub use save::{
    SaveTime, save_time_from_unix, snapshot_layout_for, snapshot_layout_supported, thumbnail_bmp,
    utc_save_time,
};

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

/// Read and decode the startup script selected by [`GameFiles`].
///
/// `root` is the game directory. Returns an error for I/O failures, oversized input,
/// or malformed script data; no script instructions are executed.
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
/// Result of one [`Startup::tick`], used to drive the host loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEvent {
    /// The session has ended; stop executing its VM.
    Quit,
    /// Host is unfocused and ACTIVE_DRAW is off; no VM instruction was executed.
    Inactive,
    /// Execution advanced; the host may tick again within its instruction budget.
    Progress,
    /// The VM yielded; the host may process input, advance time, and present a frame.
    Yield,
    /// A system command has no handler and remains pending at `pc`.
    Unhandled {
        /// Byte offset in the active script's code segment.
        pc: usize,
        /// System command identifier after any profile-specific translation.
        command: u16,
    },
    /// The main script reached a snapshot barrier; this session cannot continue past it.
    MainPreparedNeedsSnapshot,
    /// Main entry reached; the original snapshot was not built and saving stays unavailable.
    MainEnteredWithoutSnapshot,
    /// The engine snapshot used by gameplay-slot writes was captured.
    SnapshotCaptured,
}
/// How [`Startup`] handles script requests to capture gameplay state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SnapshotPolicy {
    /// Capture the supported engine state, returning an error for unsupported records.
    #[default]
    Capture,
    /// Stop at the first snapshot point (research).
    Barrier,
    /// Apply only the live-state effect and continue without an image (research).
    ContinueWithoutImage,
}
/// A running game session with VM, engine state, and resource services.
///
/// Borrows the startup [`Script`] for the session lifetime. Construction opens
/// resources and configuration; execution begins with [`Self::tick`]. See the
/// [module guide](self) for initialization and frame-loop order.
pub struct Startup<'a> {
    slot_edit: Option<slot_edit::SlotEdit>,
    main_script_name: &'static [u8],
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
    /// The host supplies requested screen captures from the frame it drew.
    host_capture: bool,
    /// A requested capture awaits the host's frame; the next VM step draws it otherwise.
    capture_due: bool,
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
    /// Create a session using Shift-JIS text and the identified game's profile.
    ///
    /// `root` is the game directory and `virtual_tick` initializes the callback clock.
    /// Uses recording audio and [`SnapshotPolicy::Capture`]. Configure output services
    /// and a writable save profile before ticking. Returns errors from script
    /// validation, configuration, or resource initialization.
    pub fn new(
        script: &'a Script,
        root: &Path,
        game: &Game<'a>,
        virtual_tick: u32,
    ) -> Result<Self, Box<dyn Error>> {
        Self::new_with_encoding(
            script,
            root,
            game,
            virtual_tick,
            crate::TextEncoding::default(),
        )
    }
    /// Create a session with an explicit text encoding; otherwise like [`Self::new`].
    pub fn new_with_encoding(
        script: &'a Script,
        root: &Path,
        game: &Game<'a>,
        virtual_tick: u32,
        encoding: crate::TextEncoding,
    ) -> Result<Self, Box<dyn Error>> {
        let mut machine = Machine::new_with_encoding(script, encoding)?;
        machine.set_command_map(game.selection.profile.command_map());
        machine
            .state_mut()
            .set_string_slots(game.selection.profile.string_slots());
        if let Some([integers, bits, floats, strings]) = game.selection.profile.initial_globals() {
            // Verified PE zero-fill globals (docs/games/aoitori/vm-semantics.md
            // and docs/games/hapymaherfd/README.md). Keep standalone
            // research VMs and other profiles unknown until separately verified.
            let state = machine.state_mut();
            state
                .fill_integers(0, integers, 0)
                .map_err(crate::FormatError)?;
            state
                .fill_bits(0, bits, false)
                .map_err(crate::FormatError)?;
            state
                .fill_floats(0, floats, 0)
                .map_err(crate::FormatError)?;
            state
                .fill_strings(0, strings, b"")
                .map_err(crate::FormatError)?;
        }
        machine.set_current_name(game.files.start_name());
        // 0x14000a3c0 reads the startup settings; cmvs.cfg is optional and read only.
        let configuration = crate::settings::Configuration::read(&root.join("cmvs.cfg"))?
            .unwrap_or(crate::settings::Configuration::new(Vec::new())?);
        let screen_size = configuration.screen_size_for_profile(game.selection.profile)?;
        let mut scene = SceneHost::for_profile(game.selection.profile);
        scene.set_screen_size(screen_size);
        let mut buttons = ButtonHost::for_profile(game.selection.profile);
        buttons.set_screen_size(screen_size);
        let mut camera = CameraHost::for_profile(game.selection.profile);
        camera.set_screen_size(screen_size);
        let mut messages = MessageHost::for_profile(game.selection.profile);
        messages.set_screen_size(screen_size);
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
        let mut platform = PlatformHost::default();
        platform.configure_display_selection(text);
        if game.selection.profile.is_inspected_20070119() {
            platform.configure_window_full_move(text);
        }
        Ok(Self {
            slot_edit: None,
            main_script_name: match game.files.start_name().as_slice() {
                b"start.ps2" => b"main.ps2",
                b"start.pss" => b"main.pss",
                _ => b"main.ps3",
            },
            quit: false,
            machine,
            gate: CommandGate::new(game.selection.clone()),
            notices: Vec::new(),
            configuration,
            focused: true,
            bootstrap: Bootstrap::default(),
            control: ScriptControl::new(virtual_tick),
            source: ArchiveScripts::from_start(
                File::open(root.join(&game.files.script_archive))?,
                script,
                game.files
                    .script_archive
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or(crate::FormatError("archive basename is not UTF-8"))?
                    .as_bytes(),
            )?,
            saves: DirectorySaves::new(root.join("save"))?,
            save_menu,
            // CVNS keeps CSS0 in the save directory the start script registers (0x2026).
            system_source: if game.selection.profile.family == crate::profile::Family::Cvns {
                FileSystemSave::css0(root.join("save/save000.dat"))
            } else {
                FileSystemSave::new(root.join("save/system.dat"))
            },
            system_host: SystemSaveHost::default(),
            system_output: None,
            assets: DirectoryAssets::new(root)?,
            platform,
            scene,
            buttons,
            movies: MovieHost::for_profile(game.selection.profile),
            scene_movies: ResourceArchives::new(root)?,
            root: fs::canonicalize(root)?,
            sequence: SequenceHost::default(),
            camera,
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
            messages,
            history: HistoryHost::default(),
            // Explicit research seed, like the virtual tick; the original seeds from timeGetTime.
            random: CrtRandom::new(0),
            audio,
            audio_backend: Box::new(RecordingAudio::default()),
            steps: 0,
            since_yield: 0,
            yield_watchdog: 1_000_000,
            host_capture: false,
            capture_due: false,
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
    /// Inspect VM state and the current instruction position.
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
    /// Block unreviewed commands before entering the selected dispatch handler.
    pub fn set_block_unreviewed(&mut self, block: bool) {
        self.gate.set_block_unreviewed(block);
    }
    /// Choose whether commands supported only by legacy presumed evidence are blocked.
    pub fn set_block_presumed(&mut self, block: bool) {
        self.gate.set_block_presumed(block);
    }
    /// Inspect the selected profile, command policy, and dispatch counts.
    pub fn gate(&self) -> &CommandGate<'a> {
        &self.gate
    }
    /// Unreviewed commands let through for the first time since the previous call.
    pub fn take_unreviewed_notices(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.notices)
    }
    /// Whether the configured startup auto-load skip has been applied.
    pub fn auto_resume_skipped(&self) -> bool {
        self.auto_resume_skipped
    }
    /// Startup settings and registered resource paths.
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }
    /// Request the script's window-close callback; return `false` if none is enabled.
    pub fn request_close(&mut self) -> bool {
        self.control.request_close()
    }
    /// Inspect script slots and callback scheduling state.
    pub fn control(&self) -> &ScriptControl {
        &self.control
    }
    /// Inspect platform-command state recorded by the session.
    pub fn platform(&self) -> &PlatformHost {
        &self.platform
    }
    /// Scene objects and effects to be consumed by the renderer.
    pub fn scene(&self) -> &SceneHost {
        &self.scene
    }
    /// Camera state used for projection and scene rendering.
    pub fn camera(&self) -> &CameraHost {
        &self.camera
    }
    /// Current timed action-sequence state.
    pub fn sequence(&self) -> &SequenceHost {
        &self.sequence
    }
    /// Movie player and scene-movie state.
    pub fn movies(&self) -> &MovieHost {
        &self.movies
    }
    /// Latest decoded full-screen movie frame, if one is available.
    pub fn movie_frame(&self) -> Option<&crate::raster::RgbaImage> {
        self.movies.player_image()
    }
    /// Install the host-provided Theora decoder factory for MGV movies.
    pub fn set_mgv_factory(&mut self, factory: Box<dyn crate::mgv::DecoderFactory>) {
        self.movies.set_mgv_factory(factory);
    }
    /// Enable scene-movie prefetching on a worker thread.
    ///
    /// See [`MovieHost::set_scene_prefetch`] for the thread-start callback contract.
    pub fn set_scene_prefetch(&mut self, on_thread_start: fn()) {
        self.movies.set_scene_prefetch(on_thread_start);
    }
    /// Set the decoded-image cache budget in bytes; see [`ImageArchives::set_decoded_cache_limit`].
    pub fn set_decoded_image_cache(&mut self, bytes: usize) {
        self.images.set_decoded_cache_limit(bytes);
    }
    /// Configure extra image-decoding threads; see [`ImageArchives::set_decode_threads`].
    pub fn set_image_decode_threads(&mut self, threads: crate::image::DecodeThreads) {
        self.images.set_decode_threads(threads);
    }
    /// Enable or disable full-screen CMV frame decoding; disabled by default.
    pub fn set_decode_movie_frames(&mut self, enabled: bool) {
        self.movies.set_decode_frames(enabled);
    }
    /// Logical screen dimensions `[width, height]` in engine pixels.
    pub fn screen_size(&self) -> [u32; 2] {
        self.messages.screen_size()
    }
    /// Return the latest full-screen movie frame, or render the scene in software.
    ///
    /// Movie frames keep their decoded dimensions; scene output uses [`Self::screen_size`].
    /// To include message windows and glyphs, pass them to
    /// [`crate::scene_render::render_scene`] as additional objects. Unsupported scene
    /// state returns an error.
    pub fn render_frame(&self) -> Result<crate::raster::RgbaImage, crate::FormatError> {
        match self.movie_frame() {
            Some(image) => Ok(image.clone()),
            None => {
                let [width, height] = self.screen_size();
                crate::scene_render::render_scene(
                    &self.scene,
                    Some(&self.camera),
                    &[],
                    width,
                    height,
                )
            }
        }
    }
    /// Service audio and advance movies and scene state by `delta_ms` milliseconds.
    ///
    /// Uses the movie audio clock when provided. Full-screen CMV decoding requires
    /// [`Self::set_decode_movie_frames`]; MGV playback uses its installed decoder.
    /// Call this or [`Self::advance_frame`] once at the frame boundary, not both.
    /// Resource, codec, audio, and scene-update failures propagate to the host.
    pub fn advance_decoded_frame(&mut self, delta_ms: u32) -> Result<(), Box<dyn Error>> {
        if self.quit {
            return Ok(());
        }
        self.audio_backend.update()?;
        let clock = self.audio_backend.movie_clock()?;
        let had_movie = self.movies.has_player();
        self.movies.advance_with_clock(
            delta_ms,
            clock,
            &mut DirectoryMovies {
                root: &self.root,
                directory: self.bootstrap.paths().get(&(0x2015, 0)).map(Vec::as_slice),
            },
        )?;
        if had_movie && !self.movies.has_player() && clock.is_some() {
            self.audio_backend.stop_movie_audio()?;
        }
        self.movies.advance_scene(delta_ms, &mut self.scene)?;
        self.advance_frame_state(delta_ms)
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
        if self.movies.has_player() {
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
        if self.host_capture {
            self.capture_due = true;
            return Ok(());
        }
        self.draw_capture()
    }
    fn draw_capture(&mut self) -> Result<(), Box<dyn Error>> {
        let windows: Vec<_> = self.messages.visible_window_objects().collect();
        let [width, height] = self.screen_size();
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
    /// A host that composites frames itself (a GPU) can hand over the frame it drew for
    /// a requested capture instead of having the core draw the scene again in software.
    /// After each frame advance it checks [`Self::capture_due`] and calls
    /// [`Self::supply_capture`]; a capture it leaves due is drawn in software by the
    /// next VM step.
    pub fn set_host_capture(&mut self, enabled: bool) {
        self.host_capture = enabled;
    }
    /// Whether the last frame update left a screenshot request for the host to fulfill.
    pub fn capture_due(&self) -> bool {
        self.capture_due
    }
    /// Supply the host-rendered scene and message windows for a pending screenshot.
    ///
    /// The frame must match [`Self::screen_size`] and exclude host overlays such as
    /// the cursor. Returns an error if no capture is due or the dimensions differ.
    pub fn supply_capture(
        &mut self,
        frame: crate::raster::RgbaImage,
    ) -> Result<(), Box<dyn Error>> {
        if !self.capture_due {
            return Err("no screen capture is due".into());
        }
        if [frame.width(), frame.height()] != self.screen_size() {
            return Err("supplied screen capture differs from the screen size".into());
        }
        self.capture_due = false;
        self.capture.take(frame)?;
        Ok(())
    }
    /// Inspect the retained screenshot and pending request state.
    pub fn capture(&self) -> &ScreenCapture {
        &self.capture
    }
    /// Button groups and selection state for this frame.
    pub fn buttons(&self) -> &ButtonHost {
        &self.buttons
    }
    /// Current action and pointer state, shared across VM budget slices.
    pub fn input(&self) -> &ActionInput {
        &self.input
    }
    /// Host input is refreshed once at a frame boundary, and remains consumable across
    /// cooperative budget slices. Core does not interpret physical window events.
    pub fn input_mut(&mut self) -> &mut ActionInput {
        &mut self.input
    }
    /// Advance scene and movie timing by `delta_ms` milliseconds without full-screen CMV reads.
    ///
    /// Unlike [`Self::advance_decoded_frame`], this does not service audio output or
    /// use its movie clock. Scene movies and MGV playback still advance. The VM timer
    /// delta is consumed once after yield, never per instruction or budget slice.
    pub fn advance_frame(&mut self, delta_ms: u32) -> Result<(), Box<dyn Error>> {
        if self.quit {
            return Ok(());
        }
        self.movies.advance_scene(delta_ms, &mut self.scene)?;
        self.movies.advance_mgv(delta_ms, None)?;
        self.movies.advance(delta_ms);
        self.advance_frame_state(delta_ms)
    }
    fn advance_frame_state(&mut self, delta_ms: u32) -> Result<(), Box<dyn Error>> {
        if !self.frame_running() {
            self.frame_delta = 0;
            return Ok(());
        }
        self.sequence.advance_frame_with_windows(
            delta_ms,
            &mut self.scene,
            &mut self.images.with_paths(self.bootstrap.paths()),
            &mut self.camera,
            &mut self.messages,
        )?;
        self.scene.advance_frame(delta_ms);
        self.scene
            .advance_effect_pools(delta_ms, &mut self.random, &self.camera)?;
        self.messages.advance_frame(delta_ms);
        self.messages
            .advance_auxiliary_motion(delta_ms, &mut self.scene)?;
        self.camera.advance_frame(delta_ms);
        // cmvs-20070119 0x407810 updates the effect slots after the view motion.
        self.scene.advance_legacy_effects(
            delta_ms,
            &mut self.camera,
            self.audio.settings(),
            self.audio_backend.as_mut(),
        )?;
        self.frame_delta = delta_ms;
        self.take_capture()
    }
    /// Inspect accumulated VM timers and the independent stopwatch.
    pub fn timers(&self) -> &VmTimers {
        &self.timers
    }
    /// Current script-provided title for gameplay saves.
    pub fn save_title(&self) -> &SaveTitle {
        &self.save_title
    }
    /// Current script-provided gameplay-save metadata.
    pub fn save_info(&self) -> SaveInfo {
        self.save_info
    }
    /// Current pointer drag and flick state.
    pub fn gesture(&self) -> &PointerGesture {
        &self.gesture
    }
    /// Current script-controlled message wait and skip flags.
    pub fn message_flags(&self) -> MessageFlags {
        self.message_flags
    }
    /// Update host focus, discarding pending frame time when the running state changes.
    pub fn set_focused(&mut self, focused: bool) {
        let was_running = self.frame_running();
        self.focused = focused;
        if was_running != self.frame_running() {
            self.frame_delta = 0;
        }
    }
    /// Whether the session is running under the current focus policy.
    pub fn frame_running(&self) -> bool {
        !self.quit && (self.focused || self.settings.active_draw())
    }
    /// Runtime settings, including font, focus, and message preferences.
    pub fn settings(&self) -> &crate::settings::SettingsHost {
        &self.settings
    }
    // Startup check 0x14000b666 substitutes a missing configured font face.
    /// Set the initial font face, encoded in the game's text encoding.
    ///
    /// Call before font objects are created. Hosts may use this to replace an
    /// unavailable configured face with the built-in default.
    pub fn set_font_face(&mut self, face: &[u8]) -> Result<(), Box<dyn Error>> {
        self.settings.set_font_face(face)?;
        self.messages.set_default_face(face);
        Ok(())
    }
    /// Window and text state to be composed by the host.
    pub fn messages(&self) -> &MessageHost {
        &self.messages
    }
    /// Recorded message history.
    pub fn history(&self) -> &HistoryHost {
        &self.history
    }
    /// Explicit single-line message recording; no font pixels are generated.
    pub fn set_record_text(&mut self, enabled: bool) {
        self.messages.set_record_output(enabled);
    }
    /// Install font measurements for text layout before executing text output.
    pub fn set_glyph_metrics(&mut self, metrics: Box<dyn crate::message::GlyphMetrics>) {
        self.messages.set_glyph_metrics(metrics);
    }
    /// Gameplay-save slot metadata and menu state.
    pub fn save_menu(&self) -> &SaveMenuHost {
        &self.save_menu
    }
    /// Total VM step attempts made by the session.
    pub fn steps(&self) -> u64 {
        self.steps
    }
    /// Latest captured gameplay snapshot, or its unavailable state.
    pub fn save_image(&self) -> &SaveImage {
        &self.save_image
    }
    /// Service host-driven output independently of VM/frame progression, including inactivity.
    pub fn poll_audio_output(&mut self) -> Result<(), Box<dyn Error>> {
        self.audio_backend.update()
    }
    /// Audio channels, resource registrations, and playback request state.
    pub fn audio(&self) -> &AudioHost {
        &self.audio
    }
    /// Install a host audio backend.
    ///
    /// Configures the new backend with registered paths, archive keys, voice ducking,
    /// and output activity. If configuration fails, the existing backend remains installed.
    pub fn set_audio_backend(
        &mut self,
        mut backend: Box<dyn AudioBackend>,
    ) -> Result<(), Box<dyn Error>> {
        backend.configure_paths(self.bootstrap.paths())?;
        backend.configure_archive_parameters(self.bootstrap.archive_parameters())?;
        backend.configure_voice_ducking(self.audio.voice_ducking())?;
        if !self.audio.output_enabled() {
            backend.set_output_enabled(false)?;
        }
        self.audio_backend = backend;
        Ok(())
    }
    /// Set the deterministic random seed; frame updates also consume random values.
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
        self.set_save_profile(profile)
    }
    /// Install an already locked profile, including an explicitly selected
    /// game-local savedata profile. The lock remains owned by this session.
    pub fn set_save_profile(&mut self, profile: ProfileSystemSave) -> Result<(), Box<dyn Error>> {
        if self.steps != 0 || self.system_output.is_some() {
            return Err("select one system-save output mode before VM execution".into());
        }
        let saves = DirectorySaves::new(profile.directory())?;
        let configuration = profile.read_configuration()?;
        let primary = profile.path();
        let source = if fs::try_exists(&primary)? {
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
    /// Limit VM steps between yields; the default is 1,000,000.
    ///
    /// Once the limit is reached, [`Self::tick`] returns an error without stepping.
    /// This host watchdog does not limit total session steps. Hosts must also use
    /// a per-frame instruction budget to keep their event loop responsive.
    pub fn set_yield_watchdog(&mut self, steps: u64) {
        self.yield_watchdog = steps;
    }
    /// Choose how subsequent gameplay-snapshot requests are handled.
    pub fn set_snapshot_policy(&mut self, policy: SnapshotPolicy) {
        self.snapshot_policy = policy;
    }
    /// Attempt one VM step and dispatch its system command through the profile gate.
    ///
    /// Returns [`StartupEvent::Inactive`] without stepping when focus policy pauses
    /// execution, or [`StartupEvent::Quit`] after shutdown. Stop on an unhandled
    /// command or snapshot barrier; repeated ticks cannot advance past either.
    /// Errors include command-gate rejection, the yield watchdog, and failures from
    /// instruction execution, resources, or backend services. See the [module guide](self)
    /// for input and frame-update order.
    pub fn tick(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        if self.quit {
            return Ok(StartupEvent::Quit);
        }
        self.audio.sync_output_activity(
            self.focused || self.settings.active_draw(),
            self.audio_backend.as_mut(),
        )?;
        if !self.frame_running() {
            return Ok(StartupEvent::Inactive);
        }
        if std::mem::take(&mut self.capture_due) {
            self.draw_capture()?;
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
    fn handle_runtime(
        &mut self,
        request: crate::dispatch::RuntimeCall,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        use crate::dispatch::RuntimeOperation::*;
        request.validate(&self.machine)?;
        match request.operation() {
            Quit | Reset => return self.handle_lifecycle(&request),
            CaptureSnapshot => {
                let pc = request.pc();
                let mut vm = self
                    .machine
                    .capture_vm()
                    .map_err(|why| crate::evaluate::error(pc, 0x22b2, why))?;
                vm.pc = pc as u32 + 2;
                vm.repeat_count = 0;
                self.capture_snapshot(vm)?;
                request.complete(&mut self.machine, 0x4000)?;
                return Ok(StartupEvent::SnapshotCaptured);
            }
            LoadSlot { slot } => self.handle_load_slot(&request, slot, backend)?,
            SaveSlot { slot } => self.handle_save_slot(&request, slot)?,
            EditSlots { first, second } => self.handle_edit_slots(&request, first, second)?,
            SetSlotComment { slot, text } => self.handle_set_slot_comment(&request, slot, text)?,
            SetSlotInfo { slot, values } => self.handle_set_slot_info(&request, slot, values)?,
            Thumbnail => self.handle_thumbnail(&request)?,
            LatestSlot { start, count } => self.handle_latest_slot(&request, start, count)?,
            SlotInfo {
                slot,
                object,
                child,
                label_slot,
                text_slot,
            } => self.handle_slot_info(&request, slot, object, child, label_slot, text_slot)?,
        }
        Ok(StartupEvent::Progress)
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
        let resolved = self.gate.resolve(&self.machine, pc, command)?;
        if let Some(first) = resolved.first_unreviewed {
            self.notices.push(first);
        }
        if crate::dispatch::is_lifecycle(command) {
            let flow = crate::dispatch::run(
                resolved.semantics,
                &mut self.machine,
                &mut crate::dispatch::Hosts {
                    runtime: true,
                    ..Default::default()
                },
            )?;
            return match flow {
                Some(crate::dispatch::Flow::Runtime(request)) => {
                    self.handle_runtime(request, backend)
                }
                _ => Ok(StartupEvent::Unhandled {
                    pc,
                    command: command + 0x2000,
                }),
            };
        }
        let id = self.system_save_identifier();
        if command == 0x129
            && let Some(output) = &mut self.system_output
        {
            let flow = crate::dispatch::run(
                resolved.semantics,
                &mut self.machine,
                &mut crate::dispatch::Hosts {
                    system_write: Some(crate::dispatch::SystemWriteHosts {
                        host: &mut self.system_host,
                        sink: output,
                        random: &mut self.random,
                        game_id: id,
                    }),
                    ..Default::default()
                },
            )?;
            if flow == Some(crate::dispatch::Flow::SystemWritten) {
                self.system_source = FileSystemSave::new(output.path());
                return Ok(StartupEvent::Progress);
            }
            return Ok(StartupEvent::Unhandled {
                pc,
                command: command + 0x2000,
            });
        }
        if crate::dispatch::uses_bootstrap_for(resolved.semantics, command) {
            let mut hosts = crate::dispatch::Hosts::default();
            if command == 0x34d {
                hosts.function_keys =
                    Some(crate::dispatch::FunctionKeys::Startup(&mut self.bootstrap));
            } else {
                hosts.bootstrap = Some(&mut self.bootstrap);
            }
            let flow = crate::dispatch::run(resolved.semantics, &mut self.machine, &mut hosts)?;
            self.images
                .configure_parameters(self.bootstrap.archive_parameters());
            self.scene_movies
                .configure_parameters(self.bootstrap.archive_parameters());
            // The audio backend keeps its own copy, so every later sound load by name
            // sees the current registrations and keys.
            self.audio_backend.configure_paths(self.bootstrap.paths())?;
            self.audio_backend
                .configure_archive_parameters(self.bootstrap.archive_parameters())?;
            return Ok(if flow == Some(crate::dispatch::Flow::Done) {
                StartupEvent::Progress
            } else {
                StartupEvent::Unhandled {
                    pc,
                    command: command + 0x2000,
                }
            });
        }
        let gate = SkipGate {
            effects: self.effects,
            skip: self.message_flags.skip(),
        };
        if crate::dispatch::is_wait_for(resolved.semantics, command) {
            let flow = crate::dispatch::run(
                resolved.semantics,
                &mut self.machine,
                &mut crate::dispatch::Hosts {
                    waits: Some(crate::dispatch::WaitHosts {
                        scene: Some(&mut self.scene),
                        camera: Some(&mut self.camera),
                        input: &mut self.input,
                        gate,
                        change_images: true,
                    }),
                    ..Default::default()
                },
            )?
            .expect("registered wait has a dispatch result");
            self.resume_run = flow.yields();
            return Ok(match flow {
                crate::dispatch::Flow::Done => StartupEvent::Progress,
                crate::dispatch::Flow::Runtime(_)
                | crate::dispatch::Flow::Control(_)
                | crate::dispatch::Flow::SystemSave(_)
                | crate::dispatch::Flow::SystemWritten
                | crate::dispatch::Flow::Platform(_) => {
                    unreachable!("wait handler returned a platform event")
                }
                crate::dispatch::Flow::Yield | crate::dispatch::Flow::Wait => StartupEvent::Yield,
                crate::dispatch::Flow::Unhandled | crate::dispatch::Flow::Unavailable => {
                    StartupEvent::Unhandled {
                        pc,
                        command: command + 0x2000,
                    }
                }
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
        if crate::dispatch::is_message_wait(command) {
            let flow = crate::dispatch::run(
                resolved.semantics,
                &mut self.machine,
                &mut crate::dispatch::Hosts {
                    message_wait: Some(crate::dispatch::MessageWaitHosts {
                        messages: &mut self.messages,
                        input: &mut self.input,
                        flags: message_flags,
                        context: crate::message::AutoAdvanceContext {
                            settings: self.settings.auto_advance(),
                            delta_ms: self.timers.delta_ms(),
                            gesture_result: self
                                .gesture
                                .field(88)
                                .is_some_and(|value| value as i32 >= 0),
                            read_eligible,
                            audio: self.audio_backend.as_ref(),
                        },
                    }),
                    ..Default::default()
                },
            )?
            .expect("registered message wait");
            self.message_flags = message_flags;
            self.resume_run = flow.yields();
            return Ok(match flow {
                crate::dispatch::Flow::Done => StartupEvent::Progress,
                crate::dispatch::Flow::Wait | crate::dispatch::Flow::Yield => StartupEvent::Yield,
                _ => StartupEvent::Unhandled {
                    pc,
                    command: command + 0x2000,
                },
            });
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
        let mut image_source = self.images.with_paths(self.bootstrap.paths());
        let input_clock = self.timers.clock();
        let movie_settings = self.audio.settings();
        let mut movie_source = DirectoryMovies {
            root: &self.root,
            directory: self.bootstrap.paths().get(&(0x2015, 0)).map(Vec::as_slice),
        };
        let mut scene_movie_source = self
            .scene_movies
            .with_paths(self.bootstrap.paths(), ResourceKind::SceneMovie);
        let is_movie_player = crate::dispatch::uses_movie_player_for(resolved.semantics, command);
        let mut sequence_messages = Some(&mut self.messages);
        let mut hosts = crate::dispatch::Hosts {
            runtime: true,
            message_wait: None,
            snapshot: Some(crate::dispatch::SnapshotHosts {
                image: &mut self.save_image,
                capture: self.snapshot_policy == SnapshotPolicy::Capture,
                continue_without_image: self.snapshot_policy
                    == SnapshotPolicy::ContinueWithoutImage,
            }),
            scripts: Some(crate::dispatch::ScriptHosts {
                host: &mut self.control,
                source: &mut self.source,
            }),
            system_read: Some(crate::dispatch::SystemReadHosts {
                host: &mut self.system_host,
                source: &mut self.system_source,
                game_id: id,
                persistent: self.bootstrap.cvns_persistent_starts(),
            }),
            system_write: None,
            save_source: Some(&mut self.saves),
            bootstrap: None,
            function_keys: None,
            platform: None,
            device_frame: None,
            capture: Some(&mut self.capture),
            sequence: None,
            movies: Some(&mut self.movies),
            movie_scene: None,
            movie_player: None,
            scene_movie_playback: true,
            audio: None,
            scene: None,
            camera: None,
            images: None,
            waits: None,
            text: None,
            messages: None,
            window_images: None,
            input: None,
            buttons: Some(&mut self.buttons),
            list_input: true,
            timers: Some(&mut self.timers),
            flags: Some(&mut self.message_flags),
            random: Some(&mut self.random),
            settings: Some(&mut self.settings),
            history: Some(&mut self.history),
            strings: true,
            math: true,
            gesture: Some(&mut self.gesture),
            popup: Some(&mut self.save_menu),
            save_title: Some(&mut self.save_title),
            save_info: Some(&mut self.save_info),
            effects: Some(&mut self.effects),
        };
        if crate::dispatch::uses_message_state(command) {
            hosts.messages = sequence_messages.take();
            if crate::dispatch::uses_input_for(resolved.semantics, command) {
                hosts.input = Some(crate::dispatch::InputHosts {
                    input: &mut self.input,
                    skip: message_flags.skip(),
                    clock: Some(input_clock),
                });
            }
        } else if crate::dispatch::uses_input_for(resolved.semantics, command) {
            hosts.input = Some(crate::dispatch::InputHosts {
                input: &mut self.input,
                skip: message_flags.skip(),
                clock: Some(input_clock),
            });
        } else if !crate::dispatch::uses_sequence(command) {
            hosts.text = Some(crate::dispatch::TextHosts {
                messages: sequence_messages.take().expect("text owns message host"),
                input: &mut self.input,
                flags: message_flags,
            });
        }
        if crate::dispatch::uses_platform_for(resolved.semantics, command) {
            hosts.platform = Some(crate::dispatch::PlatformHosts {
                host: &mut self.platform,
                backend,
                assets: &mut self.assets,
                cursors: self.bootstrap.cursor_names(),
            });
        } else if crate::dispatch::uses_device_frame_for(resolved.semantics, command) {
            hosts.device_frame = Some(backend);
        }
        if is_movie_player {
            hosts.movie_player = Some(crate::dispatch::MoviePlayerHosts {
                source: &mut movie_source,
                settings: movie_settings,
                backend: self.audio_backend.as_mut(),
            });
        } else {
            hosts.audio = Some(crate::dispatch::AudioHosts {
                audio: &mut self.audio,
                backend: self.audio_backend.as_mut(),
            });
        }
        if crate::dispatch::uses_sequence(command) {
            hosts.sequence = Some(crate::dispatch::SequenceHosts {
                host: &mut self.sequence,
                execution: Some(crate::sequence::SequenceContext {
                    scene: &mut self.scene,
                    images: &mut image_source,
                    camera: Some(&mut self.camera),
                    messages: sequence_messages.take(),
                }),
            });
        } else {
            hosts.camera = Some(&mut self.camera);
            if crate::dispatch::uses_movie_scene_for(resolved.semantics, command) {
                hosts.movie_scene = Some(crate::dispatch::MovieSceneHosts {
                    scene: &mut self.scene,
                    source: &mut scene_movie_source,
                });
            } else if crate::dispatch::uses_window_images(command) {
                hosts.window_images = Some(&mut image_source);
            } else if crate::dispatch::uses_images(command) {
                hosts.images = Some(crate::dispatch::ImageHosts {
                    scene: &mut self.scene,
                    source: &mut image_source,
                    change_images: true,
                });
            } else {
                hosts.scene = Some(&mut self.scene);
            }
        }
        if let Some(flow) = crate::dispatch::run(resolved.semantics, &mut self.machine, &mut hosts)?
        {
            if let crate::dispatch::Flow::Runtime(request) = flow {
                return self.handle_runtime(request, backend);
            }
            if let crate::dispatch::Flow::SystemSave(event) = flow {
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
            if let crate::dispatch::Flow::Control(event) = flow {
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
            return Ok(match flow {
                crate::dispatch::Flow::Runtime(_)
                | crate::dispatch::Flow::Control(_)
                | crate::dispatch::Flow::SystemSave(_)
                | crate::dispatch::Flow::SystemWritten => {
                    unreachable!("storage event handled before ordinary flow")
                }
                crate::dispatch::Flow::Done => StartupEvent::Progress,
                crate::dispatch::Flow::Platform(event) => {
                    let yields = event == crate::platform::PlatformEvent::Yield;
                    self.resume_run = yields;
                    if yields {
                        StartupEvent::Yield
                    } else {
                        StartupEvent::Progress
                    }
                }
                crate::dispatch::Flow::Yield | crate::dispatch::Flow::Wait => {
                    self.message_flags = message_flags;
                    self.resume_run = true;
                    StartupEvent::Yield
                }
                crate::dispatch::Flow::Unhandled | crate::dispatch::Flow::Unavailable => {
                    StartupEvent::Unhandled {
                        pc,
                        command: command + 0x2000,
                    }
                }
            });
        }
        Ok(StartupEvent::Unhandled {
            pc,
            command: command + 0x2000,
        })
    }
}

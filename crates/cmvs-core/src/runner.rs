//! Bounded real-data startup orchestration for the researched Amatsutsumi layout.
//! Snapshots remain barriers unless the caller explicitly continues without an image.
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

pub fn load_start(root: &Path) -> Result<Script, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(root.join("data/pack/start.ps3"))?
        .take(MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAYLOAD + 48 {
        return Err("startup script exceeds research limit".into());
    }
    Ok(Script::decode(&bytes)?)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEvent {
    Progress,
    Yield,
    Unhandled {
        pc: usize,
        command: u16,
    },
    MainPreparedNeedsSnapshot,
    /// Main entry reached; the original snapshot was not built and saving stays unavailable.
    MainEnteredWithoutSnapshot,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SnapshotPolicy {
    #[default]
    Barrier,
    ContinueWithoutImage,
}
pub struct Startup<'a> {
    machine: Machine<'a>,
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
    skip_auto_resume: bool,
    auto_resume_checked: bool,
    auto_resume_skipped: bool,
}
impl<'a> Startup<'a> {
    pub fn new(script: &'a Script, root: &Path, virtual_tick: u32) -> Result<Self, Box<dyn Error>> {
        let mut machine = Machine::new(script)?;
        machine.set_current_name(b"start.ps3".to_vec());
        Ok(Self {
            machine,
            bootstrap: Bootstrap::default(),
            control: ScriptControl::new(virtual_tick),
            source: ArchiveScripts::new(File::open(root.join("data/pack/script.cpz"))?)?,
            saves: DirectorySaves::new(root.join("save"))?,
            save_menu: SaveMenuHost::default(),
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
            // EFFECT_USE constructor value; the configuration may override it.
            effects: true,
            timers: VmTimers::default(),
            resume_run: true,
            frame_delta: 0,
            images: ImageArchives::new(root)?,
            save_title: SaveTitle::default(),
            save_info: SaveInfo::default(),
            gesture: PointerGesture::default(),
            message_flags: MessageFlags::default(),
            messages: MessageHost::default(),
            history: HistoryHost::default(),
            // Explicit research seed, like the virtual tick; the original seeds from timeGetTime.
            random: CrtRandom::new(0),
            audio: AudioHost::default(),
            audio_backend: Box::new(RecordingAudio::default()),
            steps: 0,
            since_yield: 0,
            yield_watchdog: 1_000_000,
            main_barrier: false,
            snapshot_policy: SnapshotPolicy::default(),
            save_image: SaveImage::default(),
            skip_auto_resume: false,
            auto_resume_checked: false,
            auto_resume_skipped: false,
        })
    }
    pub fn machine(&self) -> &Machine<'a> {
        &self.machine
    }
    /// Explicit Amatsutsumi preview policy, not a VM command: after the first system-save
    /// read, clear in-memory integer 2198 so main.ps3 does not attempt startup auto-load.
    /// Other persistent values and every file stay unchanged. Configure before ticking.
    pub fn set_skip_auto_resume(&mut self, enabled: bool) -> Result<(), &'static str> {
        if self.steps != 0 {
            return Err("skip-auto-resume must be configured before startup");
        }
        self.skip_auto_resume = enabled;
        Ok(())
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
        self.movies.advance_scene(delta_ms, &mut self.scene)?;
        self.sequence.advance_frame_with_images(
            delta_ms,
            &mut self.scene,
            &mut self.images.with_paths(self.bootstrap.paths()),
        )?;
        self.scene.advance_frame(delta_ms);
        self.messages.advance_frame(delta_ms);
        self.messages
            .advance_auxiliary_motion(delta_ms, &mut self.scene)?;
        self.movies.advance(delta_ms);
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
    pub fn set_audio_backend(&mut self, backend: Box<dyn AudioBackend>) {
        self.audio_backend = backend;
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
        let primary = profile.path();
        let source = if primary.try_exists()? {
            primary
        } else {
            self.root.join("save/system.dat")
        };
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
    fn step_once(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        if self.main_barrier {
            return Ok(StartupEvent::MainPreparedNeedsSnapshot);
        }
        if self.resume_run {
            // 0x140019430: the gesture update follows the input refresh and precedes the VM.
            let delta = std::mem::take(&mut self.frame_delta);
            self.timers.begin_run(delta);
            self.gesture.frame(&mut self.input, self.timers.clock())?;
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
        if matches!(command, 0xa0 | 0xa4 | 0xa6 | 0xb0 | 0xba) {
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
        if let Some(step) =
            self.messages
                .try_wait(&mut self.machine, &mut self.input, self.message_flags)?
        {
            self.resume_run = step == Step::Yield;
            return Ok(if step == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        }
        if let Some(step) = self.messages.try_record_output(
            &mut self.machine,
            &mut self.input,
            self.message_flags,
        )? {
            self.resume_run = step == Step::Yield;
            return Ok(StartupEvent::Yield);
        }
        if command == 0x128
            && self.skip_auto_resume
            && !self.auto_resume_checked
            && id != Some(0x1780_0016)
        {
            return Err(
                "skip-auto-resume is only verified for Amatsutsumi game ID 0x17800016".into(),
            );
        }
        if let Some(event) =
            self.system_host
                .try_handle(&mut self.machine, &mut self.system_source, id)?
        {
            if !self.auto_resume_checked {
                if self.skip_auto_resume && event == SystemSaveEvent::Applied {
                    self.machine.state_mut().set_integer(2198, 0)?;
                    self.auto_resume_skipped = true;
                }
                self.auto_resume_checked = true;
            }
            return Ok(StartupEvent::Progress);
        }
        if self
            .platform
            .try_handle(
                &mut self.machine,
                backend,
                &mut self.assets,
                self.bootstrap.cursor_names(),
            )?
            .is_some()
        {
            return Ok(StartupEvent::Progress);
        }
        if let Some(event) = self
            .control
            .try_handle(&mut self.machine, &mut self.source)?
        {
            if event == ControlEvent::MainPreparedNeedsSnapshot {
                if self.snapshot_policy == SnapshotPolicy::Barrier {
                    self.main_barrier = true;
                    return Ok(StartupEvent::MainPreparedNeedsSnapshot);
                }
                self.save_image.capture_unavailable(&mut self.machine)?;
                return Ok(StartupEvent::MainEnteredWithoutSnapshot);
            }
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
            || self
                .messages
                .try_handle_scene(&mut self.machine, &mut self.scene)?
            || self.messages.try_handle(&mut self.machine)?
            || self.history.try_handle(&mut self.machine)?
            || self
                .messages
                .try_handle_pointer(&mut self.machine, &self.input)?
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
            || text::try_handle(&mut self.machine)?
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

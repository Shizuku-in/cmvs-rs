//! Bounded real-data startup orchestration for the researched Amatsutsumi layout.
//! Snapshots remain barriers unless the caller explicitly continues without an image.
use crate::{
    MAX_PAYLOAD,
    audio::{AudioBackend, AudioHost, RecordingAudio},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    camera::CameraHost,
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
    save_menu::{DirectorySaves, SaveMenuHost},
    scene::SceneHost,
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    sequence::SequenceHost,
    snapshot::{SaveImage, SaveInfo, SaveTitle},
    system_save::{FileSystemSave, SystemSaveHost},
    text,
    timers::VmTimers,
};
use std::{
    error::Error,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

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
    assets: DirectoryAssets,
    platform: PlatformHost,
    scene: SceneHost,
    buttons: ButtonHost,
    movies: MovieHost,
    root: PathBuf,
    sequence: SequenceHost,
    camera: CameraHost,
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
    steps: u32,
    main_barrier: bool,
    snapshot_policy: SnapshotPolicy,
    save_image: SaveImage,
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
            assets: DirectoryAssets::new(root)?,
            platform: PlatformHost::default(),
            scene: SceneHost::default(),
            buttons: ButtonHost::default(),
            movies: MovieHost::default(),
            root: root.canonicalize()?,
            sequence: SequenceHost::default(),
            camera: CameraHost::default(),
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
            main_barrier: false,
            snapshot_policy: SnapshotPolicy::default(),
            save_image: SaveImage::default(),
        })
    }
    pub fn machine(&self) -> &Machine<'a> {
        &self.machine
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
    pub fn movie_frame(&self) -> Option<&crate::raster::RgbaImage> {
        self.movies.player().and_then(|m| m.image.as_ref())
    }
    pub fn set_decode_movie_frames(&mut self, enabled: bool) {
        self.movies.set_decode_frames(enabled);
    }
    pub fn render_frame(&self) -> Result<crate::raster::RgbaImage, crate::FormatError> {
        match self.movie_frame() {
            Some(image) => Ok(image.clone()),
            None => crate::scene_render::render(&self.scene, 1280, 720),
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
        self.scene.advance_frame(delta_ms);
        self.messages.advance_frame(delta_ms);
        self.frame_delta = delta_ms;
        Ok(())
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
    pub fn advance_frame(&mut self, delta_ms: u32) {
        self.scene.advance_frame(delta_ms);
        self.messages.advance_frame(delta_ms);
        self.movies.advance(delta_ms);
        self.frame_delta = delta_ms;
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
    pub fn steps(&self) -> u32 {
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
    pub fn set_snapshot_policy(&mut self, policy: SnapshotPolicy) {
        self.snapshot_policy = policy;
    }
    pub fn tick(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        if self.main_barrier {
            return Ok(StartupEvent::MainPreparedNeedsSnapshot);
        }
        if self.steps >= 1_000_000 {
            return Err("startup research step budget exhausted".into());
        }
        if self.resume_run {
            self.timers.begin_run(std::mem::take(&mut self.frame_delta));
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
        if self
            .system_host
            .try_handle(&mut self.machine, &mut self.system_source, id)?
            .is_some()
        {
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
            || self.scene.try_handle_image(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.scene.try_handle_mask(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.save_title.try_handle(&mut self.machine)?
            || self.save_info.try_handle(&mut self.machine)?
            || self.gesture.try_handle(&mut self.machine)?
            || self.save_menu.try_handle_popup(&mut self.machine)?
            || self.message_flags.try_handle(&mut self.machine)?
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
            || self
                .buttons
                .try_handle_input(&mut self.machine, &mut self.scene, &mut self.input)?
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
            || self.sequence.try_handle(&mut self.machine)?
            || self.camera.try_handle(&mut self.machine)?
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

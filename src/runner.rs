//! Bounded real-data startup orchestration for the researched Amatsutsumi layout.
//! The main-transition snapshot is a barrier unless the caller explicitly continues without an image.
use crate::{
    MAX_PAYLOAD,
    audio::{AudioBackend, AudioHost, RecordingAudio},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    fade::SkipGate,
    gesture::PointerGesture,
    image::ImageArchives,
    input::ActionInput,
    machine::{Machine, Step},
    message_flags::MessageFlags,
    movie::MovieHost,
    platform::{DirectoryAssets, PlatformBackend, PlatformHost},
    ps3::Script,
    random::CrtRandom,
    save_menu::{DirectorySaves, SaveMenuHost},
    scene::SceneHost,
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    sequence::SequenceHost,
    snapshot::{SaveImage, SaveTitle},
    system_save::{FileSystemSave, SystemSaveHost},
    text,
    timers::VmTimers,
};
use std::{error::Error, fs::File, io::Read, path::Path};

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
    sequence: SequenceHost,
    input: ActionInput,
    effects: bool,
    timers: VmTimers,
    resume_run: bool,
    frame_delta: u32,
    images: ImageArchives,
    save_title: SaveTitle,
    gesture: PointerGesture,
    message_flags: MessageFlags,
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
            sequence: SequenceHost::default(),
            // No verified key bindings yet: no action is active (see `input`).
            input: ActionInput::default(),
            // EFFECT_USE constructor value; the configuration may override it.
            effects: true,
            timers: VmTimers::default(),
            resume_run: true,
            frame_delta: 0,
            images: ImageArchives::new(root)?,
            save_title: SaveTitle::default(),
            gesture: PointerGesture::default(),
            message_flags: MessageFlags::default(),
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
    pub fn buttons(&self) -> &ButtonHost {
        &self.buttons
    }
    pub fn input(&self) -> &ActionInput {
        &self.input
    }
    /// Update the scene and supply the latest delta for the next VM run entry.
    /// The timer delta is consumed once after yield, never per instruction/budget slice.
    /// Callback registration time remains a separate research clock.
    pub fn advance_frame(&mut self, delta_ms: u32) {
        self.scene.advance_frame(delta_ms);
        self.frame_delta = delta_ms;
    }
    pub fn timers(&self) -> &VmTimers {
        &self.timers
    }
    pub fn save_title(&self) -> &SaveTitle {
        &self.save_title
    }
    pub fn gesture(&self) -> &PointerGesture {
        &self.gesture
    }
    pub fn message_flags(&self) -> MessageFlags {
        self.message_flags
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
            || self.timers.try_handle(&mut self.machine)?
            || self.bootstrap.try_handle(&mut self.machine)?.is_some()
            || self.scene.try_handle(&mut self.machine)?
            || self.scene.try_handle_image(
                &mut self.machine,
                &mut self.images.with_paths(self.bootstrap.paths()),
            )?
            || self.save_title.try_handle(&mut self.machine)?
            || self.gesture.try_handle(&mut self.machine)?
            || self.save_menu.try_handle_popup(&mut self.machine)?
            || self.message_flags.try_handle(&mut self.machine)?
            || self
                .buttons
                .try_handle(&mut self.machine, &mut self.scene)?
            || self
                .buttons
                .try_handle_input(&mut self.machine, &mut self.scene, &mut self.input)?
            || self.movies.try_handle(&mut self.machine)?
            || self.sequence.try_handle(&mut self.machine)?
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

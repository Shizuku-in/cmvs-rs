//! Shared static system-command routing and sparse identity-bound research overrides.
//! Gate resolution precedes dispatch. Runtime-wide operations return a checked
//! request for the caller's orchestration; they remain pending until acknowledged.
//! A selected override never falls through to root semantics.
pub mod ownership;

use std::{error::Error, fmt};

use crate::{
    audio::{AudioBackend, AudioHost},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    bytecode::DecodeError,
    camera::CameraHost,
    capture::ScreenCapture,
    evaluate::error,
    fade::SkipGate,
    gesture::PointerGesture,
    history::HistoryHost,
    image::ImageSource,
    input::ActionInput,
    machine::{Machine, Step},
    message::{MessageHost, font_cells},
    message_flags::MessageFlags,
    movie::{MovieHost, MovieSource},
    platform::{PlatformAssets, PlatformBackend, PlatformEvent, PlatformHost},
    profile::{EngineProfile, Semantics},
    random::CrtRandom,
    resources::ResourceSource,
    save_menu::{SaveMenuHost, SaveSource},
    scene::SceneHost,
    script_control::{ControlEvent, ScriptControl, ScriptSource},
    sequence::{SequenceContext, SequenceHost},
    settings::SettingsHost,
    snapshot::{SaveInfo, SaveTitle},
    system_save::{SystemSaveEvent, SystemSaveHost, SystemSaveSink, SystemSaveSource},
    timers::VmTimers,
    vm::SystemResult,
};

/// A command failure with its original typed cause and dispatch location.
/// Display delegates to the cause to preserve existing diagnostics.
#[derive(Debug)]
pub struct DispatchError {
    pub pc: usize,
    /// VM command number (opcode minus 0x2000), including for backend failures.
    pub command: u16,
    cause: Box<dyn Error>,
}

impl DispatchError {
    pub fn as_decode(&self) -> Option<&DecodeError> {
        self.cause.downcast_ref()
    }
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(f)
    }
}

impl Error for DispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

pub struct InputHosts<'a> {
    pub input: &'a mut ActionInput,
    pub skip: bool,
    /// Current host clock for drag initiation; absence keeps that capability unavailable.
    pub clock: Option<u32>,
}

pub struct ImageHosts<'a> {
    pub scene: &'a mut SceneHost,
    pub source: &'a mut dyn ImageSource,
    pub change_images: bool,
}

pub struct AudioHosts<'a> {
    pub audio: &'a mut AudioHost,
    pub backend: &'a mut dyn AudioBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Runtime(RuntimeCall),
    Control(ControlEvent),
    SystemSave(SystemSaveEvent),
    SystemWritten,
    Platform(PlatformEvent),
    Done,
    Yield,
    Wait,
    Unhandled,
    Unavailable,
}

impl Flow {
    pub(crate) fn from_step(step: Step, pending: bool) -> Self {
        match step {
            Step::Continue => Self::Done,
            Step::Yield if pending => Self::Wait,
            Step::Yield => Self::Yield,
            _ => unreachable!("command completion only returns continue or yield"),
        }
    }

    pub fn yields(self) -> bool {
        matches!(
            self,
            Self::Yield | Self::Wait | Self::Platform(PlatformEvent::Yield)
        )
    }

    pub(crate) fn wait_step(self) -> Step {
        match self {
            Self::Done => Step::Continue,
            Self::Yield | Self::Wait => Step::Yield,
            _ => unreachable!("wait handlers only return done, yield or wait"),
        }
    }
}

pub struct WaitHosts<'a> {
    pub scene: Option<&'a mut SceneHost>,
    pub camera: Option<&'a mut CameraHost>,
    pub input: &'a mut ActionInput,
    pub gate: SkipGate,
    pub change_images: bool,
}

/// Work requiring the runtime's complete host set. The VM remains pending until
/// the caller executes the request and acknowledges the original invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOperation {
    Quit,
    Reset,
    CaptureSnapshot,
    SaveSlot {
        slot: u32,
    },
    LoadSlot {
        slot: u32,
    },
    EditSlots {
        first: u32,
        second: u32,
    },
    Thumbnail,
    LatestSlot {
        start: i32,
        count: i32,
    },
    SlotInfo {
        slot: i32,
        object: u32,
        child: i32,
        label_slot: u32,
        text_slot: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCall {
    pc: usize,
    command: u16,
    arguments: u64,
    operation: RuntimeOperation,
    stack_len: usize,
    values: [u32; 5],
}
impl RuntimeCall {
    pub fn operation(&self) -> RuntimeOperation {
        self.operation
    }
    pub fn pc(&self) -> usize {
        self.pc
    }
    pub fn command(&self) -> u16 {
        self.command
    }
    /// Reject stale work before any runtime side effect or VM acknowledgement.
    pub fn validate(&self, vm: &Machine<'_>) -> Result<(), DecodeError> {
        if vm.pending_system() != Some((self.pc, self.command))
            || vm.stack().len_bytes() != self.stack_len
            || (0..self.arguments.count_ones() as usize)
                .any(|i| vm.stack().peek_u32(4 * (i + 1)).ok() != Some(self.values[i]))
        {
            return Err(error(
                self.pc,
                self.command + 0x2000,
                "runtime request no longer matches the pending command",
            ));
        }
        Ok(())
    }
    pub fn complete(&self, vm: &mut Machine<'_>, flags: u16) -> Result<Flow, DecodeError> {
        self.validate(vm)?;
        Call {
            vm,
            pc: self.pc,
            command: self.command,
            arguments: self.arguments,
        }
        .finish(flags)
    }
}
pub struct SnapshotHosts<'a> {
    pub image: &'a mut crate::snapshot::SaveImage,
    pub capture: bool,
    pub continue_without_image: bool,
}
pub struct MessageWaitHosts<'a> {
    pub messages: &'a mut MessageHost,
    pub input: &'a mut ActionInput,
    pub flags: MessageFlags,
    pub context: crate::message::AutoAdvanceContext<'a>,
}
pub struct ScriptHosts<'a> {
    pub host: &'a mut ScriptControl,
    pub source: &'a mut dyn ScriptSource,
}
pub struct SystemReadHosts<'a> {
    pub host: &'a mut SystemSaveHost,
    pub source: &'a mut dyn SystemSaveSource,
    pub game_id: Option<u32>,
}
pub struct SystemWriteHosts<'a> {
    pub host: &'a mut SystemSaveHost,
    pub sink: &'a mut dyn SystemSaveSink,
    pub random: &'a mut CrtRandom,
    pub game_id: Option<u32>,
}
pub struct PlatformHosts<'a> {
    pub host: &'a mut PlatformHost,
    pub backend: &'a mut dyn PlatformBackend,
    pub assets: &'a mut dyn PlatformAssets,
    pub cursors: &'a [Vec<u8>],
}
/// The existing startup setting and input-reporting modes have different effects.
/// Callers choose explicitly; handler order never chooses the function-key policy.
pub enum FunctionKeys<'a> {
    Startup(&'a mut Bootstrap),
    Input(&'a mut ActionInput),
}

pub struct SequenceHosts<'a> {
    pub host: &'a mut SequenceHost,
    pub execution: Option<SequenceContext<'a>>,
}
pub struct MovieSceneHosts<'a> {
    pub scene: &'a mut SceneHost,
    pub source: &'a mut dyn ResourceSource,
}
pub struct MoviePlayerHosts<'a> {
    pub source: &'a mut dyn MovieSource,
    pub settings: crate::audio::AudioSettings,
    pub backend: &'a mut dyn AudioBackend,
}

#[derive(Default)]
pub struct Hosts<'a> {
    pub runtime: bool,
    pub snapshot: Option<SnapshotHosts<'a>>,
    pub message_wait: Option<MessageWaitHosts<'a>>,
    pub scripts: Option<ScriptHosts<'a>>,
    pub system_read: Option<SystemReadHosts<'a>>,
    pub system_write: Option<SystemWriteHosts<'a>>,
    pub save_source: Option<&'a mut dyn SaveSource>,
    pub bootstrap: Option<&'a mut Bootstrap>,
    pub function_keys: Option<FunctionKeys<'a>>,
    pub platform: Option<PlatformHosts<'a>>,
    pub device_frame: Option<&'a mut dyn PlatformBackend>,
    pub capture: Option<&'a mut ScreenCapture>,
    pub sequence: Option<SequenceHosts<'a>>,
    pub movies: Option<&'a mut MovieHost>,
    pub movie_scene: Option<MovieSceneHosts<'a>>,
    pub movie_player: Option<MoviePlayerHosts<'a>>,
    /// Scene playback is unavailable in tools that only expose slot queries.
    pub scene_movie_playback: bool,
    pub messages: Option<&'a mut MessageHost>,
    pub window_images: Option<&'a mut dyn ImageSource>,
    pub input: Option<InputHosts<'a>>,
    pub buttons: Option<&'a mut ButtonHost>,
    /// Explicit support for auxiliary-list interaction beyond list metadata.
    pub list_input: bool,
    pub scene: Option<&'a mut SceneHost>,
    pub camera: Option<&'a mut CameraHost>,
    pub images: Option<ImageHosts<'a>>,
    pub audio: Option<AudioHosts<'a>>,
    pub waits: Option<WaitHosts<'a>>,
    pub text: Option<TextHosts<'a>>,
    pub timers: Option<&'a mut VmTimers>,
    pub flags: Option<&'a mut MessageFlags>,
    pub random: Option<&'a mut CrtRandom>,
    pub settings: Option<&'a mut SettingsHost>,
    pub history: Option<&'a mut HistoryHost>,
    pub gesture: Option<&'a mut PointerGesture>,
    pub popup: Option<&'a mut SaveMenuHost>,
    pub save_title: Option<&'a mut SaveTitle>,
    pub save_info: Option<&'a mut SaveInfo>,
    pub effects: Option<&'a mut bool>,
    /// Availability of stateless services in this host environment.
    pub strings: bool,
    pub math: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Runtime,
    Snapshot,
    MessageWait,
    ScriptControl,
    SystemRead,
    SystemWrite,
    SaveMenu,
    Bootstrap,
    FunctionKeys,
    Platform,
    FontDialog,
    Capture,
    CaptureDevice,
    Sequence,
    MovieState,
    MovieScene,
    MovieStart,
    MoviePlayer,
    Quake,
    Strip,
    TextState,
    AuxiliaryScene,
    Windows,
    WindowPointer,
    WindowButtons,
    WindowImages,
    Input,
    Buttons,
    ButtonInput,
    ListInput,
    DragPress,
    Scene,
    Camera,
    SceneImages,
    MaskImages,
    ImageChange,
    Audio,
    Text,
    Timers,
    Flags,
    Random,
    Settings,
    History,
    Strings,
    Math,
    Gesture,
    Popup,
    SaveTitle,
    SaveInfo,
    Effects,
    EffectObjects,
    ParticleImages,
    FadeWait,
    MaskWait,
    QuakeWait,
    StripWait,
    ChangeWait,
    CameraWait,
}

impl Hosts<'_> {
    fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Runtime => self.runtime,
            Capability::Snapshot => self.snapshot.is_some(),
            Capability::MessageWait => self.message_wait.is_some(),
            Capability::ScriptControl => self.scripts.is_some(),
            Capability::SystemRead => self.system_read.is_some(),
            Capability::SystemWrite => self.system_write.is_some(),
            Capability::SaveMenu => self.popup.is_some() && self.save_source.is_some(),
            Capability::Bootstrap => self.bootstrap.is_some(),
            Capability::FunctionKeys => self.function_keys.is_some(),
            Capability::Platform => self.platform.is_some(),
            Capability::FontDialog => {
                self.device_frame.is_some() && self.settings.is_some() && self.messages.is_some()
            }
            Capability::Capture => self.capture.is_some() && self.scene.is_some(),
            Capability::CaptureDevice => {
                self.capture.is_some() && self.scene.is_some() && self.device_frame.is_some()
            }
            Capability::Sequence => self.sequence.is_some(),
            Capability::MovieState => self.movies.is_some(),
            Capability::MovieScene => self.movies.is_some() && self.movie_scene.is_some(),
            Capability::MovieStart => self.movies.is_some() && self.scene_movie_playback,
            Capability::MoviePlayer => self.movies.is_some() && self.movie_player.is_some(),
            Capability::TextState | Capability::AuxiliaryScene => self.messages.is_some(),
            Capability::Windows => self.messages.is_some(),
            Capability::WindowPointer | Capability::WindowButtons => {
                self.messages.is_some() && self.input.is_some()
            }
            Capability::WindowImages => self.messages.is_some() && self.window_images.is_some(),
            Capability::Input => self.input.is_some(),
            Capability::Buttons => self.buttons.is_some() && self.scene.is_some(),
            Capability::ButtonInput => {
                self.buttons.is_some() && self.scene.is_some() && self.input.is_some()
            }
            Capability::ListInput => {
                self.buttons.is_some()
                    && self.scene.is_some()
                    && self.input.is_some()
                    && self.gesture.is_some()
                    && self.list_input
            }
            Capability::DragPress => {
                self.gesture.is_some() && self.input.as_ref().is_some_and(|h| h.clock.is_some())
            }
            Capability::Scene | Capability::Quake | Capability::Strip => self.scene.is_some(),
            Capability::Camera => self.camera.is_some(),
            Capability::SceneImages | Capability::MaskImages => self.images.is_some(),
            Capability::ImageChange => self.images.as_ref().is_some_and(|h| h.change_images),
            Capability::Audio => self.audio.is_some(),
            Capability::Text => self.text.is_some(),
            Capability::Timers => self.timers.is_some(),
            Capability::Flags => self.flags.is_some(),
            Capability::Random => self.random.is_some(),
            Capability::Settings => self.settings.is_some(),
            Capability::History => self.history.is_some(),
            Capability::Strings => self.strings,
            Capability::Math => self.math,
            Capability::Gesture => self.gesture.is_some(),
            Capability::Popup => self.popup.is_some(),
            Capability::SaveTitle => self.save_title.is_some(),
            Capability::SaveInfo => self.save_info.is_some(),
            Capability::Effects => self.effects.is_some(),
            Capability::EffectObjects => self.scene.is_some(),
            Capability::ParticleImages => self.images.is_some() && self.random.is_some(),
            Capability::FadeWait
            | Capability::MaskWait
            | Capability::QuakeWait
            | Capability::StripWait => self.waits.as_ref().is_some_and(|h| h.scene.is_some()),
            Capability::ChangeWait => self
                .waits
                .as_ref()
                .is_some_and(|h| h.scene.is_some() && h.change_images),
            Capability::CameraWait => self.waits.as_ref().is_some_and(|h| h.camera.is_some()),
        }
    }
}

pub struct TextHosts<'a> {
    pub messages: &'a mut MessageHost,
    pub input: &'a mut ActionInput,
    pub flags: MessageFlags,
}

pub struct Call<'a, 'script> {
    pub vm: &'a mut Machine<'script>,
    pub pc: usize,
    pub command: u16,
    arguments: u64,
}
impl<'a, 'script> Call<'a, 'script> {
    pub(crate) fn new(vm: &'a mut Machine<'script>, pc: usize, command: u16) -> Self {
        Self {
            vm,
            pc,
            command,
            arguments: 0,
        }
    }

    pub fn argument(&mut self, distance: usize, name: &'static str) -> Result<u32, DecodeError> {
        if distance == 0 || !distance.is_multiple_of(4) || distance > 256 {
            return Err(self.error("invalid named argument distance"));
        }
        let value = self
            .vm
            .stack()
            .peek_u32(distance)
            .map_err(|_| error(self.pc, self.command + 0x2000, name))?;
        self.arguments |= 1 << (distance / 4 - 1);
        Ok(value)
    }
    /// Account for an unused parameter without requiring its bytes to be known.
    /// The word must still exist and agree with the command's stack cleanup.
    pub fn ignore_argument(
        &mut self,
        distance: usize,
        name: &'static str,
    ) -> Result<(), DecodeError> {
        if distance == 0 || !distance.is_multiple_of(4) || distance > 256 {
            return Err(self.error("invalid named argument distance"));
        }
        if self.vm.stack().len_bytes() < distance {
            return Err(self.error(name));
        }
        self.arguments |= 1 << (distance / 4 - 1);
        Ok(())
    }
    pub fn error(&self, why: &'static str) -> DecodeError {
        error(self.pc, self.command + 0x2000, why)
    }
    fn require_arguments(&self, bytes: usize) -> Result<(), DecodeError> {
        if bytes > 256 || !bytes.is_multiple_of(4) {
            return Err(self.error("invalid command argument byte count"));
        }
        let words = (bytes / 4) as u32;
        let expected = 1u64.checked_shl(words).unwrap_or(0).wrapping_sub(1);
        if self.arguments != expected {
            return Err(self.error("named arguments disagree with command stack cleanup"));
        }
        Ok(())
    }

    /// Transfers validate named parameters without completing the old instruction.
    /// Machine owns the return frame or main replacement and clears pending once.
    pub fn enter_common(
        &mut self,
        slot: u32,
        name: Vec<u8>,
        script: crate::ps3::Script,
    ) -> Result<(), DecodeError> {
        self.require_arguments(8)?;
        self.vm.enter_common(slot, name, script)
    }
    pub fn switch_main(
        &mut self,
        name: Vec<u8>,
        script: crate::ps3::Script,
    ) -> Result<(), DecodeError> {
        self.require_arguments(4)?;
        self.vm.switch_main_core(name, script)
    }
    pub fn call_script(&mut self, slot: u8, target: usize) -> Result<(), DecodeError> {
        self.require_arguments(4)?;
        self.vm.call_script(slot, target)
    }

    fn defer(&self, operation: RuntimeOperation, bytes: usize) -> Result<Flow, DecodeError> {
        if bytes > 20 {
            return Err(self.error("runtime request exceeds its parameter capacity"));
        }
        self.require_arguments(bytes)?;
        let mut values = [0; 5];
        for (i, value) in values.iter_mut().take(bytes / 4).enumerate() {
            *value = self
                .vm
                .stack()
                .peek_u32(4 * (i + 1))
                .map_err(|_| self.error("runtime argument is unknown"))?;
        }
        Ok(Flow::Runtime(RuntimeCall {
            pc: self.pc,
            command: self.command,
            arguments: self.arguments,
            operation,
            stack_len: self.vm.stack().len_bytes(),
            values,
        }))
    }
    pub fn complete(&mut self, flags: u16) -> Result<(), DecodeError> {
        self.finish(flags).map(|_| ())
    }

    pub fn finish(&mut self, flags: u16) -> Result<Flow, DecodeError> {
        self.require_arguments(SystemResult(flags).cleanup_bytes())?;
        let step = self.vm.complete_system(SystemResult(flags))?;
        Ok(Flow::from_step(step, false))
    }

    /// Validate the retained arguments before yielding without advancing PC or SP.
    pub fn wait(&mut self, bytes: usize) -> Result<Flow, DecodeError> {
        self.require_arguments(bytes)?;
        self.vm.yield_pending_system()?;
        Ok(Flow::Wait)
    }
}

type Handler = fn(&mut Hosts<'_>, &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>>;
pub struct Entry {
    pub command: u16,
    pub name: &'static str,
    pub capability: Capability,
    pub handler: Handler,
}

fn root_text(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_handle(call.vm)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}
fn root_output(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(
        match hosts
            .messages
            .try_record_output(call.vm, hosts.input, hosts.flags)?
        {
            Some(step) => Flow::from_step(step, call.vm.pending_system().is_some()),
            None => Flow::Unhandled,
        },
    )
}
fn root_poll(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_record_poll(call.vm, hosts.flags)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}

fn root_timers(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .timers
        .as_deref_mut()
        .expect("timers capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_flags(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .flags
        .as_deref_mut()
        .expect("flags capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_random(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .random
        .as_deref_mut()
        .expect("random capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_settings(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .settings
        .as_deref_mut()
        .expect("settings capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_history(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .history
        .as_deref_mut()
        .expect("history capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_strings(_hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    crate::text::handle(call)?;
    Ok(Flow::Done)
}

fn root_math(_hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    crate::math::handle(call)?;
    Ok(Flow::Done)
}

fn root_gesture(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .gesture
        .as_deref_mut()
        .expect("gesture capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_popup(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .popup
        .as_deref_mut()
        .expect("popup capability checked")
        .handle_popup(call)?;
    Ok(Flow::Done)
}

fn root_save_title(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .save_title
        .as_deref_mut()
        .expect("save_title capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_save_info(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .save_info
        .as_deref_mut()
        .expect("save_info capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_effects(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    crate::fade::handle_effects(
        call,
        hosts
            .effects
            .as_deref_mut()
            .expect("effects capability checked"),
    )?;
    Ok(Flow::Done)
}

fn root_effect_objects(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("effect objects capability checked")
        .release_effect(call)?;
    Ok(Flow::Done)
}

fn root_wait_fade(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .fade_mut()
        .handle_wait(call, h.gate, h.input)?)
}

fn root_wait_mask(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_mask(call, h.gate, h.input)?)
}

fn root_wait_quake(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_quake(call)?)
}

fn root_wait_strip(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_strip(call, h.gate, h.input)?)
}

fn root_wait_change(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_change(call, h.gate, h.input)?)
}

fn root_wait_camera(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.camera
        .as_deref_mut()
        .expect("camera wait capability checked")
        .handle_wait_move(call, h.gate, h.input)?)
}

fn root_audio(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.handle(call, h.backend)?;
    Ok(Flow::Done)
}

fn root_text_state(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("message capability checked")
        .handle_text_state(call)?;
    Ok(Flow::Done)
}
fn root_auxiliary_scene(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    Ok(hosts
        .messages
        .as_deref_mut()
        .expect("message capability checked")
        .handle_auxiliary_scene(call, hosts.scene.as_deref_mut())?)
}
pub fn uses_message_state(command: u16) -> bool {
    uses_windows(command)
        || ROOT.iter().any(|e| {
            e.command == command
                && matches!(
                    e.capability,
                    Capability::TextState | Capability::AuxiliaryScene | Capability::FontDialog
                )
        })
}

fn root_windows(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window(call)?;
    Ok(Flow::Done)
}
fn root_window_pointer(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_pointer(
            call,
            hosts
                .input
                .as_ref()
                .expect("input capability checked")
                .input,
        )?;
    Ok(Flow::Done)
}
fn root_window_buttons(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_buttons(
            call,
            hosts
                .input
                .as_mut()
                .expect("input capability checked")
                .input,
        )?;
    Ok(Flow::Done)
}
fn root_window_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_image(
            call,
            hosts
                .window_images
                .as_deref_mut()
                .expect("image source capability checked"),
        )?;
    Ok(Flow::Done)
}

pub fn uses_windows(command: u16) -> bool {
    ROOT.iter().any(|entry| {
        entry.command == command
            && matches!(
                entry.capability,
                Capability::Windows
                    | Capability::WindowPointer
                    | Capability::WindowButtons
                    | Capability::WindowImages
            )
    })
}
pub fn uses_window_images(command: u16) -> bool {
    ROOT.iter()
        .any(|entry| entry.command == command && entry.capability == Capability::WindowImages)
}

fn root_input(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    if call.command == 0x2ef {
        crate::input::handle_skipping(call, h.input, h.skip)?;
    } else {
        crate::input::handle(call, h.input)?;
    }
    Ok(Flow::Done)
}
fn root_buttons(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
        )?;
    Ok(Flow::Done)
}
fn root_button_input(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle_input(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
            h.input,
        )?;
    Ok(Flow::Done)
}
fn root_list_input(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle_list_input(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
            h.input,
            hosts
                .gesture
                .as_deref()
                .expect("gesture capability checked"),
        )?;
    Ok(Flow::Done)
}
fn root_drag_press(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .gesture
        .as_deref_mut()
        .expect("gesture capability checked")
        .handle_drag_press(call, h.input, h.clock.expect("clock capability checked"))?;
    Ok(Flow::Done)
}

pub fn uses_input(command: u16) -> bool {
    ROOT.iter().any(|entry| {
        entry.command == command
            && matches!(
                entry.capability,
                Capability::Input
                    | Capability::ButtonInput
                    | Capability::ListInput
                    | Capability::DragPress
                    | Capability::WindowPointer
                    | Capability::WindowButtons
            )
    })
}

fn root_runtime(_hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    use RuntimeOperation::*;
    let (operation, bytes) = match call.command {
        1 => (Quit, 0),
        3 => (Reset, 0),
        0x2b5 => (
            LoadSlot {
                slot: call.argument(4, "save slot")?,
            },
            4,
        ),
        0x2b6 => (
            SaveSlot {
                slot: call.argument(4, "save slot")?,
            },
            4,
        ),
        0x28c | 0x28e => (
            EditSlots {
                first: call.argument(4, "first slot")?,
                second: call.argument(8, "second slot")?,
            },
            8,
        ),
        0x2b8 => (Thumbnail, 0),
        0x2b7 => (
            LatestSlot {
                start: call.argument(4, "first slot")? as i32,
                count: call.argument(8, "slot count")? as i32,
            },
            8,
        ),
        0x2bd => {
            let (slot, object, child) = (
                call.argument(4, "save slot")? as i32,
                call.argument(8, "object slot")?,
                call.argument(12, "child slot")? as i32,
            );
            let label_slot = crate::text::bank_slot(call.argument(16, "label destination")?)
                .ok_or_else(|| {
                    call.error("label destination outside the string bank is not modeled")
                })?;
            let text_slot = crate::text::bank_slot(call.argument(20, "text destination")?)
                .ok_or_else(|| {
                    call.error("text destination outside the string bank is not modeled")
                })?;
            (
                SlotInfo {
                    slot,
                    object,
                    child,
                    label_slot,
                    text_slot,
                },
                20,
            )
        }
        _ => unreachable!("registered runtime command"),
    };
    Ok(call.defer(operation, bytes)?)
}
fn root_snapshot(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .snapshot
        .as_mut()
        .expect("snapshot capability checked");
    if h.capture {
        return Ok(if hosts.runtime {
            call.defer(RuntimeOperation::CaptureSnapshot, 0)?
        } else {
            Flow::Unavailable
        });
    }
    if !h.continue_without_image {
        return Ok(Flow::Unavailable);
    }
    h.image.handle_unavailable(call)?;
    Ok(Flow::Done)
}
fn root_message_wait(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .message_wait
        .as_mut()
        .expect("message wait capability checked");
    Ok(h.messages.handle_wait(call, h.input, h.flags, h.context)?)
}
pub fn is_message_wait(command: u16) -> bool {
    ROOT.iter()
        .any(|e| e.command == command && e.capability == Capability::MessageWait)
}
pub fn is_lifecycle(command: u16) -> bool {
    ROOT.iter().any(|e| {
        e.command == command && e.capability == Capability::Runtime && matches!(command, 1 | 3)
    })
}

fn root_script_control(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.scripts.as_mut().expect("script capability checked");
    Ok(Flow::Control(h.host.handle(call, h.source)?))
}
fn root_system_read(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .system_read
        .as_mut()
        .expect("system read capability checked");
    Ok(Flow::SystemSave(
        h.host.handle_read(call, h.source, h.game_id)?,
    ))
}
fn root_system_write(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .system_write
        .as_mut()
        .expect("system write capability checked");
    h.host.handle_write(call, h.sink, h.random, h.game_id)?;
    Ok(Flow::SystemWritten)
}
fn root_save_menu(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .popup
        .as_deref_mut()
        .expect("save menu checked")
        .handle(
            call,
            hosts
                .save_source
                .as_deref_mut()
                .expect("save source checked"),
        )?;
    Ok(Flow::Done)
}
fn root_bootstrap(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    Ok(hosts
        .bootstrap
        .as_deref_mut()
        .expect("bootstrap capability checked")
        .handle(call)?)
}
fn root_function_keys(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    match hosts
        .function_keys
        .as_mut()
        .expect("function key capability checked")
    {
        FunctionKeys::Startup(host) => Ok(host.handle(call)?),
        FunctionKeys::Input(input) => {
            crate::input::handle(call, input)?;
            Ok(Flow::Done)
        }
    }
}
fn root_platform(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .platform
        .as_mut()
        .expect("platform capability checked");
    Ok(Flow::Platform(
        h.host.handle(call, h.backend, h.assets, h.cursors)?,
    ))
}
/// 0x213d / 0x140081520 seeds the modal dialog with the global face (+3160).
/// An accepted face updates the setting and reselects only texts using the default;
/// cancellation still completes with no arguments or result writes.
fn root_font_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let settings = hosts
        .settings
        .as_deref_mut()
        .expect("font settings checked");
    let messages = hosts
        .messages
        .as_deref_mut()
        .expect("font messages checked");
    let chosen = hosts
        .device_frame
        .as_deref_mut()
        .expect("font backend checked")
        .choose_font(settings.font_face())?;
    let mut next = settings.clone();
    if let Some(face) = &chosen {
        next.set_font_face(face).map_err(|why| call.error(why))?;
    }
    call.complete(0x4000)?;
    if chosen.is_some() {
        *settings = next;
        messages.set_default_face(settings.font_face());
        messages.refresh_default_fonts();
    }
    Ok(Flow::Done)
}
fn root_capture(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .capture
        .as_deref_mut()
        .expect("capture capability checked")
        .handle(
            call,
            hosts.scene.as_deref_mut().expect("capture scene checked"),
        )?;
    Ok(Flow::Done)
}
fn root_capture_device(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .capture
        .as_deref_mut()
        .expect("capture capability checked")
        .handle_immediate(
            call,
            hosts.scene.as_deref_mut().expect("capture scene checked"),
            hosts
                .device_frame
                .as_deref_mut()
                .expect("capture device checked"),
        )?;
    Ok(Flow::Done)
}
/// Platform and capture backends must be borrowed independently of image sources.
pub fn uses_bootstrap(command: u16) -> bool {
    ROOT.iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::Bootstrap | Capability::FunctionKeys
            )
    })
}
pub fn uses_platform(command: u16) -> bool {
    ROOT.iter()
        .any(|e| e.command == command && e.capability == Capability::Platform)
}
pub fn uses_device_frame(command: u16) -> bool {
    ROOT.iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::CaptureDevice | Capability::FontDialog
            )
    })
}

fn root_sequence(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .sequence
        .as_mut()
        .expect("sequence capability checked");
    let context = h.execution.as_mut().map(|c| SequenceContext {
        scene: &mut *c.scene,
        images: &mut *c.images,
        camera: c.camera.as_deref_mut(),
    });
    h.host.handle(call, context)?;
    Ok(Flow::Done)
}
fn root_movie_state(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}
fn root_movie_start(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_start_scene(call)?;
    Ok(Flow::Done)
}
fn root_movie_scene(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_scene
        .as_mut()
        .expect("scene movie capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_create_scene(call, h.scene, h.source)?;
    Ok(Flow::Done)
}
fn root_movie_player(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_player
        .as_mut()
        .expect("movie player capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_player(call, h.source, h.settings, h.backend)?;
    Ok(Flow::Done)
}
pub fn uses_sequence(command: u16) -> bool {
    ROOT.iter()
        .any(|e| e.command == command && e.capability == Capability::Sequence)
}
pub fn uses_movie_scene(command: u16) -> bool {
    ROOT.iter()
        .any(|e| e.command == command && e.capability == Capability::MovieScene)
}
pub fn uses_movie_player(command: u16) -> bool {
    ROOT.iter()
        .any(|e| e.command == command && e.capability == Capability::MoviePlayer)
}

fn root_quake(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle_quake(call)?;
    Ok(Flow::Done)
}
fn root_strip(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle_strip(call)?;
    Ok(Flow::Done)
}
fn root_scene(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}
fn root_camera(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .camera
        .as_deref_mut()
        .expect("camera capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_images(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_image(call, h.source)?;
    Ok(Flow::Done)
}
fn root_mask_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_mask(call, h.source)?;
    Ok(Flow::Done)
}
fn root_change_image(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_change_image(call, h.source)?;
    Ok(Flow::Done)
}

pub fn uses_images(command: u16) -> bool {
    ROOT.iter()
        .chain(RESEARCH_TABLES.iter().flat_map(|table| table.iter()))
        .any(|e| {
            e.command == command
                && matches!(
                    e.capability,
                    Capability::SceneImages
                        | Capability::MaskImages
                        | Capability::ImageChange
                        | Capability::ParticleImages
                )
        })
}

pub fn is_wait(command: u16) -> bool {
    ROOT.iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::FadeWait
                    | Capability::MaskWait
                    | Capability::QuakeWait
                    | Capability::StripWait
                    | Capability::ChangeWait
                    | Capability::CameraWait
            )
    })
}

pub static ROOT: &[Entry] = &[
    Entry {
        command: 0x1,
        name: "quit runtime",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x3,
        name: "reset runtime",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x28c,
        name: "swap save slots",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x28e,
        name: "move save slot",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2b5,
        name: "load save slot",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2b6,
        name: "write save slot",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2b8,
        name: "build thumbnail",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2b7,
        name: "latest save slot",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2bd,
        name: "save slot information",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x2b2,
        name: "snapshot policy",
        capability: Capability::Snapshot,
        handler: root_snapshot,
    },
    Entry {
        command: 0x153,
        name: "message wait",
        capability: Capability::MessageWait,
        handler: root_message_wait,
    },
    Entry {
        command: 0x158,
        name: "timed message wait",
        capability: Capability::MessageWait,
        handler: root_message_wait,
    },
    Entry {
        command: 0x80,
        name: "replace main script",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x81,
        name: "enter common script",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x83,
        name: "script control words",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x88,
        name: "register callback",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x89,
        name: "automatic callbacks",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x8a,
        name: "call callback",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x8b,
        name: "disable callback",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x8c,
        name: "suspend callbacks",
        capability: Capability::ScriptControl,
        handler: root_script_control,
    },
    Entry {
        command: 0x128,
        name: "read system save",
        capability: Capability::SystemRead,
        handler: root_system_read,
    },
    Entry {
        command: 0x129,
        name: "write system save",
        capability: Capability::SystemWrite,
        handler: root_system_write,
    },
    Entry {
        command: 0x2b3,
        name: "scan save menu",
        capability: Capability::SaveMenu,
        handler: root_save_menu,
    },
    Entry {
        command: 0x2b4,
        name: "second save group",
        capability: Capability::SaveMenu,
        handler: root_save_menu,
    },
    Entry {
        command: 0x2bb,
        name: "save signature",
        capability: Capability::SaveMenu,
        handler: root_save_menu,
    },
    Entry {
        command: 0x10,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x11,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x12,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x13,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x14,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x15,
        name: "register resource directory",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x16,
        name: "register resource directory",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x19,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x1a,
        name: "register resource path",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x1d,
        name: "startup setting",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x1e,
        name: "startup setting",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x1f,
        name: "startup setting",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x90,
        name: "startup setting",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x91,
        name: "initialize bits",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x92,
        name: "initialize integers",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x93,
        name: "initialize floats",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x94,
        name: "initialize strings",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x98,
        name: "register cursor",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x34d,
        name: "function key policy",
        capability: Capability::FunctionKeys,
        handler: root_function_keys,
    },
    Entry {
        command: 0x8f,
        name: "icon",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x99,
        name: "cursor",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x160,
        name: "window title",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x161,
        name: "scene title",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x163,
        name: "fullscreen query",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x164,
        name: "fullscreen set",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x16b,
        name: "shader capability",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x16c,
        name: "render mode query",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x16d,
        name: "render mode set",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x16e,
        name: "reset window size",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x16f,
        name: "resizable",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2b0,
        name: "thumbnail config",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2c8,
        name: "stretch filter",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2c9,
        name: "aspect scaling set",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2ca,
        name: "aspect scaling query",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2fc,
        name: "aspect mode query",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2fd,
        name: "aspect mode set",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x2ff,
        name: "display selection",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x13d,
        name: "font dialog",
        capability: Capability::FontDialog,
        handler: root_font_dialog,
    },
    Entry {
        command: 0x3c,
        name: "give screen capture",
        capability: Capability::Capture,
        handler: root_capture,
    },
    Entry {
        command: 0x304,
        name: "request screen capture",
        capability: Capability::Capture,
        handler: root_capture,
    },
    Entry {
        command: 0x305,
        name: "release screen capture",
        capability: Capability::Capture,
        handler: root_capture,
    },
    Entry {
        command: 0x306,
        name: "screen capture present",
        capability: Capability::Capture,
        handler: root_capture,
    },
    Entry {
        command: 0x3e,
        name: "read device frame",
        capability: Capability::CaptureDevice,
        handler: root_capture_device,
    },
    Entry {
        command: 0x280,
        name: "reset sequence",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x281,
        name: "sequence index",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x282,
        name: "start sequence",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x283,
        name: "stop sequence",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x284,
        name: "finish sequence",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x285,
        name: "sequence count",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x286,
        name: "append sequence wait",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x287,
        name: "append camera motion",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x288,
        name: "append image replacement",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x289,
        name: "append snapping world motion",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x28a,
        name: "append world motion",
        capability: Capability::Sequence,
        handler: root_sequence,
    },
    Entry {
        command: 0x308,
        name: "create scene movie",
        capability: Capability::MovieScene,
        handler: root_movie_scene,
    },
    Entry {
        command: 0x309,
        name: "start scene movie",
        capability: Capability::MovieStart,
        handler: root_movie_start,
    },
    Entry {
        command: 0x30a,
        name: "destroy scene movie",
        capability: Capability::MovieState,
        handler: root_movie_state,
    },
    Entry {
        command: 0x30b,
        name: "resume scene movie",
        capability: Capability::MovieState,
        handler: root_movie_state,
    },
    Entry {
        command: 0x30c,
        name: "pause scene movie",
        capability: Capability::MovieState,
        handler: root_movie_state,
    },
    Entry {
        command: 0x31a,
        name: "scene movie active",
        capability: Capability::MovieState,
        handler: root_movie_state,
    },
    Entry {
        command: 0xc0,
        name: "open movie",
        capability: Capability::MoviePlayer,
        handler: root_movie_player,
    },
    Entry {
        command: 0xc1,
        name: "stop movie",
        capability: Capability::MoviePlayer,
        handler: root_movie_player,
    },
    Entry {
        command: 0xc2,
        name: "movie frame",
        capability: Capability::MoviePlayer,
        handler: root_movie_player,
    },
    Entry {
        command: 0x20c,
        name: "quake paused",
        capability: Capability::Quake,
        handler: root_quake,
    },
    Entry {
        command: 0x225,
        name: "quake create",
        capability: Capability::Quake,
        handler: root_quake,
    },
    Entry {
        command: 0x226,
        name: "quake remaining",
        capability: Capability::Quake,
        handler: root_quake,
    },
    Entry {
        command: 0x227,
        name: "quake pause",
        capability: Capability::Quake,
        handler: root_quake,
    },
    Entry {
        command: 0x229,
        name: "quake release",
        capability: Capability::Quake,
        handler: root_quake,
    },
    Entry {
        command: 0x22f,
        name: "strip create",
        capability: Capability::Strip,
        handler: root_strip,
    },
    Entry {
        command: 0x230,
        name: "strip finish",
        capability: Capability::Strip,
        handler: root_strip,
    },
    Entry {
        command: 0x231,
        name: "strip release",
        capability: Capability::Strip,
        handler: root_strip,
    },
    Entry {
        command: 0x232,
        name: "strip remaining",
        capability: Capability::Strip,
        handler: root_strip,
    },
    Entry {
        command: 0x100,
        name: "create text object",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x101,
        name: "release text object",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x102,
        name: "text position",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x103,
        name: "text area",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x104,
        name: "clear text",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x105,
        name: "text glyph geometry",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x106,
        name: "text font face",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x107,
        name: "text colors",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x10c,
        name: "text pen",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x10d,
        name: "text reveal",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x10e,
        name: "text delay",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x10f,
        name: "text decoration",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x110,
        name: "text layer",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x112,
        name: "text layout query",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x113,
        name: "text outline mode",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x114,
        name: "text ruby enabled",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x115,
        name: "text ruby size",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x116,
        name: "text ruby spacing",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x117,
        name: "text ruby every line",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x119,
        name: "encoded text length",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x11a,
        name: "text glyph parameter",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x11e,
        name: "text line break rules",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x11f,
        name: "text measurement",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x380,
        name: "create auxiliary text",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x382,
        name: "release auxiliary text",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x385,
        name: "auxiliary layout",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x386,
        name: "clear auxiliary text",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x387,
        name: "auxiliary text parameters",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x389,
        name: "auxiliary colors",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x38c,
        name: "auxiliary pen",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x38e,
        name: "auxiliary gradient",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x38f,
        name: "auxiliary layer",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x390,
        name: "auxiliary measurement",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x392,
        name: "auxiliary outline mode",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x393,
        name: "auxiliary outline color",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x396,
        name: "auxiliary drawing bounds",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x398,
        name: "auxiliary position query",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x39a,
        name: "auxiliary position bounds",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x39b,
        name: "auxiliary moving",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x39c,
        name: "move auxiliary text",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x39e,
        name: "clamp auxiliary position",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x39f,
        name: "auxiliary eased move",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x3a0,
        name: "stop auxiliary motion",
        capability: Capability::TextState,
        handler: root_text_state,
    },
    Entry {
        command: 0x383,
        name: "auxiliary position",
        capability: Capability::AuxiliaryScene,
        handler: root_auxiliary_scene,
    },
    Entry {
        command: 0x39d,
        name: "bind auxiliary scene",
        capability: Capability::AuxiliaryScene,
        handler: root_auxiliary_scene,
    },
    Entry {
        command: 0xec,
        name: "window text scale",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x140,
        name: "create window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x141,
        name: "destroy window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x142,
        name: "window text layer",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x143,
        name: "window glyph size",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x144,
        name: "window text spacing",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x145,
        name: "window text area",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x146,
        name: "window text colors",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x147,
        name: "window character delay",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x148,
        name: "window reveal settings",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x149,
        name: "window decoration",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14a,
        name: "clear window text",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14b,
        name: "window exists",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14c,
        name: "hide window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14d,
        name: "window hidden",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14e,
        name: "show window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x14f,
        name: "window text pen",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x150,
        name: "move window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x151,
        name: "window cursor",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x155,
        name: "window glyph parameter",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x15a,
        name: "window alternate position",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x15b,
        name: "window position switch",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x15d,
        name: "create logical window",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x171,
        name: "release window image",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x178,
        name: "create window child",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x179,
        name: "destroy window child",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x17a,
        name: "window object visibility",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x17b,
        name: "window object exists",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x17c,
        name: "window object logical size",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x17f,
        name: "create window draw",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x180,
        name: "reset window draw",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x182,
        name: "window source rectangle",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x183,
        name: "window object position",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x184,
        name: "window object layer",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x185,
        name: "window object origin",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x186,
        name: "window object opacity",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x18d,
        name: "window draw query",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x190,
        name: "window button state image",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x191,
        name: "window button hit rectangle",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x192,
        name: "enable window button",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x195,
        name: "window button enabled",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x196,
        name: "set window button state",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x197,
        name: "window button state",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1cf,
        name: "create window animation",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1d0,
        name: "window animation frame",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1d1,
        name: "start window animation",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x15f,
        name: "window pointer hit",
        capability: Capability::WindowPointer,
        handler: root_window_pointer,
    },
    Entry {
        command: 0x194,
        name: "window button input",
        capability: Capability::WindowButtons,
        handler: root_window_buttons,
    },
    Entry {
        command: 0x170,
        name: "load window image",
        capability: Capability::WindowImages,
        handler: root_window_images,
    },
    Entry {
        command: 0x397,
        name: "auxiliary second texture",
        capability: Capability::WindowImages,
        handler: root_window_images,
    },
    Entry {
        command: 0xc8,
        name: "pointer position",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0xc9,
        name: "pointer moved",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0xca,
        name: "pointer rectangle",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x19a,
        name: "clear input edges",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x19c,
        name: "query input action 15",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x19d,
        name: "consume input action 15",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x19e,
        name: "query input action 16",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x19f,
        name: "consume input action 16",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a0,
        name: "query input action 0",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a1,
        name: "consume input action 0",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a2,
        name: "query input action 1",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a3,
        name: "consume input action 1",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a4,
        name: "query input action 7",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a6,
        name: "query input action 8",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a8,
        name: "query input action 6",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1a9,
        name: "consume input action 6",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1aa,
        name: "query input action 9",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1ab,
        name: "consume input action 9",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1ac,
        name: "query input action 10",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1ad,
        name: "consume input action 10",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1ae,
        name: "query input action 11",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1af,
        name: "consume input action 11",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b0,
        name: "query input action 12",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b1,
        name: "consume input action 12",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b2,
        name: "query input action 13",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b3,
        name: "consume input action 13",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b4,
        name: "query input action 14",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b5,
        name: "consume input action 14",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b6,
        name: "query input action 2",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b7,
        name: "consume input action 2",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b8,
        name: "query input action 3",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1b9,
        name: "consume input action 3",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1ba,
        name: "query input action 4",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1bb,
        name: "consume input action 4",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1bc,
        name: "query input action 5",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x1bd,
        name: "consume input action 5",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x344,
        name: "query input action 22",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x345,
        name: "consume input action 22",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x346,
        name: "query input action 23",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x347,
        name: "consume input action 23",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x34a,
        name: "query input action 20",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x34b,
        name: "consume input action 20",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x34e,
        name: "query function key",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x34f,
        name: "clear function key",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x35c,
        name: "left press",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x35d,
        name: "right press",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x2ef,
        name: "query skipping",
        capability: Capability::Input,
        handler: root_input,
    },
    Entry {
        command: 0x87,
        name: "merge button states",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x210,
        name: "create button group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x211,
        name: "release button group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x212,
        name: "append button",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x213,
        name: "button rectangle",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x214,
        name: "button state image",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x215,
        name: "show buttons",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x21b,
        name: "create scaled button group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x370,
        name: "create list group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x371,
        name: "release list group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x372,
        name: "append list button",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x373,
        name: "list button rectangle",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x374,
        name: "list button state image",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x375,
        name: "show list buttons",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x376,
        name: "current list button",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x378,
        name: "deactivate list group",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x379,
        name: "list navigation links",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x37c,
        name: "list keep input",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x37d,
        name: "list hit area",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x37e,
        name: "list disable navigation",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x37f,
        name: "list hit point",
        capability: Capability::Buttons,
        handler: root_buttons,
    },
    Entry {
        command: 0x216,
        name: "current button",
        capability: Capability::ButtonInput,
        handler: root_button_input,
    },
    Entry {
        command: 0x217,
        name: "update buttons",
        capability: Capability::ButtonInput,
        handler: root_button_input,
    },
    Entry {
        command: 0x21a,
        name: "select button",
        capability: Capability::ButtonInput,
        handler: root_button_input,
    },
    Entry {
        command: 0x377,
        name: "update list buttons",
        capability: Capability::ListInput,
        handler: root_list_input,
    },
    Entry {
        command: 0x369,
        name: "drag press",
        capability: Capability::DragPress,
        handler: root_drag_press,
    },
    Entry {
        command: 0x20,
        name: "create object",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x21,
        name: "destroy object",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x22,
        name: "create child",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x23,
        name: "destroy child",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x26,
        name: "object visibility",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x27,
        name: "object exists",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x28,
        name: "move object slot",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2a,
        name: "object logical size",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2d,
        name: "attach child queue",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2e,
        name: "clear object flag",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2f,
        name: "destroy all objects",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x33,
        name: "image information",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x34,
        name: "image pixel",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x40,
        name: "create draw state",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x42,
        name: "draw coordinate mode",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x43,
        name: "draw solid fill",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x44,
        name: "draw source rectangle",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x45,
        name: "draw position",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x46,
        name: "draw origin",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x47,
        name: "draw layer",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x48,
        name: "draw opacity",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x49,
        name: "draw scale",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x4a,
        name: "draw rotation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x4b,
        name: "draw blend mode",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x4c,
        name: "draw camera mode",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x50,
        name: "create child draw state",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x70,
        name: "draw reference depth",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x71,
        name: "draw vertical offset",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x72,
        name: "draw world x",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x73,
        name: "draw world y",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x74,
        name: "draw world z",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x75,
        name: "draw depth bias",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x78,
        name: "world position query",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e0,
        name: "create scale motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e1,
        name: "set key scale motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e2,
        name: "start scale motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e3,
        name: "stop scale motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e4,
        name: "query scale motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e5,
        name: "create rotation track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e6,
        name: "set key rotation track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e7,
        name: "start rotation track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e8,
        name: "stop rotation track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1e9,
        name: "query rotation track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1ea,
        name: "create opacity track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1eb,
        name: "set key opacity track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1ec,
        name: "start opacity track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1ed,
        name: "stop opacity track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1ee,
        name: "query opacity track",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1ef,
        name: "create frame animation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f0,
        name: "set key frame animation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f1,
        name: "start frame animation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f2,
        name: "stop frame animation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f3,
        name: "query frame animation",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f4,
        name: "create position motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f5,
        name: "set key position motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f6,
        name: "start position motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f7,
        name: "stop position motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f8,
        name: "query position motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1f9,
        name: "create world motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1fa,
        name: "set key world motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1fb,
        name: "start world motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1fc,
        name: "stop world motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x1fd,
        name: "query world motion",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x220,
        name: "create fade",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x221,
        name: "release fade",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x222,
        name: "finish fade",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x223,
        name: "fade remaining",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x250,
        name: "clear depth",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2c5,
        name: "cancel image change",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2c6,
        name: "image change active",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x2cb,
        name: "clear color",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x321,
        name: "second texture mode",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x322,
        name: "second texture subtraction",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x329,
        name: "reset object effects",
        capability: Capability::Scene,
        handler: root_scene,
    },
    Entry {
        command: 0x56,
        name: "project coordinates",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x58,
        name: "set camera geometry",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x59,
        name: "set camera viewport",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5a,
        name: "set camera position",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5b,
        name: "set camera angle",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5c,
        name: "set camera depth offset",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5d,
        name: "set camera center",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5e,
        name: "set camera enabled",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x5f,
        name: "set camera type",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x60,
        name: "set camera geometry",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x61,
        name: "set camera viewport",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x62,
        name: "set camera position",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x63,
        name: "set camera angle",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x64,
        name: "set camera depth offset",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x65,
        name: "set camera center",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x66,
        name: "set camera snapshot mode",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x67,
        name: "set camera scale",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x68,
        name: "stop camera move",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x6a,
        name: "camera moving",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x6b,
        name: "move camera XZ",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x6c,
        name: "move camera XYZ",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x6d,
        name: "move camera Y",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x9c,
        name: "camera effect",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x126,
        name: "screen size",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x260,
        name: "view position",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x261,
        name: "view angle x",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x262,
        name: "view angle y",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x263,
        name: "view angle z",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x264,
        name: "view field of view",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x30,
        name: "scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
    Entry {
        command: 0x35,
        name: "scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
    Entry {
        command: 0x38,
        name: "scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
    Entry {
        command: 0x3b,
        name: "scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
    Entry {
        command: 0x3f,
        name: "scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
    Entry {
        command: 0x22a,
        name: "mask transition",
        capability: Capability::MaskImages,
        handler: root_mask_images,
    },
    Entry {
        command: 0x22d,
        name: "mask transition",
        capability: Capability::MaskImages,
        handler: root_mask_images,
    },
    Entry {
        command: 0x234,
        name: "mask transition",
        capability: Capability::MaskImages,
        handler: root_mask_images,
    },
    Entry {
        command: 0x237,
        name: "mask transition",
        capability: Capability::MaskImages,
        handler: root_mask_images,
    },
    Entry {
        command: 0x2c4,
        name: "replace image",
        capability: Capability::ImageChange,
        handler: root_change_image,
    },
    Entry {
        command: 0xa0,
        name: "play BGM",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa1,
        name: "stop BGM",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa2,
        name: "fade BGM",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa4,
        name: "play BGM pair",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa6,
        name: "play voice",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa8,
        name: "stop voice",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xa9,
        name: "fade voice",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xaa,
        name: "voice position",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xaf,
        name: "play voice with rate",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb0,
        name: "play SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb1,
        name: "start SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb3,
        name: "stop SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb5,
        name: "stop all SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb6,
        name: "fade SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xb7,
        name: "SE position",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0xba,
        name: "fade in SE",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x238,
        name: "set ducking percent",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x239,
        name: "get ducking percent",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23a,
        name: "set ducking fade",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23b,
        name: "get ducking fade",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23c,
        name: "set ducking wait",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23d,
        name: "get ducking wait",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23e,
        name: "set ducking use",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x23f,
        name: "get ducking use",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x241,
        name: "stop channel 15",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2d6,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2d7,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2d8,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2d9,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2da,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2db,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2dc,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x2dd,
        name: "audio settings",
        capability: Capability::Audio,
        handler: root_audio,
    },
    Entry {
        command: 0x224,
        name: "wait fade",
        capability: Capability::FadeWait,
        handler: root_wait_fade,
    },
    Entry {
        command: 0x22b,
        name: "wait first mask",
        capability: Capability::MaskWait,
        handler: root_wait_mask,
    },
    Entry {
        command: 0x235,
        name: "wait second mask",
        capability: Capability::MaskWait,
        handler: root_wait_mask,
    },
    Entry {
        command: 0x228,
        name: "wait quake",
        capability: Capability::QuakeWait,
        handler: root_wait_quake,
    },
    Entry {
        command: 0x233,
        name: "wait strip wipe",
        capability: Capability::StripWait,
        handler: root_wait_strip,
    },
    Entry {
        command: 0x2c7,
        name: "wait image change",
        capability: Capability::ChangeWait,
        handler: root_wait_change,
    },
    Entry {
        command: 0x69,
        name: "wait camera move",
        capability: Capability::CameraWait,
        handler: root_wait_camera,
    },
    Entry {
        command: 0x350,
        name: "gesture field 0",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x351,
        name: "gesture paired flags",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x352,
        name: "gesture field 8",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x353,
        name: "gesture flick direction",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x356,
        name: "gesture field 12",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x357,
        name: "gesture field 16",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x358,
        name: "gesture field 20",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x359,
        name: "gesture result pair",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x35a,
        name: "gesture field 36",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x35b,
        name: "gesture field 32",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x360,
        name: "gesture hold result",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x362,
        name: "gesture held query",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x364,
        name: "gesture mode",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x365,
        name: "drag state query",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x366,
        name: "gesture region",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x367,
        name: "clear gesture field 172",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x368,
        name: "gesture release results",
        capability: Capability::Gesture,
        handler: root_gesture,
    },
    Entry {
        command: 0x2cd,
        name: "popup enabled setter",
        capability: Capability::Popup,
        handler: root_popup,
    },
    Entry {
        command: 0x2ce,
        name: "popup enabled query",
        capability: Capability::Popup,
        handler: root_popup,
    },
    Entry {
        command: 0x2d3,
        name: "first confirmation setting",
        capability: Capability::Popup,
        handler: root_popup,
    },
    Entry {
        command: 0x2d4,
        name: "second confirmation setting",
        capability: Capability::Popup,
        handler: root_popup,
    },
    Entry {
        command: 0x2ee,
        name: "second menu group query",
        capability: Capability::Popup,
        handler: root_popup,
    },
    Entry {
        command: 0x162,
        name: "save title",
        capability: Capability::SaveTitle,
        handler: root_save_title,
    },
    Entry {
        command: 0x2b9,
        name: "first save header word",
        capability: Capability::SaveInfo,
        handler: root_save_info,
    },
    Entry {
        command: 0x2ba,
        name: "remaining save header words",
        capability: Capability::SaveInfo,
        handler: root_save_info,
    },
    Entry {
        command: 0x2d1,
        name: "effects setting",
        capability: Capability::Effects,
        handler: root_effects,
    },
    Entry {
        command: 0x2a0,
        name: "release effect object",
        capability: Capability::EffectObjects,
        handler: root_effect_objects,
    },
    Entry {
        command: 0xd0,
        name: "integer absolute",
        capability: Capability::Math,
        handler: root_math,
    },
    Entry {
        command: 0xd1,
        name: "integer clamp",
        capability: Capability::Math,
        handler: root_math,
    },
    Entry {
        command: 0xd2,
        name: "integer maximum",
        capability: Capability::Math,
        handler: root_math,
    },
    Entry {
        command: 0xd3,
        name: "integer minimum",
        capability: Capability::Math,
        handler: root_math,
    },
    Entry {
        command: 0xf1,
        name: "compare strings",
        capability: Capability::Strings,
        handler: root_strings,
    },
    Entry {
        command: 0xf3,
        name: "substring",
        capability: Capability::Strings,
        handler: root_strings,
    },
    Entry {
        command: 0xf8,
        name: "string byte length",
        capability: Capability::Strings,
        handler: root_strings,
    },
    Entry {
        command: 0xfc,
        name: "format string",
        capability: Capability::Strings,
        handler: root_strings,
    },
    Entry {
        command: 0x12f,
        name: "clear history",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x130,
        name: "history count",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x131,
        name: "history string 1",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x132,
        name: "history string 2",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x133,
        name: "history string 0",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x134,
        name: "history string 3",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x135,
        name: "history word",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x136,
        name: "append history",
        capability: Capability::History,
        handler: root_history,
    },
    Entry {
        command: 0x2d5,
        name: "active drawing",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2de,
        name: "font face",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2df,
        name: "text outline",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e0,
        name: "text bold",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e1,
        name: "text speed",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e2,
        name: "text reveal mode",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e3,
        name: "text reveal duration",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e4,
        name: "read skip",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e6,
        name: "automatic voice sync",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e7,
        name: "automatic base delay",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e8,
        name: "automatic glyph delay",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2e9,
        name: "cursor auto clear",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x2f8,
        name: "automatic voice wait",
        capability: Capability::Settings,
        handler: root_settings,
    },
    Entry {
        command: 0x8,
        name: "frame clock",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0x9,
        name: "frame delta",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xb,
        name: "reset timer",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xc,
        name: "query timer",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xe,
        name: "mark stopwatch",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xf,
        name: "query stopwatch",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xd4,
        name: "random remainder",
        capability: Capability::Random,
        handler: root_random,
    },
    Entry {
        command: 0x2d0,
        name: "voice cut mode",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x2e5,
        name: "automatic message advance",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x2eb,
        name: "message skip",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x340,
        name: "message bypass",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x10b,
        name: "direct text",
        capability: Capability::Text,
        handler: root_text,
    },
    Entry {
        command: 0x152,
        name: "append window",
        capability: Capability::Text,
        handler: root_output,
    },
    Entry {
        command: 0x154,
        name: "direct window",
        capability: Capability::Text,
        handler: root_text,
    },
    Entry {
        command: 0x157,
        name: "append window",
        capability: Capability::Text,
        handler: root_output,
    },
    Entry {
        command: 0x159,
        name: "poll window",
        capability: Capability::Text,
        handler: root_poll,
    },
    Entry {
        command: 0x38b,
        name: "auxiliary text",
        capability: Capability::Text,
        handler: root_text,
    },
];

/// Research-only overrides: all selected commands remain Unreviewed in the
/// bound profile. This is not a registered Reviewed/ContractEquivalent variant.
pub static FONT_CELLS: &[Entry] = &[
    Entry {
        command: 0x10b,
        name: "font-cell direct text",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::direct_text(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x11d,
        name: "pixel text measurement",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::measure_text(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x152,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::append_window(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x154,
        name: "font-cell direct window",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::direct_window(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x157,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::append_window(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x159,
        name: "font-cell poll window",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::poll_window(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x38b,
        name: "font-cell auxiliary text",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::auxiliary_text(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
    Entry {
        command: 0x3ac,
        name: "font-cell mode",
        capability: Capability::Text,
        handler: |hosts, call| {
            Ok(font_cells::set_mode(
                hosts.text.as_mut().expect("text capability checked"),
                call,
            )?)
        },
    },
];

pub static PARTICLES: &[Entry] = &[Entry {
    command: 0x2a5,
    name: "create particle effect",
    capability: Capability::ParticleImages,
    handler: |hosts, call| {
        let h = hosts
            .images
            .as_mut()
            .expect("particle image capability checked");
        h.scene.create_particles(
            call,
            h.source,
            hosts
                .random
                .as_deref_mut()
                .expect("particle random capability checked"),
        )?;
        Ok(Flow::Done)
    },
}];

pub static FALLING_BILLBOARDS: &[Entry] = &[Entry {
    command: 0x2a2,
    name: "create falling billboard effect",
    capability: Capability::ParticleImages,
    handler: |hosts, call| {
        let h = hosts
            .images
            .as_mut()
            .expect("particle image capability checked");
        h.scene.create_falling_effect(
            call,
            h.source,
            hosts
                .random
                .as_deref_mut()
                .expect("particle random capability checked"),
        )?;
        Ok(Flow::Done)
    },
}];

pub static BACKGROUND_AUDIO: &[Entry] = &[Entry {
    command: 0x2ed,
    name: "background audio output",
    capability: Capability::Audio,
    handler: |hosts, call| {
        let h = hosts.audio.as_mut().expect("audio capability checked");
        h.audio.handle_background_output(call, h.backend)?;
        Ok(Flow::Done)
    },
}];

/// Static research tables used when provisioning shared host capabilities.
pub static RESEARCH_TABLES: &[&[Entry]] =
    &[FONT_CELLS, PARTICLES, BACKGROUND_AUDIO, FALLING_BILLBOARDS];

pub fn research_entry(id: crate::profile::ResearchId, command: u16) -> Option<&'static Entry> {
    let entries = match id {
        crate::profile::ResearchId::FontCells => FONT_CELLS,
        crate::profile::ResearchId::Particles => PARTICLES,
        crate::profile::ResearchId::BackgroundAudio => BACKGROUND_AUDIO,
        crate::profile::ResearchId::FallingBillboards => FALLING_BILLBOARDS,
    };
    entries.iter().find(|entry| entry.command == command)
}

pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
    match profile.semantics(command) {
        Semantics::Research(crate::profile::ResearchId::FontCells) => "research text override",
        Semantics::Research(crate::profile::ResearchId::Particles) => "research particle override",
        Semantics::Research(crate::profile::ResearchId::BackgroundAudio) => {
            "research background audio override"
        }
        Semantics::Research(crate::profile::ResearchId::FallingBillboards) => {
            "research falling billboard override"
        }
        Semantics::Root => "base semantics",
    }
}

/// Called after CommandGate::resolve. None means no pending command or no root
/// entry. Unavailable preserves the pending command and arguments; the
/// caller may report the missing capability, but must not try another handler.
pub fn run(
    semantics: Semantics,
    vm: &mut Machine<'_>,
    hosts: &mut Hosts<'_>,
) -> Result<Option<Flow>, DispatchError> {
    let Some((pc, command)) = vm.pending_system() else {
        return Ok(None);
    };
    let entry = match semantics {
        Semantics::Research(id) => research_entry(id, command),
        Semantics::Root => ROOT.iter().find(|e| e.command == command),
    };
    let Some(entry) = entry else {
        return Ok((semantics != Semantics::Root).then_some(Flow::Unhandled));
    };
    if !hosts.has(entry.capability) {
        return Ok(Some(Flow::Unavailable));
    }
    (entry.handler)(hosts, &mut Call::new(vm, pc, command))
        .map(Some)
        .map_err(|cause| DispatchError { pc, command, cause })
}

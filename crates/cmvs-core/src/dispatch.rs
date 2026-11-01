//! Incremental command-table migration. Root entries share one dispatch path;
//! text research overrides are sparse. Other commands still use the legacy chain. Gate checks run
//! before this table. A selected override never falls through to root semantics.
pub mod ownership;

use std::{error::Error, fmt};

use crate::{
    audio::{AudioBackend, AudioHost},
    buttons::ButtonHost,
    bytecode::DecodeError,
    camera::CameraHost,
    evaluate::error,
    fade::SkipGate,
    gesture::PointerGesture,
    history::HistoryHost,
    image::ImageSource,
    input::ActionInput,
    machine::{Machine, Step},
    message::{MessageHost, font_cells},
    message_flags::MessageFlags,
    profile::{EngineProfile, Semantics},
    random::CrtRandom,
    save_menu::SaveMenuHost,
    scene::SceneHost,
    settings::SettingsHost,
    snapshot::{SaveInfo, SaveTitle},
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
        matches!(self, Self::Yield | Self::Wait)
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

#[derive(Default)]
pub struct Hosts<'a> {
    pub windows: Option<&'a mut MessageHost>,
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
    pub effect_objects: bool,
    /// Availability of stateless services in this host environment.
    pub strings: bool,
    pub math: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
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
            Capability::Windows => self.windows.is_some(),
            Capability::WindowPointer | Capability::WindowButtons => {
                self.windows.is_some() && self.input.is_some()
            }
            Capability::WindowImages => self.windows.is_some() && self.window_images.is_some(),
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
            Capability::Scene => self.scene.is_some(),
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
            Capability::EffectObjects => self.effect_objects,
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
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    crate::effect_objects::handle(call)?;
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

fn root_windows(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>> {
    hosts
        .windows
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
        .windows
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
        .windows
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
        .windows
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
    ROOT.iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::SceneImages | Capability::MaskImages | Capability::ImageChange
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

pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
    if profile.semantics(command) == Semantics::ResearchFontCells {
        "research text override"
    } else {
        "base semantics"
    }
}

/// Called after CommandGate::resolve. None delegates an unmigrated command to the
/// legacy chain. Unavailable preserves the pending command and arguments; the
/// caller may report the missing capability, but must not try another handler.
pub fn run(
    semantics: Semantics,
    vm: &mut Machine<'_>,
    hosts: &mut Hosts<'_>,
) -> Result<Option<Flow>, DispatchError> {
    let Some((pc, command)) = vm.pending_system() else {
        return Ok(None);
    };
    let specific = if semantics == Semantics::ResearchFontCells {
        FONT_CELLS
    } else {
        &[]
    }
    .iter()
    .find(|e| e.command == command);
    let entry = if semantics == Semantics::ResearchFontCells {
        specific
    } else {
        ROOT.iter().find(|e| e.command == command)
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

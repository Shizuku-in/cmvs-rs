//! Shared static system-command routing and sparse identity-bound variant overrides.
//! Gate resolution precedes dispatch. Runtime-wide operations return a checked
//! request for the caller's orchestration; they remain pending until acknowledged.
//! A selected override never falls through to root semantics.
//!
//! Custom executors use [`run`] with [`Hosts`] after [`crate::profile::CommandGate`]
//! resolves the command. Complete hosts should use [`crate::runtime::Runtime`],
//! which performs this sequence and handles runtime requests.
//!
//! [`Entry`], [`ROOT`], [`VARIANT_TABLES`], and [`variant_entry`] expose registration
//! metadata for inspection. Handlers and their argument bookkeeping are internal;
//! metadata alone does not authorize a command or establish host capabilities.
mod call;
mod handlers;
mod hosts;
pub mod ownership;
mod registry;

pub(crate) use call::Call;
pub use call::{RuntimeCall, RuntimeOperation};
pub use hosts::{
    AudioHosts, Capability, FunctionKeys, Hosts, ImageHosts, InputHosts, MessageWaitHosts,
    MoviePlayerHosts, MovieSceneHosts, PlatformHosts, ScriptHosts, SequenceHosts, SnapshotHosts,
    SystemReadHosts, SystemWriteHosts, TextHosts, WaitHosts,
};
pub(crate) use registry::root_entry;
pub use registry::{
    ACTIVATION_PASS, AUDIO_PRELOAD, BACKGROUND_AUDIO, CAMERA_MOVE_X, COLLISION_PARTICLES,
    DISPLAY_COUNT, DRAW_QUERY, FADING_PARTICLES, FALLING_BILLBOARDS, FALLING_BILLBOARDS_X87,
    FONT_CELLS, FRAGMENTS, FULLSCREEN_MONITOR, IMAGE_TABLES, INPUT_PRESS, KEYED_ARCHIVES,
    LEGACY_EFFECTS, LIST_SELECT, LOGICAL_TEXT, MGV_PLAYER, MIXER_GAIN, MOVIE_CONTROL, MOVIE_SEEK,
    OBJECT_QUEUE, PARTICLES, PARTICLES_X87, POLAR_POINT, RAIN, ROOT, SCREEN_COORDINATES, SE_GAIN,
    SE_RESTART, SEQUENCE_WINDOW_IMAGE, TEXT_ALPHA, TEXT_DIALOG, VARIANT_TABLES, VIEW_MOTION,
    WINDOW_CHILD_IMAGE, WINDOW_DRAW_SCALE, WINDOW_FULL_MOVE, WINDOW_OPACITY_TRACK,
    WINDOW_POSITION_MOTION, WINDOW_SCALE_MOTION, is_lifecycle, is_message_wait, is_wait,
    is_wait_for, semantics_name, uses_bootstrap, uses_bootstrap_for, uses_device_frame,
    uses_device_frame_for, uses_images, uses_input, uses_input_for, uses_message_state,
    uses_movie_player, uses_movie_player_for, uses_movie_scene, uses_movie_scene_for,
    uses_platform, uses_platform_for, uses_sequence, uses_window_images, uses_windows,
    variant_entry,
};

use crate::{
    bytecode::DecodeError,
    machine::{Machine, Step},
    platform::PlatformEvent,
    profile::Semantics,
    script_control::ControlEvent,
    system_save::SystemSaveEvent,
};
use std::{error::Error, fmt};

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
        crate::find_error_source(Some(self.cause.as_ref()))
    }

    pub fn as_format(&self) -> Option<&crate::FormatError> {
        crate::find_error_source(Some(self.cause.as_ref()))
    }

    pub fn as_io(&self) -> Option<&std::io::Error> {
        crate::find_error_source(Some(self.cause.as_ref()))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Runtime(RuntimeCall),
    Control(ControlEvent),
    SystemSave(SystemSaveEvent),
    SystemWritten,
    Platform(PlatformEvent),
    DeviceFramePending,
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

type Handler = fn(&mut Hosts<'_>, &mut Call<'_, '_>) -> Result<Flow, Box<dyn Error>>;

/// Registration metadata for command inspection. Execute commands through [`run`].
pub struct Entry {
    /// VM system-command number (opcode minus 0x2000).
    pub command: u16,
    pub name: &'static str,
    pub capability: Capability,
    handler: Handler,
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
        Semantics::Variant(id) => variant_entry(id, command),
        Semantics::Root => root_entry(command),
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

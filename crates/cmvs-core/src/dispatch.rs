//! Shared static system-command routing and sparse identity-bound variant overrides.
//! Gate resolution precedes dispatch. Runtime-wide operations return a checked
//! request for the caller's orchestration; they remain pending until acknowledged.
//! A selected override never falls through to root semantics.
//!
//! Custom executors use [`run`] with [`Hosts`] after [`crate::profile::CommandGate`]
//! resolves the command. Complete hosts should use [`crate::runtime::Runtime`],
//! which performs this sequence and handles runtime requests.
//!
//! [`Entry`], [`ROOT`], [`CVNS`], [`VARIANT_TABLES`], and [`variant_entry`] expose registration
//! metadata for inspection. Handlers and their argument bookkeeping are internal;
//! metadata alone does not authorize a command or establish host capabilities.
mod call;
mod handlers;
pub(crate) use handlers::platform::{VmTextDialog, cvns_message_request, store_color_result};
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
pub(crate) use registry::resolve_entry;
pub use registry::{
    ACTIVATION_PASS, AUDIO_PRELOAD, BACKGROUND_AUDIO, CAMERA_MOVE_X, COLLISION_PARTICLES, CVNS,
    DISPLAY_COUNT, DRAW_QUERY, FADING_PARTICLES, FALLING_BILLBOARDS, FALLING_BILLBOARDS_X87,
    FONT_CELLS, FRAGMENTS, FULLSCREEN_MONITOR, IMAGE_TABLES, INPUT_PRESS, KEYED_ARCHIVES,
    LEGACY_EFFECTS, LIST_SELECT, LOGICAL_TEXT, MGV_PLAYER, MIXER_GAIN, MOVIE_CONTROL, MOVIE_SEEK,
    OBJECT_QUEUE, PARTICLES, PARTICLES_X87, POLAR_POINT, RAIN, ROOT, SCREEN_COORDINATES, SE_GAIN,
    SE_RESTART, SEQUENCE_WINDOW_IMAGE, TEXT_ALPHA, TEXT_DIALOG, VARIANT_TABLES, VIEW_MOTION,
    WINDOW_CHILD_IMAGE, WINDOW_DRAW_SCALE, WINDOW_FULL_MOVE, WINDOW_OPACITY_TRACK,
    WINDOW_POSITION_MOTION, WINDOW_SCALE_MOTION, entry, is_lifecycle, is_message_wait, is_wait,
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
#[derive(Debug)]
pub struct DispatchError {
    pub pc: usize,
    /// VM command number (opcode minus 0x2000), including for backend failures.
    pub command: u16,
    /// Raw script opcode (`0x2000 + pending_source`), before any CVNS command mapping.
    pub opcode: u16,
    cause: Box<dyn Error + Send + Sync>,
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
        if let Some(err) = self.cause.downcast_ref::<DecodeError>() {
            DecodeError {
                pc: self.pc,
                opcode: Some(self.opcode),
                reason: err.reason,
            }
            .fmt(f)
        } else if let Some(err) = self.cause.downcast_ref::<crate::scene::ImageLoadError>() {
            err.fmt_at(f, self.pc, self.opcode)
        } else {
            write!(
                f,
                "system command 0x{:04x} at 0x{:x}: {}",
                self.opcode, self.pc, self.cause
            )
        }
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

type Handler = fn(&mut Hosts<'_>, &mut Call<'_>) -> Result<Flow, Box<dyn Error + Send + Sync>>;

/// Registration metadata for command inspection. Execute commands through [`run`].
pub struct Entry {
    /// VM system-command number (opcode minus 0x2000).
    pub command: u16,
    pub name: &'static str,
    pub capability: Capability,
    handler: Handler,
}

impl Entry {
    pub(crate) fn is_lifecycle(&self) -> bool {
        self.capability == Capability::Runtime && matches!(self.command, 1 | 3)
    }

    pub(crate) fn is_message_wait(&self) -> bool {
        self.capability == Capability::MessageWait
    }

    pub(crate) fn is_wait(&self) -> bool {
        self.capability.is_wait()
    }

    pub(crate) fn uses_bootstrap(&self) -> bool {
        self.capability.uses_bootstrap()
    }

    pub(crate) fn uses_input(&self) -> bool {
        self.capability.uses_input()
    }

    pub(crate) fn uses_platform(&self) -> bool {
        self.capability == Capability::Platform
    }

    pub(crate) fn uses_device_frame(&self) -> bool {
        self.capability.uses_device_frame()
    }

    pub(crate) fn uses_sequence(&self) -> bool {
        self.capability == Capability::Sequence
    }

    pub(crate) fn uses_movie_scene(&self) -> bool {
        self.capability == Capability::MovieScene
    }

    pub(crate) fn uses_movie_player(&self) -> bool {
        self.capability == Capability::MoviePlayer
    }

    pub(crate) fn uses_window_images(&self) -> bool {
        self.capability == Capability::WindowImages
    }

    pub(crate) fn uses_images(&self) -> bool {
        self.capability.uses_scene_images()
    }

    pub(crate) fn uses_windows(&self) -> bool {
        self.capability.uses_windows()
    }

    pub(crate) fn uses_message_state(&self) -> bool {
        self.capability.uses_message_state(self.command)
    }

    pub(crate) fn run(
        &self,
        vm: &mut Machine,
        hosts: &mut Hosts<'_>,
    ) -> Result<Option<Flow>, DispatchError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(None);
        };
        debug_assert_eq!(command, self.command);
        if !hosts.has(self.capability) {
            return Ok(Some(Flow::Unavailable));
        }
        let mut call = Call::new(vm, pc, command);
        let opcode = call.opcode();
        (self.handler)(hosts, &mut call)
            .map(Some)
            .map_err(|cause| DispatchError {
                pc,
                command,
                opcode,
                cause,
            })
    }
}

/// Called after CommandGate::resolve. None means no pending command or no root
/// entry. Unavailable preserves the pending command and arguments; the
/// caller may report the missing capability, but must not try another handler.
pub fn run(
    semantics: Semantics,
    vm: &mut Machine,
    hosts: &mut Hosts<'_>,
) -> Result<Option<Flow>, DispatchError> {
    let Some((_, command)) = vm.pending_system() else {
        return Ok(None);
    };
    let Some(entry) = resolve_entry(semantics, command) else {
        return Ok((semantics != Semantics::Root).then_some(Flow::Unhandled));
    };
    entry.run(vm, hosts)
}

#[cfg(test)]
mod tests {
    use super::DispatchError;
    use std::error::Error;

    #[test]
    fn dispatch_error_display_uses_source_opcode_and_avoids_duplicate_prefixes() {
        let err = DispatchError {
            pc: 0x1a4,
            command: 0x0152,
            opcode: 0x2152,
            cause: "bad operand".into(),
        };
        assert_eq!(
            err.to_string(),
            "system command 0x2152 at 0x1a4: bad operand"
        );
        assert!(err.source().is_some());

        // CVNS commands format their raw script opcode rather than internal 0x24xx/mapped values.
        let cvns = DispatchError {
            pc: 0x80,
            command: 0x04c0,
            opcode: 0x20c0,
            cause: "movie unavailable".into(),
        };
        assert_eq!(
            cvns.to_string(),
            "system command 0x20c0 at 0x80: movie unavailable"
        );

        let decode = crate::evaluate::error(0x44, 0x20a7, "missing operand");
        let expected = decode.to_string();
        let wrapped = DispatchError {
            pc: 0x44,
            command: 0x04a7,
            opcode: 0x20a7,
            cause: Box::new(decode),
        };
        assert_eq!(wrapped.to_string(), expected);
    }
}

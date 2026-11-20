//! CMVS resource formats, script execution, rendering model, and host interfaces.
//!
//! # Start here
//!
//! Choose the level of integration first:
//!
//! | Integration | Entry point | Caller supplies |
//! |---|---|---|
//! | Embed a complete player | `cmvs-host::Player` (separate crate) | Presentation, input, save directory, font |
//! | Build a custom host | [`runtime::Runtime`], [`runtime::RuntimeOptions`], [`runtime::FrameDriver`] | Device services and frame composition |
//! | Step scripts or build analysis tools | [`vm::machine::Machine`] | Resources, command handling, scheduling |
//!
//! For a custom host, start with [`profile::Registry`] to identify the game, then
//! follow the initialization and frame-loop guide in [`runtime`].
//!
//! # Domain modules
//!
//! The public API is organized into six domain modules:
//!
//! - [`mod@format`]: Archive, script, image, video, cursor, executable, and save formats.
//! - [`vm`]: Bytecode decoding, VM value/stack model, bounded [`vm::machine::Machine`] execution, and [`vm::dispatch`].
//! - [`profile`]: Engine build registry, game identification, and [`profile::CommandGate`] authorization.
//! - [`render`]: Scene graph, camera, message layout, glyph rasterization, software compositing, and viewport mapping.
//! - [`host`]: Host capability traits and adapters for filesystem, resources, audio, video, input, gestures, and settings.
//! - [`runtime`]: Full session orchestration ([`runtime::Runtime`]), bootstrap, script control, and save-menu state.
//!
//! # Resource APIs
//!
//! These readers and decoders can be used independently of a running game.
//!
//! | Task | Start here |
//! |---|---|
//! | Inspect or extract an archive | [`format::cpz::ArchiveIndex`] |
//! | Read a script or disassemble instructions | [`format::ps3::Script`], [`vm::bytecode`] |
//! | Decode an image | [`format::pb3::decode`], [`host::image::load`] |
//! | Inspect save data | [`format::save`], [`format::system_save`], [`format::snapshot`] |
//!
//! ## Inspect a script
//!
//! ```no_run
//! use cmvs_core::format::ps3::Script;
//!
//! let bytes = std::fs::read("data/pack/start.ps3")?;
//! let script = Script::decode(&bytes)?;
//! println!("entry: {}, code bytes: {}", script.header().entry_pc, script.code().len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Script text remains encoded bytes. Select [`TextEncoding`] when executing or
//! displaying text; resource names keep their own lookup rules.
//!
//! # Host services and engine state
//!
//! [`runtime::Runtime`] owns the engine state. Custom hosts supply
//! [`host::platform::PlatformBackend`] and [`host::audio::AudioBackend`], update [`host::input`], and
//! compose [`render::scene`], [`render::camera`], and [`render::message`] state through [`render::scene_render`].
//! [`render::raster::RgbaImage`] represents software output; [`render::scene_render::RenderFrame`]
//! supplies the shared draw queue for GPU backends.
//!
//! The host supplies windows, graphics devices, and audio output. The core has no
//! third-party dependencies; hosts supply storage via [`host::filesystem::HostFs`] or use
//! the optional `std-fs` feature (`StdFs`, enabled by default).
//!
//! # Errors and support
//!
//! Format readers validate bounds and return [`FormatError`] or format-specific errors.
//! Execution support depends on the selected engine profile: parsing a resource does
//! not imply that every command or effect used by its game is implemented.

#[path = "."]
mod modules {
    pub mod audio;
    pub mod bootstrap;
    pub mod buttons;
    pub mod bytecode;
    pub mod camera;
    pub mod capture;
    pub mod cmv;
    pub mod cmv9;
    pub mod cpz;
    pub mod cursor;
    pub mod cvns;
    pub mod digest;
    pub mod dispatch;
    pub mod effect_objects;
    pub mod evaluate;
    pub mod exe;
    pub mod fade;
    pub mod filesystem;
    pub mod gesture;
    pub mod history;
    pub mod image;
    pub mod input;
    pub mod jbp1;
    pub mod jbpd;
    pub mod lzss;
    pub mod machine;
    pub mod math;
    pub mod message;
    pub mod message_flags;
    pub mod mgv;
    pub mod movie;
    pub mod msk;
    pub mod mv2;
    pub mod pb2;
    pub mod pb3;
    #[cfg(feature = "encode")]
    pub(crate) mod pb_write;
    pub mod platform;
    pub mod ps3;
    pub mod random;
    pub mod raster;
    pub mod resources;
    pub mod save;
    pub mod save_menu;
    pub mod scene;
    pub mod scene_render;
    pub mod script_control;
    pub mod sequence;
    pub mod settings;
    pub mod snapshot;
    pub mod system_save;
    pub(crate) mod text;
    pub mod text_encoding;
    pub mod timers;
    pub mod viewport;
    #[path = "vm.rs"]
    pub mod vm_core;
}

#[cfg(feature = "encode")]
pub(crate) use modules::pb_write;

#[allow(unused_imports)]
pub(crate) use modules::{
    audio, bootstrap, buttons, bytecode, camera, capture, cmv, cmv9, cpz, cursor, cvns, digest,
    dispatch, effect_objects, evaluate, exe, fade, filesystem, gesture, history, image, input,
    jbp1, jbpd, lzss, machine, math, message, message_flags, mgv, movie, msk, mv2, pb2, pb3,
    platform, ps3, random, raster, resources, save, save_menu, scene, scene_render, script_control,
    sequence, settings, snapshot, system_save, text, text_encoding, timers, viewport, vm_core,
};

/// Resource containers, scripts, images, video, cursors, saves, and executable architecture.
pub mod format {
    pub use crate::modules::{
        cmv, cmv9, cpz, cursor, digest, exe, jbp1, jbpd, lzss, mgv, msk, mv2, pb2, pb3, ps3, save,
        snapshot, system_save,
    };
    pub use crate::{FormatError, MAX_PAYLOAD};
}

/// Bytecode decoding, VM value/stack model, bounded machine execution, and command dispatch.
pub mod vm {
    pub use crate::modules::vm_core::{ParameterStack, StackWord, StringRef, SystemResult};
    pub use crate::modules::{bytecode, cvns, dispatch, evaluate, machine};
}

pub mod profile;

/// Scene graph, camera, message layout, glyph rasterization, software compositing, and viewport mapping.
pub mod render {
    pub use crate::modules::{
        camera, capture, fade, message, raster, scene, scene_render, viewport,
    };
}

/// Host capability traits and adapters for filesystem, resources, audio, video, input, gestures, and settings.
pub mod host {
    pub use crate::modules::text_encoding::TextEncoding;
    pub use crate::modules::{
        audio, filesystem, gesture, image, input, movie, platform, random, resources, settings,
        text_encoding,
    };
}

pub mod runtime;

pub use host::TextEncoding;

use std::fmt;

/// A format validation failure, with a short description of the failed check.
///
/// Includes malformed, unsupported, and oversized data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub &'static str);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for FormatError {}

/// Walk an error's `source` chain to locate the first inner error of type `T`.
pub(crate) fn find_error_source<'a, T: std::error::Error + 'static>(
    mut current: Option<&'a (dyn std::error::Error + 'static)>,
) -> Option<&'a T> {
    while let Some(error) = current {
        if let Some(found) = error.downcast_ref::<T>() {
            return Some(found);
        }
        current = error.source();
    }
    None
}

/// Maximum payload accepted by bounded readers, in bytes (64 MiB).
///
/// This is an allocation policy, not an on-disk format limit or a process memory budget.
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

//! CMVS resource formats, script execution, and host interfaces.
//!
//! Choose an entry point by task:
//!
//! | Task | Start here |
//! |---|---|
//! | Inspect or extract an archive | [`cpz::ArchiveIndex`] |
//! | Read a script or disassemble instructions | [`ps3::Script`], [`bytecode`] |
//! | Decode an image | [`pb3::decode`], [`image::load`] |
//! | Identify a game and run it | [`profile::Registry`], [`runner::Startup`] |
//! | Execute individual VM instructions | [`machine::Machine`] |
//! | Implement a window, input, or output backend | [`platform::PlatformBackend`], [`input`], [`audio::AudioBackend`] |
//! | Render engine state | [`scene_render`], [`raster::RgbaImage`] |
//! | Inspect save data | [`save`], [`system_save`], [`snapshot`] |
//!
//! # Inspect a script
//!
//! ```no_run
//! use cmvs_core::ps3::Script;
//!
//! let bytes = std::fs::read("data/pack/start.ps3")?;
//! let script = Script::decode(&bytes)?;
//! println!("entry: {}, code bytes: {}", script.header.entry_pc, script.code().len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Script text remains encoded bytes. Select [`TextEncoding`] when executing or
//! displaying text; resource names keep their own lookup rules.
//!
//! # Running and hosting
//!
//! Use [`runner::Startup`] to run an identified game in a host's frame loop. It
//! combines the selected profile, VM, resources, and engine state. Use
//! [`machine::Machine`] for individual VM steps and handle system commands yourself.
//!
//! The host supplies windows, graphics devices, and audio output. Desktop builds
//! of the core use only the standard library. Android and browser builds add
//! dependencies for their filesystem adapters.
//!
//! # Errors and support
//!
//! Format readers validate bounds and return [`FormatError`] or format-specific errors.
//! Execution support depends on the selected engine profile: parsing a resource does
//! not imply that every command or effect used by its game is implemented.
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
pub mod pb3;
pub mod platform;
pub mod profile;
pub mod ps3;
pub mod random;
pub mod raster;
pub mod resources;
pub mod runner;
pub mod save;
pub mod save_menu;
pub mod scene;
pub mod scene_render;
pub mod script_control;
pub mod sequence;
pub mod settings;
pub mod snapshot;
pub mod system_save;
pub mod text;
pub mod text_encoding;
pub use text_encoding::TextEncoding;
pub mod timers;
pub mod viewport;
pub mod vm;

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

/// Maximum payload accepted by bounded readers, in bytes (64 MiB).
///
/// This is an allocation policy, not an on-disk format limit or a process memory budget.
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

pub mod filesystem;

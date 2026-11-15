//! CMVS resource formats, script execution, and host interfaces.
//!
//! # Start here
//!
//! Choose the level of integration first:
//!
//! | Integration | Entry point | Caller supplies |
//! |---|---|---|
//! | Embed a complete player | `cmvs-host::Player` (separate crate) | Presentation, input, save directory, font |
//! | Build a custom host | [`runtime::Runtime`], [`runtime::RuntimeOptions`], [`runtime::FrameDriver`] | Device services and frame composition |
//! | Step scripts or build analysis tools | [`machine::Machine`] | Resources, command handling, scheduling |
//!
//! For a custom host, start with [`profile::Registry`] to identify the game, then
//! follow the initialization and frame-loop guide in [`runtime`].
//!
//! # Resource APIs
//!
//! These readers and decoders can be used independently of a running game.
//!
//! | Task | Start here |
//! |---|---|
//! | Inspect or extract an archive | [`cpz::ArchiveIndex`] |
//! | Read a script or disassemble instructions | [`ps3::Script`], [`bytecode`] |
//! | Decode an image | [`pb3::decode`], [`image::load`] |
//! | Inspect save data | [`save`], [`system_save`], [`snapshot`] |
//!
//! ## Inspect a script
//!
//! ```no_run
//! use cmvs_core::ps3::Script;
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
//! [`platform::PlatformBackend`] and [`audio::AudioBackend`], update [`input`], and
//! compose [`scene`], [`camera`], and [`message`] state through [`scene_render`].
//! [`raster::RgbaImage`] represents software output; [`scene_render::RenderFrame`]
//! supplies the shared draw queue for GPU backends.
//!
//! The host supplies windows, graphics devices, and audio output. The core has no
//! third-party dependencies; hosts supply storage via [`filesystem::HostFs`] or use
//! the optional `std-fs` feature (`StdFs`, enabled by default).
//!
//! # Low-level execution and research
//!
//! [`vm`] and [`evaluate`] expose VM values and execution state. [`dispatch`]
//! provides command routing and metadata for custom executors; [`runtime::Runtime`]
//! already handles this layer. [`bootstrap`], [`script_control`], and the individual
//! engine-state modules support command implementation and focused tools.
//! [`exe`], [`digest`], and [`cvns`] support engine identification and analysis.
//!
//! Module paths are the canonical API. Compatibility aliases remain callable but
//! are omitted from the generated documentation.
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
pub mod runtime;
/// Compatibility path for [`runtime`].
#[doc(hidden)]
pub use runtime as runner;
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

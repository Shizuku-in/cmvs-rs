//! Platform-independent CMVS formats, VM, scene and backend contracts. No dependencies.
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
pub mod digest;
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
pub mod snapshot;
pub mod system_save;
pub mod text;
pub mod timers;
pub mod vm;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub &'static str);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for FormatError {}

/// A research-tool allocation bound, not an inferred engine format limit.
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

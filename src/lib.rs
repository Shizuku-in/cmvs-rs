//! Platform-independent CMVS research core; optional `desktop` adds a native window adapter.
pub mod audio;
pub mod bootstrap;
pub mod bytecode;
pub mod cpz;
pub mod cursor;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod evaluate;
pub mod gesture;
pub mod lzss;
pub mod machine;
pub mod pb3;
pub mod platform;
pub mod ps3;
pub mod random;
pub mod raster;
pub mod runner;
pub mod save;
pub mod save_menu;
pub mod scene;
pub mod script_control;
pub mod snapshot;
pub mod system_save;
pub mod text;
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

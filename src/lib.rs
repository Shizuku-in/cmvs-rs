//! Platform-independent CMVS format research. No game assets or OS APIs.
pub mod bytecode;
pub mod lzss;
pub mod ps3;
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

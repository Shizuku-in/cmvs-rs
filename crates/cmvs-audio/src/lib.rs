//! Shared streaming Vorbis decoder, with optional native device output.
mod program;
pub mod vorbis;
pub use program::VorbisProgram;
#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
pub use native::*;

//! Shared streaming Vorbis decoder, with an optional engine mixer (`mixer`) and native
//! device output (`native`).
mod pcm_wave;
mod program;
pub mod vorbis;
pub use program::VorbisProgram;
#[cfg(feature = "mixer")]
mod native;
#[cfg(feature = "mixer")]
pub use native::*;

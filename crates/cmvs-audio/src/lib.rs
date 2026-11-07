//! Shared streaming Ogg Vorbis and PCM WAVE decoders, with an optional engine mixer (`mixer`) and native
//! device output (`native`).
mod pcm_wave;
mod program;
pub mod vorbis;
pub use program::AudioProgram;

/// Compatibility alias for the historical name of [`AudioProgram`].
pub type VorbisProgram = AudioProgram;
#[cfg(feature = "mixer")]
mod native;
#[cfg(feature = "mixer")]
pub use native::*;

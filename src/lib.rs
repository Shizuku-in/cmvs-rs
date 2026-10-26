//! Compatibility facade for the CMVS workspace. The default dependency is only cmvs-core.
#[cfg(feature = "audio-output")]
pub use cmvs_audio as audio_output;
#[cfg(feature = "audio-output")]
pub use cmvs_audio::vorbis;
pub use cmvs_core::*;
#[cfg(feature = "desktop")]
pub use cmvs_desktop as desktop;

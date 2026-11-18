//! Compatibility entry point for the CMVS workspace.
//!
//! All public items from [`cmvs_core`] are re-exported here. See the
//! [core API guide](cmvs_core) for entry points and examples. New library
//! integrations can depend on the required workspace crates directly.
//!
//! # Optional backends
//!
//! | Feature | Additional API |
//! |---|---|
//! | `desktop` | `desktop` window and player integration |
//! | `audio-output` | `audio_output` output backends and `vorbis` decoding |
//! | `wgpu-renderer` | GPU rendering through the desktop backend |
//!
//! With default features this crate depends only on `cmvs-core`.
#[cfg(feature = "audio-output")]
pub use cmvs_audio as audio_output;
#[cfg(feature = "audio-output")]
pub use cmvs_audio::vorbis;
pub use cmvs_core::*;
#[cfg(feature = "desktop")]
pub use cmvs_desktop as desktop;

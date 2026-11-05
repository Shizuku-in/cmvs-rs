//! GPU types shared by the Horizon device and host stand-ins.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// Texture memory or descriptor slots are exhausted; release textures and retry.
    OutOfMemory,
    /// The device failed; errors are sticky and every later call fails too.
    Device(String),
}
impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory => f.write_str("GPU texture memory exhausted"),
            Self::Device(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for GpuError {}

/// A texture created by `SwitchGpu::create_texture`; stale handles are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTexture(pub(super) u32);

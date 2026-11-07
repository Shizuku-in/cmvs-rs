//! GPU types shared by the console compositors (Switch deko3d, Vita GXM) and the
//! players' host stand-ins.
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

/// A texture created by [`GpuDevice::create_texture`]; devices reject stale handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTexture(pub(crate) u32);
impl GpuTexture {
    /// A handle for device stand-ins and tests; real devices validate every handle.
    pub fn from_raw(id: u32) -> Self {
        Self(id)
    }
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// The compositing operations a console GPU executes. Frame order:
/// [`begin_frame`](Self::begin_frame), texture uploads,
/// [`begin_canvas`](Self::begin_canvas), then draws; presenting is device specific.
pub trait GpuDevice {
    /// Waits for the previous frame and recycles its memory.
    fn begin_frame(&mut self) -> Result<(), GpuError>;
    /// Uploads tightly packed RGBA rows.
    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<GpuTexture, GpuError>;
    /// Draws already submitted may still use the texture; its memory is reused later.
    fn release_texture(&mut self, texture: GpuTexture);
    /// Starts the logical canvas pass, cleared to `clear`.
    fn begin_canvas(&mut self, size: [u32; 2], clear: [u8; 4]) -> Result<(), GpuError>;
    /// `blend` is 0 replace, 1 alpha, 2 additive; `params` uses the layout of
    /// `cmvs_core::scene_render::shader` (24 or 280 floats); textures bind at slots
    /// 0 (source), 1 (mask) and 2 (screen mask).
    fn draw(
        &mut self,
        blend: u32,
        params: &[f32],
        textures: [GpuTexture; 3],
    ) -> Result<(), GpuError>;
}

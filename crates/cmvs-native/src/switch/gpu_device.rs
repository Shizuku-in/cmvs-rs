//! Safe ownership of the deko3d compositor in platforms/switch/runtime/gpu.c. The C side
//! validates every texture handle and defers freeing released textures until the GPU
//! idles, so no sequence of calls here can make the GPU read freed memory.
#![allow(unsafe_code)]
use super::{GpuError, GpuTexture};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);

const OK: i32 = 0;
const NO_MEMORY: i32 = -2;
/// Floats per draw without and with the mask coverage table.
const DRAW_FLOATS: [usize; 2] = [24, 280];
const MAX_OVERLAYS: usize = 16;

unsafe extern "C" {
    fn cmvs_gpu_open() -> i32;
    fn cmvs_gpu_error(out: *mut u8, capacity: u32) -> u32;
    fn cmvs_gpu_begin_frame() -> i32;
    fn cmvs_gpu_create_texture(width: u32, height: u32, rgba: *const u8, out: *mut u32) -> i32;
    fn cmvs_gpu_release_texture(id: u32);
    fn cmvs_gpu_begin_canvas(width: u32, height: u32, clear: *const u8) -> i32;
    fn cmvs_gpu_draw(blend: u32, params: *const f32, count: u32, ids: *const u32) -> i32;
    fn cmvs_gpu_scale(id: u32, view: *const f32) -> i32;
    fn cmvs_gpu_present(view: *const f32, overlays: *const f32, count: u32) -> i32;
    fn cmvs_gpu_read(out: *mut u8, width: u32, height: u32) -> i32;
    fn cmvs_gpu_close();
}

/// Frame order: [`begin_frame`](Self::begin_frame), texture uploads,
/// [`begin_canvas`](Self::begin_canvas), draws, then [`present`](Self::present).
pub struct SwitchGpu(PhantomData<Rc<()>>);
impl SwitchGpu {
    /// Opened by [`super::SwitchWindow::open_gpu`], which keeps the software
    /// framebuffer off the default window.
    pub(super) fn open() -> Result<Self, String> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Switch GPU already open".into());
        }
        // SAFETY: C rejects a second owner and releases partial initialization.
        if unsafe { cmvs_gpu_open() } != OK {
            let message = last_error();
            OWNED.store(false, Ordering::Release);
            return Err(format!("deko3d initialization failed: {message}"));
        }
        Ok(Self(PhantomData))
    }
    /// Waits for the previous frame and recycles its command and staging memory.
    pub fn begin_frame(&mut self) -> Result<(), GpuError> {
        // SAFETY: no arguments; this value owns the open device.
        check(unsafe { cmvs_gpu_begin_frame() })
    }
    pub fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<GpuTexture, GpuError> {
        if width == 0
            || height == 0
            || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err(GpuError::Device("invalid GPU texture pixels".into()));
        }
        let mut id = 0;
        // SAFETY: C synchronously copies exactly width*height*4 bytes and writes one u32.
        check(unsafe { cmvs_gpu_create_texture(width, height, rgba.as_ptr(), &mut id) })?;
        Ok(GpuTexture(id))
    }
    /// Draws already recorded may still use the texture; its memory is reused later.
    pub fn release_texture(&mut self, texture: GpuTexture) {
        // SAFETY: C ignores unknown or stale handles.
        unsafe { cmvs_gpu_release_texture(texture.0) }
    }
    pub fn begin_canvas(&mut self, size: [u32; 2], clear: [u8; 4]) -> Result<(), GpuError> {
        // SAFETY: C reads four bytes.
        check(unsafe { cmvs_gpu_begin_canvas(size[0], size[1], clear.as_ptr()) })
    }
    /// `blend` is 0 replace, 1 alpha, 2 additive; `params` uses the layout of
    /// `cmvs_core::scene_render::shader` (24 or 280 floats).
    pub fn draw(
        &mut self,
        blend: u32,
        params: &[f32],
        textures: [GpuTexture; 3],
    ) -> Result<(), GpuError> {
        if !DRAW_FLOATS.contains(&params.len()) {
            return Err(GpuError::Device("invalid GPU draw parameters".into()));
        }
        let ids = textures.map(|t| t.0);
        // SAFETY: C reads `params.len()` floats and three ids, and validates the ids.
        check(unsafe { cmvs_gpu_draw(blend, params.as_ptr(), params.len() as u32, ids.as_ptr()) })
    }
    /// Scales a texture onto the canvas: viewport left/top/width/height, source
    /// width/height, linear flag, 0.
    pub fn scale(&mut self, texture: GpuTexture, view: [f32; 8]) -> Result<(), GpuError> {
        // SAFETY: C reads eight floats and validates the id.
        check(unsafe { cmvs_gpu_scale(texture.0, view.as_ptr()) })
    }
    /// Shows the canvas in the 1280x720 screen viewport (as for [`Self::scale`]), then
    /// draws replace-blended 24-float scene overlays in screen coordinates.
    pub fn present(&mut self, view: [f32; 8], overlays: &[[f32; 24]]) -> Result<(), GpuError> {
        if overlays.len() > MAX_OVERLAYS {
            return Err(GpuError::Device("too many GPU overlays".into()));
        }
        // SAFETY: C reads eight floats and `overlays.len()` contiguous 24-float arrays.
        check(unsafe {
            cmvs_gpu_present(
                view.as_ptr(),
                overlays.as_ptr().cast(),
                overlays.len() as u32,
            )
        })
    }
    /// Reads the canvas as last presented into tightly packed RGBA rows.
    pub fn read(&mut self, size: [u32; 2], out: &mut [u8]) -> Result<(), GpuError> {
        if out.len() as u64 != u64::from(size[0]) * u64::from(size[1]) * 4 {
            return Err(GpuError::Device("invalid GPU readback buffer".into()));
        }
        // SAFETY: C checks the size against the canvas and writes exactly that many bytes.
        check(unsafe { cmvs_gpu_read(out.as_mut_ptr(), size[0], size[1]) })
    }
}
impl Drop for SwitchGpu {
    fn drop(&mut self) {
        // SAFETY: this value is the unique owner created by a successful open.
        unsafe { cmvs_gpu_close() };
        OWNED.store(false, Ordering::Release);
    }
}

fn check(code: i32) -> Result<(), GpuError> {
    match code {
        OK => Ok(()),
        NO_MEMORY => Err(GpuError::OutOfMemory),
        _ => Err(GpuError::Device(last_error())),
    }
}

fn last_error() -> String {
    let mut buffer = [0u8; 192];
    // SAFETY: C writes at most `capacity` bytes.
    let length = unsafe { cmvs_gpu_error(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    let text = String::from_utf8_lossy(&buffer[..length.min(buffer.len())]).into_owned();
    if text.is_empty() {
        "unknown GPU error".into()
    } else {
        text
    }
}

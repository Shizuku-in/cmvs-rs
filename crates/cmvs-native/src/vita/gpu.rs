//! Safe ownership of the GXM compositor in platforms/psvita/runtime/gpu.c. The C side
//! validates every texture handle and frees released textures only after the GPU
//! idles, so no sequence of calls here can make the GPU read freed memory. Closing the
//! window closes the compositor too; later calls then fail.
#![allow(unsafe_code)]
use crate::{GpuDevice, GpuError, GpuTexture};
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

unsafe extern "C" {
    fn cmvs_vita_gpu_open() -> i32;
    fn cmvs_vita_gpu_error(out: *mut u8, capacity: u32) -> u32;
    fn cmvs_vita_gpu_begin_frame() -> i32;
    fn cmvs_vita_gpu_create_texture(width: u32, height: u32, rgba: *const u8, out: *mut u32)
    -> i32;
    fn cmvs_vita_gpu_release_texture(id: u32);
    fn cmvs_vita_gpu_begin_canvas(width: u32, height: u32, clear: *const u8) -> i32;
    fn cmvs_vita_gpu_draw(blend: u32, params: *const f32, count: u32, ids: *const u32) -> i32;
    fn cmvs_vita_gpu_present(view: *const f32, linear: i32, cursor: *const f32) -> i32;
    fn cmvs_vita_gpu_read(out: *mut u8, width: u32, height: u32) -> i32;
    fn cmvs_vita_gpu_close();
}

/// The GXM compositor on the window's display: frames run as described on
/// [`GpuDevice`], then [`present`](Self::present).
pub struct VitaGpu(PhantomData<Rc<()>>);
impl VitaGpu {
    /// Opened by [`super::VitaWindow::open_gpu`] on the window's GXM context.
    pub(super) fn open() -> Result<Self, String> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Vita GPU already open".into());
        }
        // SAFETY: C rejects a second owner or a closed window and releases partial
        // initialization.
        if unsafe { cmvs_vita_gpu_open() } != OK {
            let message = last_error();
            OWNED.store(false, Ordering::Release);
            return Err(format!("GXM initialization failed: {message}"));
        }
        Ok(Self(PhantomData))
    }
    /// Ends the canvas pass and shows the canvas scaled into `view` (left, top, width,
    /// height in screen pixels) like a software frame, with an optional crosshair at a
    /// screen position. System dialogs redraw it until the next present.
    pub fn present(
        &mut self,
        view: [f32; 4],
        linear: bool,
        cursor: Option<[f32; 2]>,
    ) -> Result<(), GpuError> {
        let cursor_ptr = cursor.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        // SAFETY: C reads four view floats and, when non-null, two cursor floats.
        check(unsafe { cmvs_vita_gpu_present(view.as_ptr(), i32::from(linear), cursor_ptr) })
    }
    /// Reads the canvas as last presented into tightly packed RGBA rows.
    pub fn read(&mut self, size: [u32; 2], out: &mut [u8]) -> Result<(), GpuError> {
        if out.len() as u64 != u64::from(size[0]) * u64::from(size[1]) * 4 {
            return Err(GpuError::Device("invalid GPU readback buffer".into()));
        }
        // SAFETY: C checks the size against the canvas and writes exactly that many bytes.
        check(unsafe { cmvs_vita_gpu_read(out.as_mut_ptr(), size[0], size[1]) })
    }
}
impl GpuDevice for VitaGpu {
    fn begin_frame(&mut self) -> Result<(), GpuError> {
        // SAFETY: no arguments; C checks that the compositor is open.
        check(unsafe { cmvs_vita_gpu_begin_frame() })
    }
    fn create_texture(
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
        check(unsafe { cmvs_vita_gpu_create_texture(width, height, rgba.as_ptr(), &mut id) })?;
        Ok(GpuTexture(id))
    }
    fn release_texture(&mut self, texture: GpuTexture) {
        // SAFETY: C ignores unknown or stale handles.
        unsafe { cmvs_vita_gpu_release_texture(texture.0) }
    }
    fn begin_canvas(&mut self, size: [u32; 2], clear: [u8; 4]) -> Result<(), GpuError> {
        // SAFETY: C reads four bytes.
        check(unsafe { cmvs_vita_gpu_begin_canvas(size[0], size[1], clear.as_ptr()) })
    }
    fn draw(
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
        check(unsafe {
            cmvs_vita_gpu_draw(blend, params.as_ptr(), params.len() as u32, ids.as_ptr())
        })
    }
}
impl Drop for VitaGpu {
    fn drop(&mut self) {
        // SAFETY: this value is the unique owner created by a successful open; C
        // ignores a compositor the window already closed.
        unsafe { cmvs_vita_gpu_close() };
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
    let length = unsafe { cmvs_vita_gpu_error(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    let text = String::from_utf8_lossy(&buffer[..length.min(buffer.len())]).into_owned();
    if text.is_empty() {
        "unknown GPU error".into()
    } else {
        text
    }
}

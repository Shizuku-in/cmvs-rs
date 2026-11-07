//! Safe ownership of the single vita2d display. Calls stay on the creating thread.
#![allow(unsafe_code)]
use crate::{PadInput, Touch};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);
/// Set while a window is open; the dialogs need its render target.
pub(super) static OPEN: AtomicBool = AtomicBool::new(false);

const TOUCHES: usize = 4;
/// Mirrors `CmvsVitaInput` in platforms/psvita/runtime/window.c.
#[repr(C)]
#[derive(Default)]
struct RawInput {
    buttons: u32,
    flags: u32,
    sticks: [[i32; 2]; 2],
    touch_count: u32,
    touches: [[u32; 3]; TOUCHES],
}
const FLAG_FOCUSED: u32 = 2;
const MAX_TEXTURE: u32 = 4096;

unsafe extern "C" {
    fn cmvs_vita_open() -> i32;
    fn cmvs_vita_confirm_is_cross() -> i32;
    fn cmvs_vita_poll(out: *mut RawInput);
    fn cmvs_vita_present(
        rgba: *const u8,
        width: u32,
        height: u32,
        view: *const f32,
        linear: i32,
        cursor: *const f32,
    ) -> i32;
    fn cmvs_vita_close();
}

/// The 960x544 screen and its controls. The system never asks a Vita application to
/// exit: closing it from LiveArea ends the process, so [`PadInput::exit`] stays false.
pub struct VitaWindow {
    _thread: PhantomData<Rc<()>>,
}
impl VitaWindow {
    pub fn open() -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Vita window already open");
        }
        // SAFETY: C rejects a second owner and cleans up partial initialization.
        if unsafe { cmvs_vita_open() } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("vita2d initialization failed");
        }
        OPEN.store(true, Ordering::Release);
        Ok(Self {
            _thread: PhantomData,
        })
    }
    /// The GPU compositor on this window's display. It closes with the window.
    pub fn open_gpu(&mut self) -> Result<super::VitaGpu, String> {
        super::VitaGpu::open()
    }
    pub fn poll(&mut self) -> PadInput {
        let mut raw = RawInput::default();
        // SAFETY: C writes exactly one `CmvsVitaInput`, a layout this struct mirrors.
        unsafe { cmvs_vita_poll(&mut raw) };
        let count = (raw.touch_count as usize).min(TOUCHES);
        PadInput {
            exit: false,
            focused: raw.flags & FLAG_FOCUSED != 0,
            buttons: raw.buttons,
            sticks: raw.sticks,
            touches: raw.touches[..count]
                .iter()
                .map(|&[id, x, y]| Touch {
                    id,
                    position: [x, y],
                })
                .collect(),
        }
    }
    /// Shows `rgba` (`size` pixels, tightly packed) scaled into `view` (left, top,
    /// width, height in screen pixels) on black, with an optional crosshair at a
    /// screen position. Waits for the previous frame's GPU work.
    pub fn present(
        &mut self,
        rgba: &[u8],
        size: [u32; 2],
        view: [f32; 4],
        linear: bool,
        cursor: Option<[f32; 2]>,
    ) -> Result<(), &'static str> {
        let [width, height] = size;
        if width == 0
            || height == 0
            || width > MAX_TEXTURE
            || height > MAX_TEXTURE
            || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err("invalid Vita frame size");
        }
        let cursor_ptr = cursor.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        // SAFETY: C copies width*height*4 bytes, reads four view floats and, when
        // non-null, two cursor floats; it keeps no pointer.
        match unsafe {
            cmvs_vita_present(
                rgba.as_ptr(),
                width,
                height,
                view.as_ptr(),
                i32::from(linear),
                cursor_ptr,
            )
        } {
            0 => Ok(()),
            -2 => Err("Vita video memory exhausted"),
            _ => Err("Vita present failed"),
        }
    }
}
impl Drop for VitaWindow {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::Release);
        // SAFETY: this value is the unique owner created by a successful open.
        unsafe { cmvs_vita_close() };
        OWNED.store(false, Ordering::Release);
    }
}

/// The system's enter button: cross (most regions) or circle (Japan, Asia). Read when
/// the window opens; false before.
pub fn confirm_is_cross() -> bool {
    // SAFETY: reads a value cached by `cmvs_vita_open`.
    unsafe { cmvs_vita_confirm_is_cross() != 0 }
}

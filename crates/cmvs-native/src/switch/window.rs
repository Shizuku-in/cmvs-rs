//! Safe ownership of the single libnx display. Calls stay on the creating thread.
#![allow(unsafe_code)]
use super::{SwitchInput, Touch};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);

const TOUCHES: usize = 4;
/// Mirrors `CmvsNxInput` in platforms/switch/runtime/window.c.
#[repr(C)]
#[derive(Default)]
struct NxInput {
    buttons: u32,
    flags: u32,
    sticks: [[i32; 2]; 2],
    touch_count: u32,
    touches: [[u32; 3]; TOUCHES],
}
const FLAG_EXIT: u32 = 1;
const FLAG_FOCUSED: u32 = 2;

unsafe extern "C" {
    fn cmvs_nx_open(framebuffer: i32) -> i32;
    fn cmvs_nx_poll(out: *mut NxInput);
    fn cmvs_nx_present(pixels: *const u32);
    fn cmvs_nx_close();
}

pub struct SwitchWindow {
    framebuffer: bool,
    _thread: PhantomData<Rc<()>>,
}
impl SwitchWindow {
    /// Software presentation through [`SwitchWindow::present`]. Also locks application
    /// exit until drop, so closing from HOME is reported by [`SwitchWindow::poll`]
    /// instead of terminating the process (system timeout 15 s).
    pub fn open() -> Result<Self, &'static str> {
        Self::open_with(true)
    }
    /// Input and applet handling only, leaving the display to the deko3d compositor.
    pub fn open_gpu() -> Result<(Self, super::gpu_device::SwitchGpu), String> {
        let window = Self::open_with(false)?;
        let gpu = super::gpu_device::SwitchGpu::open()?;
        Ok((window, gpu))
    }
    fn open_with(framebuffer: bool) -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Switch window already open");
        }
        // SAFETY: C rejects a second owner and cleans up partial initialization.
        if unsafe { cmvs_nx_open(i32::from(framebuffer)) } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("libnx framebuffer initialization failed");
        }
        Ok(Self {
            framebuffer,
            _thread: PhantomData,
        })
    }
    pub fn poll(&mut self) -> SwitchInput {
        let mut raw = NxInput::default();
        // SAFETY: C writes exactly one `CmvsNxInput`, a layout this struct mirrors.
        unsafe { cmvs_nx_poll(&mut raw) };
        let count = (raw.touch_count as usize).min(TOUCHES);
        SwitchInput {
            exit: raw.flags & FLAG_EXIT != 0,
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
    pub fn present(&mut self, pixels: &[u32]) -> Result<(), &'static str> {
        if !self.framebuffer {
            return Err("Switch window was opened for the GPU");
        }
        if pixels.len() != 1280 * 720 {
            return Err("invalid Switch canvas length");
        }
        // SAFETY: C synchronously reads precisely 1280*720 pixels; does not retain pointer.
        unsafe { cmvs_nx_present(pixels.as_ptr()) };
        Ok(())
    }
}
impl Drop for SwitchWindow {
    fn drop(&mut self) {
        // SAFETY: this value is the unique owner created by a successful open.
        unsafe { cmvs_nx_close() };
        OWNED.store(false, Ordering::Release);
    }
}

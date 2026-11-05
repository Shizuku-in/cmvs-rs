//! Safe ownership of the single libnx display. Calls stay on the creating thread.
#![allow(unsafe_code)]
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);
unsafe extern "C" {
    fn cmvs_nx_open() -> i32;
    fn cmvs_nx_poll() -> u32;
    fn cmvs_nx_present(pixels: *const u32);
    fn cmvs_nx_close();
}
pub struct SwitchWindow(PhantomData<Rc<()>>);
impl SwitchWindow {
    pub fn open() -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Switch window already open");
        }
        // C rejects a second owner and cleans up partial initialization.
        if unsafe { cmvs_nx_open() } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("libnx framebuffer initialization failed");
        }
        Ok(Self(PhantomData))
    }
    pub fn poll(&mut self) -> u32 {
        unsafe { cmvs_nx_poll() }
    }
    pub fn present(&mut self, pixels: &[u32]) -> Result<(), &'static str> {
        if pixels.len() != 1280 * 720 {
            return Err("invalid Switch canvas length");
        }
        // C synchronously reads precisely 1280*720 pixels; does not retain pointer.
        unsafe { cmvs_nx_present(pixels.as_ptr()) };
        Ok(())
    }
}
impl Drop for SwitchWindow {
    fn drop(&mut self) {
        unsafe { cmvs_nx_close() };
        OWNED.store(false, Ordering::Release);
    }
}

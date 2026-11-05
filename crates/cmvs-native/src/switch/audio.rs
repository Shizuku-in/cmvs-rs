//! libnx audout: 48 kHz stereo signed 16-bit output fed in fixed-size blocks.
#![allow(unsafe_code)]
use std::sync::atomic::{AtomicBool, Ordering};
static OWNED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn cmvs_nx_audio_open(rate: *mut u32, channels: *mut u32, frames: *mut u32) -> i32;
    fn cmvs_nx_audio_submit(pcm: *const i16) -> i32;
    fn cmvs_nx_audio_prefer_core() -> i32;
    fn cmvs_nx_audio_close();
}

/// The single audio output. It may move to an audio thread; it is not shared.
pub struct SwitchAudioOut {
    rate: u32,
    channels: u16,
    frames: usize,
}
impl SwitchAudioOut {
    pub fn open() -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Switch audio output already open");
        }
        let (mut rate, mut channels, mut frames) = (0, 0, 0);
        // SAFETY: C writes three u32 outputs and cleans up on failure.
        if unsafe { cmvs_nx_audio_open(&mut rate, &mut channels, &mut frames) } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("libnx audio output initialization failed");
        }
        Ok(Self {
            rate,
            channels: channels as u16,
            frames: frames as usize,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Interleaved samples per [`SwitchAudioOut::submit`] call.
    pub fn block_len(&self) -> usize {
        self.frames * usize::from(self.channels)
    }
    /// Moves the calling thread to CPU core 1, away from the main thread. Call it
    /// from the thread that feeds this output; false leaves the thread unchanged.
    pub fn prefer_audio_core(&self) -> bool {
        // SAFETY: changes only the calling thread's core mask.
        unsafe { cmvs_nx_audio_prefer_core() == 0 }
    }
    /// Queues one block, waiting while the device queue is full.
    pub fn submit(&mut self, pcm: &[i16]) -> Result<(), &'static str> {
        if pcm.len() != self.block_len() {
            return Err("invalid Switch audio block length");
        }
        // SAFETY: C copies exactly `block_len` samples before returning.
        if unsafe { cmvs_nx_audio_submit(pcm.as_ptr()) } != 0 {
            return Err("libnx audio output failed");
        }
        Ok(())
    }
}
impl Drop for SwitchAudioOut {
    fn drop(&mut self) {
        // SAFETY: unique owner from a successful open.
        unsafe { cmvs_nx_audio_close() };
        OWNED.store(false, Ordering::Release);
    }
}

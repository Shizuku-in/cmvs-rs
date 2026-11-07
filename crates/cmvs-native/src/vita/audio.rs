//! SceAudioOut: one 48 kHz stereo signed 16-bit BGM port fed in fixed-size blocks.
#![allow(unsafe_code)]
use std::sync::atomic::{AtomicBool, Ordering};
static OWNED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn cmvs_vita_audio_open(rate: *mut u32, channels: *mut u32, frames: *mut u32) -> i32;
    fn cmvs_vita_audio_submit(pcm: *const i16) -> i32;
    fn cmvs_vita_audio_close();
}

/// The single audio output. It may move to an audio thread; it is not shared.
pub struct VitaAudioOut {
    rate: u32,
    channels: u16,
    frames: usize,
}
impl VitaAudioOut {
    pub fn open() -> Result<Self, &'static str> {
        if OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("Vita audio output already open");
        }
        let (mut rate, mut channels, mut frames) = (0, 0, 0);
        // SAFETY: C writes three u32 outputs and leaves nothing open on failure.
        if unsafe { cmvs_vita_audio_open(&mut rate, &mut channels, &mut frames) } != 0 {
            OWNED.store(false, Ordering::Release);
            return Err("Vita audio port could not be opened");
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
    /// Interleaved samples per [`VitaAudioOut::submit`] call.
    pub fn block_len(&self) -> usize {
        self.frames * usize::from(self.channels)
    }
    /// Queues one block, waiting while the previous one plays.
    pub fn submit(&mut self, pcm: &[i16]) -> Result<(), &'static str> {
        if pcm.len() != self.block_len() {
            return Err("invalid Vita audio block length");
        }
        // SAFETY: C copies exactly `block_len` samples before returning.
        if unsafe { cmvs_vita_audio_submit(pcm.as_ptr()) } != 0 {
            return Err("Vita audio output failed");
        }
        Ok(())
    }
}
impl Drop for VitaAudioOut {
    fn drop(&mut self) {
        // SAFETY: unique owner from a successful open.
        unsafe { cmvs_vita_audio_close() };
        OWNED.store(false, Ordering::Release);
    }
}

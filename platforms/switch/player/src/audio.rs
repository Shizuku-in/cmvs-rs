//! Feeds the engine mixer to libnx audout from a dedicated thread.
use crate::sys::SwitchAudioOut;
use cmvs_audio::NativeAudio;
use cmvs_console::audio::{AudioOutputDevice, AudioThread};
use std::{error::Error, path::Path};

struct AudioDevice(SwitchAudioOut);

impl AudioOutputDevice for AudioDevice {
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.0.channels()
    }
    fn block_len(&self) -> usize {
        self.0.block_len()
    }
    fn on_thread_start(&mut self) {
        self.0.prefer_audio_core();
    }
    fn submit(&mut self, block: &[i16]) -> Result<(), &'static str> {
        self.0.submit(block)
    }
}

/// Opens the device and returns the engine backend plus its pumping thread.
pub fn start(root: &Path) -> Result<(NativeAudio, AudioThread), Box<dyn Error + Send + Sync>> {
    AudioThread::start(root, AudioDevice(SwitchAudioOut::open()?), None)
}

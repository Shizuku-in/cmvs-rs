//! Feeds the engine mixer to SceAudioOut from a dedicated thread on CPU core 1.
use crate::sys::{VitaAudioOut, prefer_core};
use cmvs_audio::NativeAudio;
use cmvs_console::audio::{AudioOutputDevice, AudioThread};
use std::{error::Error, path::Path};

/// The mixer decodes Vorbis as it fills blocks; this keeps it off the VM's core.
const AUDIO_CORE: u32 = 1;

struct AudioDevice(VitaAudioOut);

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
        prefer_core(AUDIO_CORE);
    }
    fn submit(&mut self, block: &[i16]) -> Result<(), &'static str> {
        self.0.submit(block)
    }
}

/// Opens the device and returns the engine backend plus its pumping thread.
pub fn start(root: &Path) -> Result<(NativeAudio, AudioThread), Box<dyn Error + Send + Sync>> {
    AudioThread::start(root, AudioDevice(VitaAudioOut::open()?), Some(256 * 1024))
}

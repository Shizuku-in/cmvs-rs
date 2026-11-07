//! Feeds the engine mixer to SceAudioOut from a dedicated thread on CPU core 1.
use crate::sys::{VitaAudioOut, prefer_core};
use cmvs_audio::NativeAudio;
use std::{
    error::Error,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

/// The mixer decodes Vorbis as it fills blocks; this keeps it off the VM's core.
const AUDIO_CORE: u32 = 1;

pub struct AudioThread {
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<&'static str>>>,
    handle: Option<JoinHandle<()>>,
}
impl AudioThread {
    /// Opens the device and returns the engine backend that mixes into it.
    pub fn start(root: &Path) -> Result<(NativeAudio, Self), Box<dyn Error>> {
        let mut output = VitaAudioOut::open()?;
        let (audio, mut pcm) = NativeAudio::pull(root, output.sample_rate(), output.channels())?;
        let stop = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Mutex::new(None));
        let (stopping, failure) = (stop.clone(), fault.clone());
        let handle = std::thread::Builder::new()
            .name("audio".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                prefer_core(AUDIO_CORE);
                let mut block = vec![0; output.block_len()];
                while !stopping.load(Ordering::Relaxed) {
                    pcm.fill_i16(&mut block);
                    if let Err(error) = output.submit(&block) {
                        *failure.lock().unwrap() = Some(error);
                        break;
                    }
                }
            })?;
        Ok((
            audio,
            Self {
                stop,
                fault,
                handle: Some(handle),
            },
        ))
    }
    /// Reports a device failure from the audio thread.
    pub fn check(&self) -> Result<(), Box<dyn Error>> {
        match *self.fault.lock().unwrap() {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}
impl Drop for AudioThread {
    fn drop(&mut self) {
        // A submit waits for at most one playing block, so this joins promptly.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

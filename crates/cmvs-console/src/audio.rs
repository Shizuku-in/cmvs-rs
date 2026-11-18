//! Background audio pump shared by console players without native OS device drivers.
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

/// Fixed-block PCM output endpoint driven from [`AudioThread`].
pub trait AudioOutputDevice: Send + 'static {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn block_len(&self) -> usize;
    fn on_thread_start(&mut self) {}
    fn submit(&mut self, block: &[i16]) -> Result<(), &'static str>;
}

/// Owns the audio thread pumping mixer output blocks to an [`AudioOutputDevice`].
pub struct AudioThread {
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<&'static str>>>,
    handle: Option<JoinHandle<()>>,
}

impl AudioThread {
    /// Opens the mixer for `output` and starts a background pump thread.
    pub fn start<D: AudioOutputDevice>(
        root: &Path,
        mut output: D,
        stack_size: Option<usize>,
    ) -> Result<(NativeAudio, Self), Box<dyn Error + Send + Sync>> {
        let (audio, mut pcm) = NativeAudio::pull(root, output.sample_rate(), output.channels())?;
        let stop = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Mutex::new(None));
        let (stopping, failure) = (stop.clone(), fault.clone());
        let mut builder = std::thread::Builder::new().name("audio".into());
        if let Some(size) = stack_size {
            builder = builder.stack_size(size);
        }
        let handle = builder.spawn(move || {
            output.on_thread_start();
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
    pub fn check(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        match *self.fault.lock().unwrap() {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc::{self, SyncSender},
        time::{Duration, Instant},
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    struct FakeAudioDevice {
        blocks: usize,
        fail_after: Option<usize>,
        submitted: SyncSender<()>,
    }

    impl AudioOutputDevice for FakeAudioDevice {
        fn sample_rate(&self) -> u32 {
            48_000
        }
        fn channels(&self) -> u16 {
            2
        }
        fn block_len(&self) -> usize {
            64
        }
        fn submit(&mut self, block: &[i16]) -> Result<(), &'static str> {
            assert_eq!(block.len(), 64);
            let index = self.blocks;
            self.blocks += 1;
            let _ = self.submitted.try_send(());
            if self.fail_after.is_some_and(|limit| index >= limit) {
                Err("synthetic device failure")
            } else {
                std::thread::sleep(Duration::from_millis(1));
                Ok(())
            }
        }
    }

    #[test]
    fn audio_thread_reports_device_failures_and_stops_cleanly() {
        let root = std::env::temp_dir().join(format!("cmvs-console-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let (tx, rx) = mpsc::sync_channel(1);
        let (_audio, thread) = AudioThread::start(
            &root,
            FakeAudioDevice {
                blocks: 0,
                fail_after: None,
                submitted: tx,
            },
            None,
        )
        .unwrap();
        rx.recv_timeout(TEST_TIMEOUT)
            .expect("audio thread submitted initial block");
        assert!(thread.check().is_ok());
        drop(thread);

        let (fail_tx, fail_rx) = mpsc::sync_channel(1);
        let (_audio, failing) = AudioThread::start(
            &root,
            FakeAudioDevice {
                blocks: 0,
                fail_after: Some(0),
                submitted: fail_tx,
            },
            Some(256 * 1024),
        )
        .unwrap();
        fail_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("audio thread attempted failing submit");
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            match failing.check() {
                Ok(()) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(()) => panic!("timed out waiting for audio thread device failure"),
                Err(error) => {
                    assert!(error.to_string().contains("synthetic device failure"));
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

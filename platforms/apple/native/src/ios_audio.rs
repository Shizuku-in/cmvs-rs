// Keep the game mixer alive while replacing the RemoteIO unit on a new route.
use crate::Result;
use cmvs_audio::{NativeAudio, PcmOutput};
use cpal::{
    BufferSize, ErrorKind, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub struct Output {
    pcm: Arc<Mutex<PcmOutput>>,
    fault: Arc<Mutex<Option<cpal::Error>>>,
    stream: Option<Stream>,
    sample_rate: u32,
}

impl Output {
    pub fn new(root: &Path) -> Result<(NativeAudio, Self)> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output")?;
        let sample_rate = device.default_output_config()?.sample_rate();
        let (audio, pcm) = NativeAudio::pull(root, sample_rate, 2)?;
        let mut output = Self {
            pcm: Arc::new(Mutex::new(pcm)),
            fault: Default::default(),
            stream: None,
            sample_rate,
        };
        output.restart()?;
        Ok((audio, output))
    }

    pub fn restart(&mut self) -> Result<()> {
        self.stream.take();
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output")?;
        let pcm = self.pcm.clone();
        let fault = Arc::default();
        self.fault = Arc::clone(&fault);
        let stream = device.build_output_stream(
            StreamConfig {
                channels: 2,
                // RemoteIO converts the client format to the route format. Keep the live mixer
                // at its original rate across route changes so playback speed stays correct.
                sample_rate: self.sample_rate,
                buffer_size: BufferSize::Default,
            },
            move |samples: &mut [i16], _| match pcm.try_lock() {
                Ok(mut pcm) => pcm.fill_i16(samples),
                Err(_) => samples.fill(0),
            },
            move |error| *fault.lock().unwrap_or_else(|e| e.into_inner()) = Some(error),
            None,
        )?;
        stream.play()?;
        self.stream = Some(stream);
        Ok(())
    }
    pub fn needs_restart(&self) -> Result<bool> {
        if let Some(error) = self.fault.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return match error.kind() {
                ErrorKind::DeviceChanged
                | ErrorKind::StreamInvalidated
                | ErrorKind::DeviceNotAvailable => Ok(true),
                _ => Err(error.into()),
            };
        }
        Ok(false)
    }
}

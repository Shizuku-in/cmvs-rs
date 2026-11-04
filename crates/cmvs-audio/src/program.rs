//! Bounded, device-independent Vorbis intro and loop playback.
use crate::vorbis::Vorbis;
use cmvs_core::MAX_PAYLOAD;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Holds compressed bytes and one decoded packet, never an entire PCM recording.
pub struct VorbisProgram {
    pub(crate) bytes: Vec<Arc<[u8]>>,
    reader: Vorbis,
    pub(crate) looped: Arc<AtomicBool>,
    buffer: Vec<f32>,
    at: usize,
    cycle_has_pcm: bool,
    finished: bool,
    channels: u16,
    rate: u32,
}
impl VorbisProgram {
    /// With looping enabled, primary plays once, then secondary repeats. Without
    /// looping the original does not switch to secondary (MOG_Replay is never called).
    pub fn new(
        primary: Vec<u8>,
        secondary: Option<Vec<u8>>,
        looped: bool,
    ) -> Result<Self, Box<dyn Error>> {
        if primary
            .len()
            .checked_add(secondary.as_ref().map_or(0, Vec::len))
            .is_none_or(|n| n > MAX_PAYLOAD)
        {
            return Err("combined Ogg resources exceed research limit".into());
        }
        let mut bytes: Vec<Arc<[u8]>> = vec![primary.into()];
        if let Some(secondary) = secondary {
            bytes.push(secondary.into());
        }
        Self::from_shared(bytes, looped)
    }
    pub(crate) fn from_shared(bytes: Vec<Arc<[u8]>>, looped: bool) -> Result<Self, Box<dyn Error>> {
        let reader = Vorbis::new(bytes[0].clone())?;
        let channels = reader.channels();
        let rate = reader.sample_rate();
        if bytes.len() == 2 {
            let mut second = Vorbis::new(bytes[1].clone())?;
            if second.channels() != channels || second.sample_rate() != rate {
                return Err("BGM intro and loop formats differ".into());
            }
            // Validate the first actual packet before changing a live channel.
            let mut valid = false;
            for _ in 0..1024 {
                match second.packet()? {
                    Some(packet) if !packet.is_empty() => {
                        valid = true;
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            if !valid {
                return Err("BGM loop stream contains no PCM samples".into());
            }
        }
        let mut result = Self {
            bytes,
            reader,
            looped: Arc::new(AtomicBool::new(looped)),
            buffer: vec![],
            at: 0,
            cycle_has_pcm: false,
            finished: false,
            channels,
            rate,
        };
        result.refill()?;
        Ok(result)
    }
    /// Prepare playback from the beginning without changing the current decoder.
    /// Compressed data is shared; decoder state and loop control are independent.
    pub fn restarted(&self, looped: bool) -> Result<Self, Box<dyn Error>> {
        Self::from_shared(self.bytes.clone(), looped)
    }
    pub fn set_looped(&self, value: bool) {
        self.looped.store(value, Ordering::Relaxed);
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn refill(&mut self) -> Result<(), Box<dyn Error>> {
        if self.finished {
            return Ok(());
        }
        self.buffer.clear();
        self.at = 0;
        for _ in 0..1024 {
            match self.reader.packet()? {
                Some(samples) if !samples.is_empty() => {
                    self.cycle_has_pcm = true;
                    self.buffer = samples;
                    return Ok(());
                }
                Some(_) => {}
                None => {
                    if !self.cycle_has_pcm {
                        return Err("Vorbis stream contains no PCM samples".into());
                    }
                    if !self.looped.load(Ordering::Relaxed) {
                        self.finished = true;
                        return Ok(());
                    }
                    self.reader = Vorbis::new(self.bytes.last().unwrap().clone())?;
                    self.cycle_has_pcm = false;
                }
            }
        }
        Err("Vorbis produced too many empty packets".into())
    }
    pub fn next_sample(&mut self) -> Result<Option<f32>, Box<dyn Error>> {
        if self.at == self.buffer.len() {
            self.refill()?;
        }
        let sample = self.buffer.get(self.at).copied();
        if sample.is_some() {
            self.at += 1;
        }
        Ok(sample)
    }
}

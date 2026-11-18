//! Bounded, device-independent Vorbis/PCM WAVE intro and loop playback.
use crate::{pcm_wave::PcmWave, vorbis::Vorbis};
use cmvs_core::{MAX_PAYLOAD, format::mv2};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

enum AudioReader {
    Vorbis(Box<Vorbis>),
    Wave(PcmWave),
    Mv2(Box<mv2::Decoder>),
}
impl AudioReader {
    fn new(bytes: Arc<[u8]>) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if bytes.starts_with(b"RIFF") {
            Ok(Self::Wave(PcmWave::new(bytes)?))
        } else if bytes.starts_with(b"MV2X") {
            Ok(Self::Mv2(Box::new(mv2::Decoder::new(bytes)?)))
        } else {
            Ok(Self::Vorbis(Box::new(Vorbis::new(bytes)?)))
        }
    }
    fn channels(&self) -> u16 {
        match self {
            Self::Vorbis(v) => v.channels(),
            Self::Wave(w) => w.channels,
            Self::Mv2(m) => u16::from(m.header().channels),
        }
    }
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Vorbis(v) => v.sample_rate(),
            Self::Wave(w) => w.rate,
            Self::Mv2(m) => u32::from(m.header().sample_rate),
        }
    }
    fn packet(&mut self) -> Result<Option<Vec<f32>>, Box<dyn Error + Send + Sync>> {
        match self {
            Self::Vorbis(v) => v.packet(),
            Self::Wave(w) => Ok(w.packet()),
            Self::Mv2(m) => Ok(m
                .next_frame()?
                .map(|pcm| pcm.into_iter().map(|s| f32::from(s) / 32768.0).collect())),
        }
    }
}

/// Streams Ogg Vorbis or PCM WAVE audio, retaining source bytes and one decoded block.
pub struct AudioProgram {
    pub(crate) bytes: Vec<Arc<[u8]>>,
    reader: AudioReader,
    pub(crate) looped: Arc<AtomicBool>,
    loop_skip_samples: usize,
    buffer: Vec<f32>,
    at: usize,
    cycle_has_pcm: bool,
    finished: bool,
    channels: u16,
    rate: u32,
}
impl AudioProgram {
    /// With looping enabled, primary plays once, then secondary repeats. Without
    /// looping the original does not switch to secondary (MOG_Replay is never called).
    pub fn new(
        primary: Vec<u8>,
        secondary: Option<Vec<u8>>,
        looped: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::with_loop_frames(primary, secondary, looped, 0)
    }
    /// Variant of [`Self::new`] for CVNS `0x20a7`, skipping `loop_frames` sample frames on
    /// each subsequent loop rewind (`sub_4190C0` / `sub_419240`).
    pub fn with_loop_frames(
        primary: Vec<u8>,
        secondary: Option<Vec<u8>>,
        looped: bool,
        loop_frames: u32,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if primary
            .len()
            .checked_add(secondary.as_ref().map_or(0, Vec::len))
            .is_none_or(|n| n > MAX_PAYLOAD)
        {
            return Err("combined audio resources exceed research limit".into());
        }
        let mut bytes: Vec<Arc<[u8]>> = vec![primary.into()];
        if let Some(secondary) = secondary {
            bytes.push(secondary.into());
        }
        Self::from_shared_with_loop(bytes, looped, loop_frames)
    }
    pub(crate) fn from_shared(
        bytes: Vec<Arc<[u8]>>,
        looped: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::from_shared_with_loop(bytes, looped, 0)
    }
    fn from_shared_with_loop(
        bytes: Vec<Arc<[u8]>>,
        looped: bool,
        loop_frames: u32,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let reader = AudioReader::new(bytes[0].clone())?;
        let channels = reader.channels();
        let rate = reader.sample_rate();
        if bytes.len() == 2 {
            let mut second = AudioReader::new(bytes[1].clone())?;
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
            loop_skip_samples: (loop_frames as usize).saturating_mul(usize::from(channels)),
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
    pub fn restarted(&self, looped: bool) -> Result<Self, Box<dyn Error + Send + Sync>> {
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
    /// Length of the primary track as interleaved 16-bit PCM, before looping or mixing.
    pub fn primary_pcm_bytes(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        match AudioReader::new(self.bytes[0].clone())? {
            AudioReader::Mv2(m) => {
                let h = m.header();
                Ok(h.samples_per_channel * u32::from(h.channels) * 2)
            }
            AudioReader::Wave(w) => Ok(w.pcm_bytes),
            AudioReader::Vorbis(mut v) => {
                // Count decoder output, including any initial block padding.
                let mut bytes = 0u32;
                while let Some(packet) = v.packet()? {
                    bytes = bytes
                        .checked_add(packet.len() as u32 * 2)
                        .ok_or("voice PCM byte count exceeds u32")?;
                }
                Ok(bytes)
            }
        }
    }
    fn refill(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.finished {
            return Ok(());
        }
        self.buffer.clear();
        self.at = 0;
        let mut remaining_skip = 0usize;
        let mut empty_packets = 0usize;
        for _ in 0..65_536 {
            match self.reader.packet()? {
                Some(samples) if !samples.is_empty() => {
                    empty_packets = 0;
                    self.cycle_has_pcm = true;
                    if remaining_skip >= samples.len() {
                        remaining_skip -= samples.len();
                        continue;
                    }
                    self.buffer = if remaining_skip > 0 {
                        samples[remaining_skip..].to_vec()
                    } else {
                        samples
                    };
                    return Ok(());
                }
                Some(_) => {
                    empty_packets += 1;
                    if empty_packets >= 1024 {
                        return Err("Vorbis produced too many empty packets".into());
                    }
                }
                None => {
                    if !self.cycle_has_pcm {
                        return Err("Vorbis stream contains no PCM samples".into());
                    }
                    if !self.looped.load(Ordering::Relaxed) {
                        self.finished = true;
                        return Ok(());
                    }
                    self.reader = AudioReader::new(self.bytes.last().unwrap().clone())?;
                    self.cycle_has_pcm = false;
                    remaining_skip = self.loop_skip_samples;
                    empty_packets = 0;
                }
            }
        }
        Err("Vorbis produced too many skipped packets".into())
    }
    pub fn next_sample(&mut self) -> Result<Option<f32>, Box<dyn Error + Send + Sync>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_mv2(samples: u32, rate: u16, channels: u8, active_bit: bool) -> Vec<u8> {
        let mut out = b"MV2X".to_vec();
        out.extend(samples.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.push(channels);
        out.push(4);
        out.extend(1u32.to_le_bytes());
        // Each channel header in frame 0 takes 19 bits (+3 bits if non-zero coeff 0):
        // bit 0..10 = active, bit 10..19 = quant_index=178, unary '10', val '1'.
        let mut words = Vec::<u32>::new();
        let mut cur = 0u32;
        let mut used = 0u8;
        let mut push = |val: u32, count: u8| {
            for i in (0..count).rev() {
                cur |= ((val >> i) & 1) << used;
                used += 1;
                if used == 32 {
                    words.push(cur);
                    cur = 0;
                    used = 0;
                }
            }
        };
        for _ in 0..channels {
            if active_bit {
                push(1, 10);
                push(213, 9);
                push(0b10, 2);
                push(1, 1);
            } else {
                push(0, 10);
                push(0, 9);
            }
        }
        if used != 0 {
            words.push(cur);
        }
        for w in words {
            out.extend(w.to_le_bytes());
        }
        out
    }

    #[test]
    fn streams_single_shot_and_looping_mv2_programs() {
        let intro = synthetic_mv2(260, 22050, 1, true);
        let mut single = AudioProgram::new(intro.clone(), None, false).unwrap();
        assert_eq!(single.primary_pcm_bytes().unwrap(), 520);
        let stereo = AudioProgram::new(synthetic_mv2(260, 22050, 2, false), None, false).unwrap();
        assert_eq!(stereo.primary_pcm_bytes().unwrap(), 1040);
        assert_eq!(single.channels(), 1);
        assert_eq!(single.sample_rate(), 22050);
        let mut samples = Vec::new();
        while let Some(s) = single.next_sample().unwrap() {
            samples.push(s);
        }
        assert_eq!(samples.len(), 260);
        assert!(samples[256].abs() > 0.05);

        let loop_part = synthetic_mv2(2, 22050, 1, false);
        let mut looping = AudioProgram::new(intro, Some(loop_part), true).unwrap();
        let mut looped_samples = Vec::new();
        for _ in 0..266 {
            looped_samples.push(looping.next_sample().unwrap().unwrap());
        }
        assert!(looped_samples[256].abs() > 0.05);
        assert_eq!(&looped_samples[260..266], &[0.0; 6]);

        let mismatched = synthetic_mv2(2, 44100, 1, false);
        assert!(
            AudioProgram::new(synthetic_mv2(2, 22050, 1, false), Some(mismatched), true).is_err()
        );
    }

    #[test]
    fn cvns_loop_frames_skips_prefix_on_rewind() {
        let track = synthetic_mv2(260, 22050, 1, true);
        let mut program = AudioProgram::with_loop_frames(track, None, true, 256).unwrap();
        let mut out = Vec::new();
        for _ in 0..264 {
            out.push(program.next_sample().unwrap().unwrap());
        }
        assert!(out[256].abs() > 0.05);
        // Loop skipped initial 256 samples, so sample 260 is immediately the active frame 256.
        assert!(out[260].abs() > 0.05);
    }
}

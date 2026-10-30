//! Pitch-preserving voice time compression for 0x20af, from cmvs-20160201 x64:
//! setup 0x14008a3e0, input request 0x14008a5d0 and chunk processing 0x14008a630.
//! Positions keep the original's unsigned 32-bit arithmetic; reads past the valid
//! input and never-written output samples are zero where the original reuses stale
//! buffer contents.
use crate::FormatError;

/// 0x40000-byte float buffers for input (+0) and output (+16).
const BUFFER_SAMPLES: usize = 0x40000 / 4;
/// 0x20000-byte 16-bit result buffer (+32).
const RESULT_SAMPLES: usize = 0x20000 / 2;

/// Device mixer format (0x140089640): rate +2280 and stereo 16-bit chunk +2284.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceMixer {
    pub device_rate: u32,
    /// None when SOUND_BUFFER is outside 0..3 and the device leaves +2284 unset.
    pub chunk_bytes: Option<u32>,
}
/// Both keys default to 0 in 0x140011320's GetPrivateProfileIntA reads.
impl Default for VoiceMixer {
    fn default() -> Self {
        Self::from_ini(0, 0)
    }
}
impl VoiceMixer {
    /// 0x14000aa0b: SOUND_QUALITY 0 selects 22050 Hz, otherwise 44100. 0x140089870:
    /// SOUND_BUFFER 0..3 selects 0x1800/0x2000/0x3000/0x4000 bytes, halved at 22050 Hz.
    pub fn from_ini(quality: i32, buffer: i32) -> Self {
        let device_rate = if quality == 0 { 22050 } else { 44100 };
        let chunk = match buffer {
            0 => Some(0x1800),
            1 => Some(0x2000),
            2 => Some(0x3000),
            3 => Some(0x4000),
            _ => None,
        };
        Self {
            device_rate,
            chunk_bytes: chunk.map(|bytes| {
                if device_rate == 22050 {
                    bytes / 2
                } else {
                    bytes
                }
            }),
        }
    }
    /// 0x1400892f0: mono byte count asked of a voice per chunk, after the 22050/44100
    /// adjustment; other file rates are left unadjusted, as in the original.
    pub fn mono_request(self, file_rate: u32) -> Option<u32> {
        let mut bytes = self.chunk_bytes? >> 1;
        if self.device_rate == 22050 {
            if file_rate == 44100 {
                bytes *= 2;
            }
        } else if file_rate == 22050 {
            bytes >>= 1;
        }
        Some(bytes)
    }
}

#[derive(Debug, Clone)]
pub struct VoiceCompressor {
    rate: f32,
    template: u32,
    min_lag: u32,
    max_lag: u32,
    input: Vec<f32>,
    in_pos: u32,
    in_end: u32,
    output: Vec<f32>,
    out_pos: u32,
    out_end: u32,
    /// +40: bytes asked of the decoder for this chunk.
    requested: u32,
    /// +44: the decoder has not yet returned short.
    flowing: bool,
    /// +48: polled by the mixer, which then frees the compressor.
    ended: bool,
}

impl VoiceCompressor {
    /// 0x140088730..0x14008875b attaches it to mono sounds with 1.1 <= rate <= 3.0
    /// (single precision, so NaN never attaches).
    pub fn new(rate: f32, sample_rate: u32, channels: u16) -> Result<Option<Self>, FormatError> {
        if channels != 1 || !(1.1..=3.0).contains(&rate) {
            return Ok(None);
        }
        // 0x14008a46a..0x14008a4b0: float sample rate widened, scaled in double, truncated.
        let scaled = |factor: f64| (f64::from(sample_rate as f32) * factor) as i32;
        let (template, min_lag, max_lag) = (scaled(0.005), scaled(0.0025), scaled(0.01));
        if template <= 0 || min_lag <= 0 || max_lag as usize > BUFFER_SAMPLES / 16 {
            return Err(FormatError(
                "voice compression sample rate outside research range",
            ));
        }
        Ok(Some(Self {
            rate,
            template: template as u32,
            min_lag: min_lag as u32,
            max_lag: max_lag as u32,
            input: vec![0.0; BUFFER_SAMPLES],
            in_pos: 0,
            in_end: 0,
            output: vec![0.0; BUFFER_SAMPLES],
            out_pos: 0,
            out_end: 0,
            requested: 0,
            flowing: true,
            ended: false,
        }))
    }
    pub fn ended(&self) -> bool {
        self.ended
    }
    /// 0x14008a5d0: decoder bytes to read before processing `need` output bytes.
    pub fn request(&mut self, need: u32) -> u32 {
        let buffered = 2 * self.out_end.wrapping_sub(self.out_pos);
        self.requested =
            if buffered < need && self.in_end.wrapping_sub(self.in_pos) < 0x4000 && self.flowing {
                let deficit = need - buffered;
                if f64::from(self.rate) >= 2.1 {
                    8 * deficit
                } else {
                    5 * deficit
                }
            } else {
                0
            };
        self.requested
    }
    fn sample(&self, at: u32) -> f32 {
        self.input.get(at as usize).copied().unwrap_or(0.0)
    }
    fn write_output(&mut self, at: u32, value: f32) -> Result<(), FormatError> {
        *self.output.get_mut(at as usize).ok_or(FormatError(
            "voice compression output exceeds the device buffer",
        ))? = value;
        Ok(())
    }
    /// 0x14008a630: `need` output bytes from the `decoded` 16-bit samples the decoder
    /// returned for the last request. Samples the original leaves stale are zero.
    pub fn process(&mut self, need: u32, decoded: &[i16]) -> Result<Vec<i16>, FormatError> {
        let decoded_bytes = u32::try_from(decoded.len() * 2)
            .map_err(|_| FormatError("decoded voice chunk exceeds research limit"))?;
        let need = need >> 1;
        if need as usize > RESULT_SAMPLES {
            return Err(FormatError(
                "voice compression chunk exceeds the result buffer",
            ));
        }
        let mut result = vec![0i16; need as usize];
        let mut pad = false;
        if self.flowing && decoded_bytes < self.requested {
            self.flowing = false;
            pad = true;
        }
        let pending = self.out_end.wrapping_sub(self.out_pos);
        let mut written = 0;
        if pending != 0 {
            written = pending;
            if pending >= need {
                self.convert(self.out_pos, &mut result[..need as usize]);
                self.out_pos += need;
                return Ok(result);
            }
            self.convert(self.out_pos, &mut result[..pending as usize]);
        }
        self.out_pos = 0;
        self.out_end = 0;
        let left = self.in_end.wrapping_sub(self.in_pos);
        if left != 0 {
            let (from, len) = (self.in_pos as usize, left as usize);
            if from + len > BUFFER_SAMPLES {
                return Err(FormatError(
                    "voice compression input left the device buffer",
                ));
            }
            self.input.copy_within(from..from + len, 0);
        }
        self.in_end = left;
        self.in_pos = 0;
        let end = self.in_end as usize;
        let added = decoded.len() + if pad { 4 * self.max_lag as usize } else { 0 };
        if end + added > BUFFER_SAMPLES {
            return Err(FormatError(
                "voice compression input exceeds the device buffer",
            ));
        }
        for (slot, &sample) in self.input[end..].iter_mut().zip(decoded) {
            *slot = f32::from(sample) * (1.0 / 32768.0);
        }
        self.in_end += decoded.len() as u32;
        if pad {
            let end = self.in_end as usize;
            self.input[end..end + 4 * self.max_lag as usize].fill(0.0);
            self.in_end += 2 * self.max_lag;
        }
        let remaining = need - written;
        if self.out_end.wrapping_sub(self.out_pos) < remaining {
            loop {
                if !(self.flowing || 2 * self.max_lag <= self.in_end.wrapping_sub(self.in_pos)) {
                    self.ended = true;
                    break;
                }
                self.step()?;
                if self.out_end.wrapping_sub(self.out_pos) >= remaining {
                    break;
                }
            }
        }
        let produced = self.out_end.wrapping_sub(self.out_pos);
        if produced != 0 && written < need {
            if remaining > produced {
                let range = written as usize..(written + produced) as usize;
                self.convert(self.out_pos, &mut result[range]);
                self.out_pos = 0;
                self.out_end = 0;
            } else {
                self.convert(self.out_pos, &mut result[written as usize..]);
                self.out_pos += remaining;
            }
        }
        Ok(result)
    }
    /// One overlap step: best positive correlation lag, linear cross-fade over it,
    /// then a straight copy so that input advances `rate` times the output.
    fn step(&mut self) -> Result<(), FormatError> {
        let base = self.in_pos;
        let mut best = 0.0f32;
        let mut lag = self.min_lag;
        for candidate in self.min_lag..=self.max_lag {
            let mut sum = 0.0f32;
            for k in 0..self.template {
                sum += self.sample(base + k) * self.sample(base + candidate + k);
            }
            if sum > best {
                best = sum;
                lag = candidate;
            }
        }
        let out = self.out_end;
        for i in 0..lag {
            let value = ((lag - i) as i32 as f32 * self.sample(base + i)
                + i as i32 as f32 * self.sample(base + lag + i))
                / lag as i32 as f32;
            self.write_output(out + i, value)?;
        }
        // 0x14008b022..0x14008b035: double precision, rounded by +0.5 and truncation.
        let copy = (f64::from(lag) / (f64::from(self.rate) - 1.0) + 0.5) as i32 as u32;
        let mut at = lag;
        while at < copy {
            if !(self.flowing || lag + at < self.in_end.wrapping_sub(self.in_pos)) {
                self.ended = true;
                // The original counts the unwritten tail as output anyway.
                while at < copy {
                    self.write_output(out + at, 0.0)?;
                    at += 1;
                }
                break;
            }
            let value = self.sample(base + lag + at);
            self.write_output(out + at, value)?;
            at += 1;
        }
        self.in_pos = self.in_pos.wrapping_add(copy + lag);
        self.out_end = self.out_end.wrapping_add(copy);
        Ok(())
    }
    fn convert(&self, from: u32, target: &mut [i16]) {
        for (i, slot) in target.iter_mut().enumerate() {
            let value = self.output.get(from as usize + i).copied().unwrap_or(0.0) * 32767.0;
            // CVTTSS2SI yields i32::MIN out of range, then the branchless clamp.
            let raw = if (-2147483648.0..2147483648.0).contains(&value) {
                value as i32
            } else {
                i32::MIN
            };
            *slot = raw.clamp(-32768, 32767) as i16;
        }
    }
}

//! Bounded RIFF/WAVE PCM reader. Samples are decoded in small blocks.
use std::{error::Error, sync::Arc};

pub(crate) struct PcmWave {
    bytes: Arc<[u8]>,
    at: usize,
    end: usize,
    pub channels: u16,
    pub rate: u32,
}
impl PcmWave {
    pub fn new(bytes: Arc<[u8]>) -> Result<Self, Box<dyn Error>> {
        if bytes.len() > cmvs_core::MAX_PAYLOAD
            || bytes.len() < 12
            || &bytes[..4] != b"RIFF"
            || &bytes[8..12] != b"WAVE"
        {
            return Err("invalid PCM WAVE header".into());
        }
        let u32le = |p: usize| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
        let end = (u32le(4) as usize)
            .checked_add(8)
            .ok_or("WAVE size overflow")?;
        if end < 12 || end > bytes.len() {
            return Err("truncated WAVE".into());
        }
        let mut at = 12;
        let mut format = None;
        let mut data = None;
        while at < end {
            if end - at < 8 {
                return Err("truncated WAVE chunk header".into());
            }
            let size = u32le(at + 4) as usize;
            let start = at + 8;
            let next = start
                .checked_add(size)
                .filter(|&n| n <= end)
                .ok_or("truncated WAVE chunk")?;
            match &bytes[at..at + 4] {
                b"fmt " => {
                    if format.is_some() || size < 16 {
                        return Err("invalid WAVE format chunk".into());
                    }
                    let word = |p: usize| {
                        u16::from_le_bytes(bytes[start + p..start + p + 2].try_into().unwrap())
                    };
                    let channels = word(2);
                    let rate = u32le(start + 4);
                    if word(0) != 1
                        || word(14) != 16
                        || !(1..=2).contains(&channels)
                        || !(8000..=192000).contains(&rate)
                        || word(12) != channels * 2
                        || u32le(start + 8) != rate * u32::from(channels) * 2
                    {
                        return Err(
                            "only mono/stereo 16-bit PCM WAVE at 8..192 kHz is supported".into(),
                        );
                    }
                    format = Some((channels, rate));
                }
                b"data" => {
                    if data.replace((start, next)).is_some() {
                        return Err("duplicate WAVE data chunk".into());
                    }
                }
                _ => {}
            }
            at = next
                .checked_add(size & 1)
                .filter(|&n| n <= end)
                .ok_or("missing WAVE chunk padding")?;
        }
        let (channels, rate) = format.ok_or("missing WAVE format")?;
        let (at, end) = data.ok_or("missing WAVE data")?;
        if at == end || !(end - at).is_multiple_of(usize::from(channels) * 2) {
            return Err("empty or incomplete WAVE PCM frame".into());
        }
        Ok(Self {
            bytes,
            at,
            end,
            channels,
            rate,
        })
    }
    pub fn packet(&mut self) -> Option<Vec<f32>> {
        if self.at == self.end {
            return None;
        }
        let end = (self.at + 8192).min(self.end);
        let samples = self.bytes[self.at..end]
            .chunks_exact(2)
            .map(|s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0)
            .collect();
        self.at = end;
        Some(samples)
    }
}

#[cfg(test)]
mod tests {
    use crate::AudioProgram;
    fn fixture() -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend(48u32.to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(44100u32.to_le_bytes());
        b.extend(176400u32.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(12u32.to_le_bytes());
        for s in [i16::MIN, i16::MAX, 0, 16384, -16384, 0] {
            b.extend(s.to_le_bytes());
        }
        b
    }
    #[test]
    fn pcm_samples_loop_restart_and_eof() {
        let mut p = AudioProgram::new(fixture(), None, true).unwrap();
        assert_eq!((p.channels(), p.sample_rate()), (2, 44100));
        let expected = [-1.0, 32767.0 / 32768.0, 0.0, 0.5, -0.5, 0.0];
        for _ in 0..3 {
            for s in expected {
                assert_eq!(p.next_sample().unwrap(), Some(s));
            }
        }
        let mut p = p.restarted(false).unwrap();
        for s in expected {
            assert_eq!(p.next_sample().unwrap(), Some(s));
        }
        assert_eq!(p.next_sample().unwrap(), None);
        assert_eq!(p.next_sample().unwrap(), None);
    }
    #[test]
    fn rejects_truncation_and_unsupported_pcm() {
        let b = fixture();
        for n in 0..b.len() {
            assert!(AudioProgram::new(b[..n].to_vec(), None, false).is_err());
        }
        for (offset, value) in [(20, 3), (22, 0), (32, 2), (34, 24), (40, 11)] {
            let mut bad = b.clone();
            bad[offset] = value;
            assert!(AudioProgram::new(bad, None, false).is_err());
        }
    }
    #[test]
    #[ignore = "requires CMVS_TEST_WAV pointing to a local game resource"]
    fn external_wave_decodes_to_end() {
        let bytes = std::fs::read(std::env::var_os("CMVS_TEST_WAV").unwrap()).unwrap();
        let mut p = AudioProgram::new(bytes, None, false).unwrap();
        let mut count = 0usize;
        while let Some(s) = p.next_sample().unwrap() {
            assert!(s.is_finite());
            count += 1;
        }
        assert!(count > 0);
        assert!(count.is_multiple_of(usize::from(p.channels())));
        #[cfg(feature = "mixer")]
        {
            use cmvs_core::audio::AudioBackend;
            let path = std::path::PathBuf::from(std::env::var_os("CMVS_TEST_WAV").unwrap());
            let (mut audio, mut output) =
                crate::NativeAudio::pull(path.parent().unwrap().parent().unwrap(), 48000, 2)
                    .unwrap();
            audio
                .configure_paths(&std::collections::BTreeMap::from([(
                    (0x2012, 0),
                    [
                        path.parent()
                            .unwrap()
                            .file_name()
                            .unwrap()
                            .as_encoded_bytes(),
                        b"\\",
                    ]
                    .concat(),
                )]))
                .unwrap();
            assert!(
                audio
                    .preload(0, path.file_name().unwrap().as_encoded_bytes())
                    .unwrap()
            );
            audio.play_preloaded(0, false, 256).unwrap();
            let mut pcm = vec![0i16; count * 2 + 48000];
            output.fill_i16(&mut pcm);
            assert!(pcm.iter().any(|&s| s != 0));
        }
    }
}

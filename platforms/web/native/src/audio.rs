//! Streaming Vorbis channels backed by the page's Web Audio device.
use cmvs_audio::VorbisProgram;
use cmvs_core::{
    MAX_PAYLOAD,
    audio::{
        AudioBackend, AudioRequest, MovieAudioClock, SE_CHANNELS, SePlayback, VoiceActivity,
        VoiceCompressor, VoiceDucking, VoiceDuckingSettings, VoiceMixer,
    },
    cpz::Cpz7Parameters,
    filesystem::File,
    platform::resolve_under,
    resources::{ResourceArchives, ResourceKind, ResourceSource},
};
use std::{
    collections::BTreeMap,
    error::Error,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use wasm_bindgen::prelude::*;
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = begin)]
    fn begin(id: u32, epoch: u32, rate: u32, channels: u32, gain: f32, fade_ms: u32);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = samples)]
    fn samples(id: u32, epoch: u32, pcm: &[f32], ended: bool);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = stop)]
    fn stop(id: u32);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = gain)]
    fn gain(id: u32, value: f32);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = fade)]
    fn fade(id: u32, ms: u32);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = enabled)]
    fn enabled(value: bool);
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = position)]
    fn position(id: u32, epoch: u32) -> f64;
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsAudio"], js_name = now)]
    fn now() -> f64;
}
struct Compression {
    compressor: VoiceCompressor,
    bytes: u32,
    chunk: Vec<i16>,
    at: usize,
    ended: bool,
}
struct Channel {
    program: VorbisProgram,
    compression: Option<Compression>,
    epoch: u32,
    started: bool,
    drained: bool,
    fading: bool,
    sent: u64,
    volume: f32,
}
impl Channel {
    fn new(program: VorbisProgram, volume: u32) -> Self {
        Self {
            program,
            compression: None,
            epoch: 0,
            started: false,
            drained: false,
            fading: false,
            sent: 0,
            volume: volume.min(256) as f32 / 256.0,
        }
    }
    fn pcm(&mut self) -> Result<Option<i16>, Box<dyn Error>> {
        Ok(self
            .program
            .next_sample()?
            .map(|x| (x * 32768.0).clamp(-32768.0, 32767.0).trunc() as i16))
    }
    fn next(&mut self) -> Result<Option<i16>, Box<dyn Error>> {
        let Some(compression) = &mut self.compression else {
            return self.pcm();
        };
        if compression.at == compression.chunk.len() {
            if compression.ended || compression.compressor.ended() {
                return Ok(None);
            }
            let bytes = compression.bytes;
            let request = compression.compressor.request(bytes) / 2;
            let mut decoded = Vec::with_capacity(request as usize);
            while decoded.len() < request as usize {
                match self.pcm()? {
                    Some(sample) => decoded.push(sample),
                    None => break,
                }
            }
            let compression = self.compression.as_mut().expect("checked");
            compression.chunk = compression.compressor.process(bytes, &decoded)?;
            compression.at = 0;
            compression.ended = compression.compressor.ended();
            if compression.chunk.is_empty() {
                return Ok(None);
            }
        }
        let compression = self.compression.as_mut().expect("checked");
        let sample = compression.chunk[compression.at];
        compression.at += 1;
        Ok(Some(sample))
    }
    fn pump(&mut self, id: usize) -> Result<(), Box<dyn Error>> {
        if !self.started || self.drained {
            return Ok(());
        }
        let rate = self.program.sample_rate();
        let channels = u32::from(self.program.channels());
        let consumed = position(id as u32, self.epoch).max(0.0);
        while self.sent as f64 * 1000.0 / f64::from(rate * channels) - consumed < 250.0
            && !self.drained
        {
            let mut pcm = Vec::with_capacity((rate / 10 * channels) as usize);
            for _ in 0..rate / 10 * channels {
                match self.next()? {
                    Some(sample) => pcm.push(f32::from(sample) / 32768.0),
                    None => {
                        self.drained = true;
                        break;
                    }
                }
            }
            self.sent += pcm.len() as u64;
            samples(id as u32, self.epoch, &pcm, self.drained);
        }
        Ok(())
    }
}
pub struct BrowserAudio {
    root: PathBuf,
    archives: ResourceArchives,
    paths: BTreeMap<(u16, u32), Vec<u8>>,
    slots: [Option<Channel>; 16],
    epoch: u32,
    ducking: VoiceDucking,
    duck_gain: f32,
}
impl BrowserAudio {
    pub fn new(root: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            root: root.into(),
            archives: ResourceArchives::new(root)?,
            paths: BTreeMap::new(),
            slots: std::array::from_fn(|_| None),
            epoch: 0,
            ducking: VoiceDucking::default(),
            duck_gain: 1.0,
        })
    }
    fn load(&mut self, kind: ResourceKind, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        if name.is_empty() {
            return Ok(None);
        }
        if !name.to_ascii_lowercase().ends_with(b".ogg") {
            return Err("browser audio supports Ogg Vorbis resources only".into());
        }
        self.archives.with_paths(&self.paths, kind).read(name)
    }
    fn release(&mut self, id: usize) {
        stop(id as u32);
        self.slots[id] = None;
    }
    fn install(
        &mut self,
        id: usize,
        channel: Option<Channel>,
        start: bool,
        fade_ms: u32,
    ) -> Result<(), Box<dyn Error>> {
        self.release(id);
        self.epoch = self.epoch.wrapping_add(1);
        self.slots[id] = channel.map(|mut c| {
            c.epoch = self.epoch;
            c
        });
        if start {
            self.start_slot(id, fade_ms)?;
        }
        Ok(())
    }
    fn start_slot(&mut self, id: usize, fade_ms: u32) -> Result<(), Box<dyn Error>> {
        if let Some(channel) = &mut self.slots[id]
            && !channel.started
        {
            let volume = channel.volume * if id < 2 { self.duck_gain } else { 1.0 };
            begin(
                id as u32,
                channel.epoch,
                channel.program.sample_rate(),
                u32::from(channel.program.channels()),
                volume,
                fade_ms,
            );
            channel.started = true;
            channel.pump(id)?;
        }
        Ok(())
    }
    fn fade_slot(&mut self, id: usize, ms: u32) {
        if let Some(channel) = &mut self.slots[id]
            && !channel.fading
        {
            channel.fading = true;
            if channel.started {
                fade(id as u32, ms);
            } else {
                self.release(id);
            }
        }
    }
    fn source_position(&self, id: usize) -> Option<u32> {
        let channel = self.slots[id].as_ref().filter(|c| c.started)?;
        let value = position(id as u32, channel.epoch);
        (value >= 0.0).then_some(value.min(f64::from(u32::MAX)) as u32)
    }
    fn se(channel: u8) -> Result<usize, Box<dyn Error>> {
        if channel < 6 {
            Ok(9 + usize::from(channel))
        } else {
            Err("SE channel out of range".into())
        }
    }
    fn voice(channel: u8) -> Result<usize, Box<dyn Error>> {
        if channel < 4 {
            Ok(5 + usize::from(channel))
        } else {
            Err("voice channel out of range".into())
        }
    }
}
impl AudioBackend for BrowserAudio {
    fn set_output_enabled(&mut self, value: bool) -> Result<(), Box<dyn Error>> {
        enabled(value);
        Ok(())
    }
    fn configure_voice_ducking(
        &mut self,
        value: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error>> {
        self.ducking.configure(value);
        Ok(())
    }
    fn configure_paths(
        &mut self,
        value: &BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        self.paths.clone_from(value);
        Ok(())
    }
    fn configure_archive_parameters(
        &mut self,
        value: &BTreeMap<(u16, u32), Cpz7Parameters>,
    ) -> Result<(), Box<dyn Error>> {
        self.archives.configure_parameters(value);
        Ok(())
    }
    fn configure_se(
        &mut self,
        values: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error>> {
        let mut prepared: [Option<Channel>; 6] = std::array::from_fn(|_| None);
        for (id, value) in values.iter().enumerate() {
            if let Some(value) = value
                && let Some(data) = self.load(ResourceKind::SoundEffect, &value.name)?
            {
                prepared[id] = Some(Channel::new(
                    VorbisProgram::new(data, None, value.looped)?,
                    value.volume,
                ));
            }
        }
        for (id, candidate) in prepared.into_iter().enumerate() {
            self.install(
                9 + id,
                candidate,
                values[id].as_ref().is_some_and(|v| v.start),
                0,
            )?;
        }
        Ok(())
    }
    fn set_volumes(&mut self, values: &[Option<u32>; 15]) -> Result<(), Box<dyn Error>> {
        for (id, value) in values.iter().enumerate() {
            if let Some(value) = value
                && *value <= 256
                && let Some(channel) = &mut self.slots[id]
            {
                channel.volume = *value as f32 / 256.0;
                if channel.started {
                    gain(
                        id as u32,
                        channel.volume * if id < 2 { self.duck_gain } else { 1.0 },
                    );
                }
            }
        }
        Ok(())
    }
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        let AudioRequest::PlaySe {
            channel,
            name,
            volume,
            looped,
            flag,
            fade_in_ms,
        } = request
        else {
            return Err("expected PlaySe".into());
        };
        let id = Self::se(*channel)?;
        let Some(data) = self.load(ResourceKind::SoundEffect, name)? else {
            self.release(id);
            return Ok(false);
        };
        let candidate = Channel::new(VorbisProgram::new(data, None, *looped)?, *volume);
        self.install(
            id,
            Some(candidate),
            *flag,
            if *flag { *fade_in_ms } else { 0 },
        )?;
        Ok(true)
    }
    fn start(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error>> {
        let id = Self::se(channel)?;
        if let Some(c) = &self.slots[id]
            && !c.started
        {
            c.program.set_looped(looped);
        }
        self.start_slot(id, 0)
    }
    fn restart_se(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error>> {
        let id = Self::se(channel)?;
        let Some(current) = &self.slots[id] else {
            return Ok(());
        };
        let mut candidate = Channel::new(current.program.restarted(looped)?, 0);
        candidate.volume = current.volume;
        // Decode the initial queue before releasing the old channel, so failures
        // leave its playback and clock intact. No fallible work follows install.
        let count = candidate.program.sample_rate() as usize
            * usize::from(candidate.program.channels())
            / 4;
        let mut pcm = Vec::with_capacity(count);
        for _ in 0..count {
            match candidate.next()? {
                Some(sample) => pcm.push(f32::from(sample) / 32768.0),
                None => {
                    candidate.drained = true;
                    break;
                }
            }
        }
        candidate.sent = pcm.len() as u64;
        self.install(id, Some(candidate), false, 0)?;
        let channel = self.slots[id].as_mut().unwrap();
        begin(
            id as u32,
            channel.epoch,
            channel.program.sample_rate(),
            u32::from(channel.program.channels()),
            channel.volume,
            0,
        );
        channel.started = true;
        samples(id as u32, channel.epoch, &pcm, channel.drained);
        Ok(())
    }
    fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        let id = Self::se(channel)?;
        self.release(id);
        Ok(())
    }
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error>> {
        self.release(15);
        Ok(())
    }
    fn fade_out_se(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        let id = Self::se(channel)?;
        self.fade_slot(id, ms);
        Ok(())
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        let id = Self::voice(channel)?;
        self.release(id);
        Ok(())
    }
    fn fade_out_voice(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        let id = Self::voice(channel)?;
        self.fade_slot(id, ms);
        Ok(())
    }
    fn play_voice(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        let AudioRequest::PlayVoice {
            channel,
            name,
            volume,
            start,
            rate_bits,
            mixer,
        } = request
        else {
            return Err("expected PlayVoice".into());
        };
        let id = Self::voice(*channel)?;
        let data = self.load(ResourceKind::Voice, name)?;
        self.release(id);
        let Some(data) = data else {
            return Ok(false);
        };
        let mut candidate = Channel::new(VorbisProgram::new(data, None, false)?, *volume);
        if let Some(compressor) = VoiceCompressor::new(
            f32::from_bits(*rate_bits),
            candidate.program.sample_rate(),
            candidate.program.channels(),
        )? {
            candidate.compression = Some(Compression {
                compressor,
                bytes: VoiceMixer::mono_request(*mixer, candidate.program.sample_rate())
                    .ok_or("SOUND_BUFFER outside 0..3")?,
                chunk: Vec::new(),
                at: 0,
                ended: false,
            });
        }
        self.install(id, Some(candidate), *start, 0)?;
        Ok(true)
    }
    fn play_bgm(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        let AudioRequest::PlayBgm {
            channel,
            previous,
            primary,
            secondary,
            volume,
            fade_ms,
            looped,
            ..
        } = request
        else {
            return Err("expected PlayBgm".into());
        };
        if *channel > 1 || *previous > 1 {
            return Err("BGM channel out of range".into());
        }
        let first = self
            .load(ResourceKind::Music, primary)?
            .ok_or("BGM primary resource missing")?;
        let second = if secondary.is_empty() {
            None
        } else {
            Some(
                self.load(ResourceKind::Music, secondary)?
                    .ok_or("BGM loop resource missing")?,
            )
        };
        let candidate = Channel::new(VorbisProgram::new(first, second, *looped)?, *volume);
        self.set_volumes(&std::array::from_fn(|i| {
            if i < 2 {
                Some((*volume).min(256))
            } else {
                None
            }
        }))?;
        if *fade_ms == 0 {
            self.release(0);
            self.release(1);
        } else {
            self.fade_slot(usize::from(*previous), *fade_ms);
        }
        self.install(usize::from(*channel), Some(candidate), true, *fade_ms)
    }
    fn fade_out_bgm(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        if channel > 1 {
            return Err("BGM channel out of range".into());
        }
        self.fade_slot(usize::from(channel), ms);
        Ok(())
    }
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error>> {
        self.release(0);
        self.release(1);
        Ok(())
    }
    fn play_movie_audio(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        let AudioRequest::PlayMovieAudio {
            path,
            offset,
            size,
            volume,
        } = request
        else {
            return Err("expected PlayMovieAudio".into());
        };
        if *size as usize > MAX_PAYLOAD {
            return Err("movie Ogg resource exceeds limit".into());
        }
        let mut file = File::open(resolve_under(&self.root, path)?)?;
        if offset
            .checked_add(u64::from(*size))
            .is_none_or(|end| file.metadata().map_or(true, |m| end > m.len()))
        {
            return Err("movie Ogg range outside file".into());
        }
        file.seek(SeekFrom::Start(*offset))?;
        let mut bytes = vec![0; *size as usize];
        file.read_exact(&mut bytes)?;
        let candidate = Channel::new(VorbisProgram::new(bytes, None, false)?, *volume);
        self.install(2, Some(candidate), true, 0)
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error>> {
        self.release(2);
        Ok(())
    }
    fn update(&mut self) -> Result<(), Box<dyn Error>> {
        let voices: [VoiceActivity; 4] = std::array::from_fn(|i| VoiceActivity {
            playing: self.source_position(i + 5).is_some(),
            fading_out: self.slots[i + 5].as_ref().is_some_and(|c| c.fading),
        });
        let duck = self.ducking.update(now().max(0.0) as u32, &voices) as f32 / 256.0;
        if duck != self.duck_gain {
            self.duck_gain = duck;
            for id in 0..2 {
                if let Some(c) = &self.slots[id]
                    && c.started
                {
                    gain(id as u32, c.volume * duck);
                }
            }
        }
        for id in 0..16 {
            if self.slots[id]
                .as_ref()
                .is_some_and(|c| c.started && position(id as u32, c.epoch) < 0.0)
            {
                self.release(id);
            }
            if let Some(channel) = &mut self.slots[id] {
                channel.pump(id)?;
            }
        }
        Ok(())
    }
    fn se_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(self.source_position(Self::se(channel)?))
    }
    fn voice_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(self.source_position(Self::voice(channel)?))
    }
    fn movie_clock(&self) -> Result<Option<MovieAudioClock>, Box<dyn Error>> {
        Ok(Some(
            self.source_position(2)
                .map_or(MovieAudioClock::Ended, |position_ms| {
                    MovieAudioClock::Playing { position_ms }
                }),
        ))
    }
}

impl Drop for BrowserAudio {
    fn drop(&mut self) {
        for id in 0..16 {
            if self.slots[id].is_some() {
                stop(id as u32);
            }
        }
    }
}

//! Optional Rust Vorbis streaming and native output. Engine state stays in `audio`.
use crate::VorbisProgram;
use cmvs_core::{
    MAX_PAYLOAD,
    audio::{
        AudioBackend, AudioRequest, MovieAudioClock, SE_CHANNELS, SePlayback, VOICE_CHANNELS,
        VoiceActivity, VoiceCompressor, VoiceDucking, VoiceDuckingSettings,
    },
    platform::resolve_under,
    resources::{ResourceArchives, ResourceKind, ResourceSource},
};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Fault = Arc<Mutex<Option<String>>>;
/// 0x1400892f0's per-chunk path for a sound with an attached compressor.
struct Compression {
    compressor: VoiceCompressor,
    chunk_bytes: u32,
    chunk: Vec<i16>,
    at: usize,
    /// The compressor ended during this chunk; the mixer frees it afterwards and the
    /// exhausted decoder stops the channel at the next chunk.
    last: bool,
}
struct StreamSource {
    output_enabled: Arc<AtomicBool>,
    host_control: HostAudioControl,
    program: VorbisProgram,
    compression: Option<Compression>,
    fault: Fault,
    failed: bool,
}
impl StreamSource {
    /// Original MOG output is saturated signed 16-bit PCM before device gain.
    fn pcm(&mut self) -> Result<Option<i16>, Box<dyn Error>> {
        Ok(self
            .program
            .next_sample()?
            .map(|x| (x * 32768.0).clamp(-32768.0, 32767.0).trunc() as i16))
    }
    fn compressed(&mut self) -> Result<Option<i16>, Box<dyn Error>> {
        let compression = self.compression.as_mut().expect("checked by caller");
        if compression.at == compression.chunk.len() {
            if compression.last || compression.compressor.ended() {
                return Ok(None);
            }
            let need = compression.chunk_bytes;
            let request = compression.compressor.request(need) / 2;
            let mut decoded = Vec::with_capacity(request as usize);
            while decoded.len() < request as usize {
                match self.pcm()? {
                    Some(sample) => decoded.push(sample),
                    None => break,
                }
            }
            let compression = self.compression.as_mut().expect("checked by caller");
            compression.chunk = compression.compressor.process(need, &decoded)?;
            compression.at = 0;
            compression.last = compression.compressor.ended();
            if compression.chunk.is_empty() {
                return Ok(None);
            }
        }
        let compression = self.compression.as_mut().expect("checked by caller");
        compression.at += 1;
        Ok(Some(compression.chunk[compression.at - 1]))
    }
}
impl Iterator for StreamSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.failed {
            return None;
        }
        let sample = if self.compression.is_some() {
            self.compressed()
        } else {
            self.pcm()
        };
        match sample {
            Ok(sample) => sample.map(|x| {
                if self.output_enabled.load(Ordering::Relaxed)
                    && self.host_control.active.load(Ordering::Relaxed)
                {
                    f32::from(x) / 32768.0
                } else {
                    0.0
                }
            }),
            Err(e) => {
                *self.fault.lock().unwrap() = Some(e.to_string());
                self.failed = true;
                None
            }
        }
    }
}
impl Source for StreamSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(self.program.channels())
            .expect("Vorbis channel count is validated non-zero")
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(self.program.sample_rate())
            .expect("Vorbis sample rate is validated non-zero")
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Envelope {
    from: f32,
    to: f32,
    ms: u32,
    started: Instant,
}
impl Envelope {
    pub fn new(from: f32, to: f32, ms: u32) -> Self {
        Self {
            from,
            to,
            ms,
            started: Instant::now(),
        }
    }
    pub fn value_at(&self, elapsed_ms: u32) -> f32 {
        if self.ms == 0 {
            return self.to;
        }
        self.from + (self.to - self.from) * (elapsed_ms.min(self.ms) as f32 / self.ms as f32)
    }
    fn value(&self) -> f32 {
        self.value_at(self.started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
    }
    fn finished(&self) -> bool {
        self.started.elapsed().as_millis() >= u128::from(self.ms)
    }
}
struct Channel {
    bytes: Vec<Arc<[u8]>>,
    sink: Player,
    fault: Fault,
    envelope: Option<Envelope>,
    release_at_end: bool,
    looped: Arc<AtomicBool>,
    gain: f32,
    duck_gain: f32,
}
impl Channel {
    fn check(&self) -> Result<(), Box<dyn Error>> {
        if let Some(error) = self.fault.lock().unwrap().as_ref() {
            return Err(format!("Vorbis playback failed: {error}").into());
        }
        Ok(())
    }
}

/// Host lifecycle gate, independent of the game's own sound settings.
/// Muting keeps consuming audio, matching the existing output gate policy.
#[derive(Clone)]
pub struct HostAudioControl {
    active: Arc<AtomicBool>,
}
impl HostAudioControl {
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }
}

/// 2007 0x450030: DirectSound hundredth-decibel conversion; invalid positive gains
/// leave the existing device volume untouched.
fn preloaded_gain(volume: u32) -> Option<f32> {
    let db = if volume == 0 {
        -10000
    } else {
        let fraction = (f64::from(volume) / 256.0) as f32;
        let logarithm = f64::from(fraction).log10() as f32;
        (f64::from(logarithm) * 4000.0) as i32
    };
    (-10000..=0)
        .contains(&db)
        .then(|| 10f32.powf(db as f32 / 2000.0))
}

/// Native output shared by desktop and Android hosts.
pub struct NativeAudio {
    preloaded: [Option<Channel>; cmvs_core::audio::PRELOADED_SOUNDS],
    output_enabled: Arc<AtomicBool>,
    host_control: HostAudioControl,
    root: PathBuf,
    paths: BTreeMap<(u16, u32), Vec<u8>>,
    archives: ResourceArchives,
    se: [Option<Channel>; SE_CHANNELS],
    voice: [Option<Channel>; VOICE_CHANNELS],
    bgm: [Option<Channel>; 2],
    movie: Option<Channel>,
    // 0x2241 addresses this device slot independently of the six SE channels.
    // Its playback command is not implemented yet, so it starts and remains empty.
    channel15: Option<Channel>,
    device_fault: Fault,
    last_movie_report_ms: u32,
    voice_ducking: VoiceDucking,
    ducking_clock: Instant,
    // Channels must drop before the device stream.
    stream: MixerDeviceSink,
}
impl NativeAudio {
    pub fn new(root: &Path) -> Result<Self, Box<dyn Error>> {
        let fault: Fault = Arc::default();
        let copy = fault.clone();
        let mut stream = DeviceSinkBuilder::from_default_device()?
            .with_error_callback(move |e| {
                *copy.lock().unwrap() = Some(e.to_string());
            })
            .open_sink_or_fallback()?;
        stream.log_on_drop(false);
        println!(
            "# native audio output: {} Hz, {} channels",
            stream.config().sample_rate(),
            stream.config().channel_count()
        );
        Ok(Self {
            preloaded: std::array::from_fn(|_| None),
            output_enabled: Arc::new(AtomicBool::new(true)),
            host_control: HostAudioControl {
                active: Arc::new(AtomicBool::new(true)),
            },
            root: root.canonicalize()?,
            paths: BTreeMap::new(),
            archives: ResourceArchives::new(root)?,
            se: std::array::from_fn(|_| None),
            voice: std::array::from_fn(|_| None),
            bgm: std::array::from_fn(|_| None),
            movie: None,
            channel15: None,
            device_fault: fault,
            last_movie_report_ms: 0,
            voice_ducking: VoiceDucking::default(),
            ducking_clock: Instant::now(),
            stream,
        })
    }
    pub fn host_control(&self) -> HostAudioControl {
        self.host_control.clone()
    }
    /// Observe BGM device channel 0 or 1 without changing playback or advancing time.
    /// Returns source consumption milliseconds, excluding device-buffer latency;
    /// an empty, paused or exhausted channel returns None. Call `update` separately
    /// to advance fades and report device failures. This is not a VM command result.
    pub fn bgm_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        let channel = self
            .bgm
            .get(usize::from(channel))
            .ok_or("BGM channel out of range")?;
        Self::position(channel.as_ref())
    }
    /// Current sink gain control, including fades and voice ducking. This does not
    /// measure device output amplitude or queued hardware buffers.
    pub fn bgm_gain(&self, channel: u8) -> Result<Option<f32>, Box<dyn Error>> {
        Ok(self
            .bgm
            .get(usize::from(channel))
            .ok_or("BGM channel out of range")?
            .as_ref()
            .map(|c| c.sink.volume()))
    }
    fn load(&mut self, kind: ResourceKind, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        if name.is_empty() {
            return Ok(None);
        }
        if !name.to_ascii_lowercase().ends_with(b".ogg") {
            return Err("native audio supports verified Ogg Vorbis resources only".into());
        }
        self.archives.with_paths(&self.paths, kind).read(name)
    }
    fn prepare(&self, program: VorbisProgram, volume: u32, fade_ms: u32) -> Channel {
        self.prepare_with(program, None, volume, fade_ms)
    }
    fn prepare_with(
        &self,
        program: VorbisProgram,
        compression: Option<Compression>,
        volume: u32,
        fade_ms: u32,
    ) -> Channel {
        let fault: Fault = Arc::default();
        let looped = program.looped.clone();
        let sink = Player::connect_new(self.stream.mixer());
        sink.pause();
        let gain = volume.min(256) as f32 / 256.0;
        let envelope = (fade_ms != 0).then(|| Envelope::new(0.0, 1.0, fade_ms));
        sink.set_volume(if envelope.is_some() { 0.0 } else { gain });
        let bytes = program.bytes.clone();
        sink.append(StreamSource {
            output_enabled: self.output_enabled.clone(),
            host_control: self.host_control.clone(),
            program,
            compression,
            fault: fault.clone(),
            failed: false,
        });
        Channel {
            bytes,
            sink,
            fault,
            envelope,
            release_at_end: false,
            looped,
            gain,
            duck_gain: 1.0,
        }
    }
    fn fade(channel: &mut Option<Channel>, ms: u32) {
        if let Some(channel) = channel
            && !channel.release_at_end
        {
            let from = channel.envelope.as_ref().map_or(1.0, Envelope::value);
            channel.envelope = Some(Envelope::new(from, 0.0, ms));
            channel.release_at_end = true;
        }
    }
    fn set_gain(channel: &mut Channel, gain: f32) {
        channel.sink.set_volume(
            gain * channel.duck_gain * channel.envelope.as_ref().map_or(1.0, Envelope::value),
        );
        channel.gain = gain;
    }
    /// Playing means started (not paused) with source data left; `update` releases
    /// finished channels. The position is the source consumption clock, not a DAC
    /// timestamp, and includes no device-buffer latency.
    fn position(channel: Option<&Channel>) -> Result<Option<u32>, Box<dyn Error>> {
        let Some(channel) = channel else {
            return Ok(None);
        };
        channel.check()?;
        if channel.sink.is_paused() || channel.sink.empty() {
            return Ok(None);
        }
        Ok(Some(
            channel.sink.get_pos().as_millis().min(u128::from(u32::MAX)) as u32,
        ))
    }
    fn tick_channel(channel: &mut Option<Channel>) -> Result<(), Box<dyn Error>> {
        let Some(current) = channel else {
            return Ok(());
        };
        current.check()?;
        if let Some(fade) = current.envelope {
            current
                .sink
                .set_volume(current.gain * current.duck_gain * fade.value());
            if fade.finished() {
                if current.release_at_end {
                    *channel = None;
                } else {
                    current.envelope = None;
                }
            }
        }
        Ok(())
    }
}
impl AudioBackend for NativeAudio {
    fn preload(&mut self, slot: u8, name: &[u8]) -> Result<bool, Box<dyn Error>> {
        let index = usize::from(slot);
        if index >= self.preloaded.len() {
            return Err("preloaded sound slot out of range".into());
        }
        let sound = self
            .load(ResourceKind::SoundEffect, name)?
            .map(|data| VorbisProgram::new(data, None, false))
            .transpose()?;
        let loaded = sound.is_some();
        self.preloaded[index] = sound.map(|program| self.prepare(program, 256, 0));
        Ok(loaded)
    }
    fn release_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error>> {
        *self
            .preloaded
            .get_mut(usize::from(slot))
            .ok_or("preloaded sound slot out of range")? = None;
        Ok(())
    }

    fn play_preloaded(
        &mut self,
        slot: u8,
        looped: bool,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        let index = usize::from(slot);
        let current = self
            .preloaded
            .get(index)
            .ok_or("preloaded sound slot out of range")?;
        if let Some(current) = current {
            current.check()?;
            if current.sink.empty() {
                let program = VorbisProgram::from_shared(current.bytes.clone(), looped)?;
                let mut candidate = self.prepare(program, 256, 0);
                candidate.gain = current.gain;
                candidate.sink.set_volume(current.gain);
                self.preloaded[index] = Some(candidate);
            }
            let current = self.preloaded[index].as_mut().unwrap();
            current.looped.store(looped, Ordering::Relaxed);
            if let Some(gain) = preloaded_gain(volume) {
                current.gain = gain;
                current.sink.set_volume(gain);
            }
            current.sink.play();
        }
        Ok(())
    }
    fn stop_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error>> {
        if let Some(current) = self
            .preloaded
            .get(usize::from(slot))
            .ok_or("preloaded sound slot out of range")?
        {
            current.sink.pause();
        }
        Ok(())
    }
    fn set_preloaded_volume(&mut self, slot: u8, volume: u32) -> Result<(), Box<dyn Error>> {
        if let Some(current) = self
            .preloaded
            .get_mut(usize::from(slot))
            .ok_or("preloaded sound slot out of range")?
            && let Some(gain) = preloaded_gain(volume)
        {
            current.gain = gain;
            current.sink.set_volume(gain);
        }
        Ok(())
    }
    fn set_output_enabled(&mut self, enabled: bool) -> Result<(), Box<dyn Error>> {
        self.output_enabled.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    fn configure_voice_ducking(
        &mut self,
        settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error>> {
        self.voice_ducking.configure(settings);
        Ok(())
    }
    fn configure_se(
        &mut self,
        channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error>> {
        let mut prepared = std::array::from_fn(|_| None);
        // Load and validate every resource while old sounds keep playing. Candidates
        // remain paused, so a later decode error leaves the audible state unchanged.
        for (index, request) in channels.iter().enumerate() {
            if let Some(request) = request
                && let Some(data) = self.load(ResourceKind::SoundEffect, &request.name)?
            {
                let program = VorbisProgram::new(data, None, request.looped)?;
                prepared[index] = Some(self.prepare(program, request.volume, 0));
            }
        }
        self.se = prepared;
        for (channel, request) in self.se.iter().zip(channels) {
            if let (Some(channel), Some(request)) = (channel, request)
                && request.start
            {
                channel.sink.play();
            }
        }
        Ok(())
    }
    fn set_volumes(&mut self, channels: &[Option<u32>; 15]) -> Result<(), Box<dyn Error>> {
        // No fallible operation after the first mutation. Keep fade envelopes and
        // playback positions intact, including paused/prepared channels.
        for (index, volume) in channels.iter().enumerate() {
            let Some(volume) = volume else {
                continue;
            };
            // 0x140087ba0 ignores a base gain above 256, preserving the old one.
            if *volume > 256 {
                continue;
            }
            let channel = match index {
                0..=1 => &mut self.bgm[index],
                2 => &mut self.movie,
                3..=4 => continue, // other movie/device slots have no player yet
                5..=8 => &mut self.voice[index - 5],
                _ => &mut self.se[index - 9],
            };
            if let Some(channel) = channel {
                Self::set_gain(channel, *volume as f32 / 256.0);
            }
        }
        Ok(())
    }
    fn configure_paths(
        &mut self,
        paths: &BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        if self.paths != *paths {
            self.paths = paths.clone();
        }
        Ok(())
    }
    fn configure_archive_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), cmvs_core::cpz::Cpz7Parameters>,
    ) -> Result<(), Box<dyn Error>> {
        self.archives.configure_parameters(parameters);
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
        if usize::from(*channel) >= SE_CHANNELS {
            return Err("SE channel out of range".into());
        }
        let data = self.load(ResourceKind::SoundEffect, name)?;
        let Some(data) = data else {
            self.se[*channel as usize] = None;
            return Ok(false);
        };
        let program = VorbisProgram::new(data, None, *looped)?;
        println!(
            "# native SE[{channel}] {}: {} Hz, {} channels, prepared={}, loop={looped}",
            name.escape_ascii(),
            program.sample_rate(),
            program.channels(),
            !flag
        );
        // The original only sets the fade-in (0x140088240) when starting immediately.
        let candidate = self.prepare(program, *volume, if *flag { *fade_in_ms } else { 0 });
        self.se[*channel as usize] = Some(candidate);
        if *flag {
            self.se[*channel as usize].as_ref().unwrap().sink.play();
        }
        Ok(true)
    }
    fn start(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error>> {
        let channel = self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")?;
        if let Some(channel) = channel
            && channel.sink.is_paused()
        {
            channel.looped.store(looped, Ordering::Relaxed);
            channel.sink.play();
        }
        Ok(())
    }
    fn restart_se(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error>> {
        let slot = usize::from(channel);
        let Some(current) = self.se.get(slot).ok_or("SE channel out of range")? else {
            return Ok(());
        };
        let program = VorbisProgram::from_shared(current.bytes.clone(), looped)?;
        let candidate = self.prepare(program, (current.gain * 256.0) as u32, 0);
        // All fallible preparation precedes replacing the old device channel.
        self.se[slot] = Some(candidate);
        self.se[slot].as_ref().unwrap().sink.play();
        Ok(())
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        *self
            .voice
            .get_mut(usize::from(channel))
            .ok_or("voice channel out of range")? = None;
        Ok(())
    }
    fn fade_out_voice(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        let channel = self
            .voice
            .get_mut(usize::from(channel))
            .ok_or("voice channel out of range")?;
        Self::fade(channel, ms);
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
        let slot = usize::from(*channel);
        if slot >= VOICE_CHANNELS {
            return Err("voice channel out of range".into());
        }
        let data = self.load(ResourceKind::Voice, name)?;
        // The device releases the channel before loading, also when loading fails.
        self.voice[slot] = None;
        let Some(data) = data else {
            return Ok(false);
        };
        let program = VorbisProgram::new(data, None, false)?;
        let rate = f32::from_bits(*rate_bits);
        let compression =
            match VoiceCompressor::new(rate, program.sample_rate(), program.channels())? {
                Some(compressor) => Some(Compression {
                    compressor,
                    chunk_bytes: mixer
                        .mono_request(program.sample_rate())
                        .ok_or("SOUND_BUFFER outside 0..3 leaves the mixer chunk unset")?,
                    chunk: Vec::new(),
                    at: 0,
                    last: false,
                }),
                None => None,
            };
        println!(
            "# native voice[{channel}] {}: {} Hz, {} channels, rate={rate}, compressed={}, started={start}",
            name.escape_ascii(),
            program.sample_rate(),
            program.channels(),
            compression.is_some()
        );
        let candidate = self.prepare_with(program, compression, *volume, 0);
        if *start {
            candidate.sink.play();
        }
        self.voice[slot] = Some(candidate);
        Ok(true)
    }
    fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        *self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")? = None;
        Ok(())
    }
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error>> {
        self.channel15 = None;
        Ok(())
    }
    fn fade_out_se(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        let channel = self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")?;
        Self::fade(channel, ms);
        Ok(())
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
        let program = VorbisProgram::new(first, second, *looped)?;
        println!(
            "# native BGM[{channel}] {} + {}: {} Hz, {} channels, loop={looped}, fade_ms={fade_ms}",
            primary.escape_ascii(),
            secondary.escape_ascii(),
            program.sample_rate(),
            program.channels()
        );
        let mut candidate = self.prepare(program, *volume, *fade_ms);
        candidate.duck_gain = self.voice_ducking.gain() as f32 / 256.0;
        Self::set_gain(&mut candidate, (*volume).min(256) as f32 / 256.0);
        for current in self.bgm.iter_mut().flatten() {
            Self::set_gain(current, (*volume).min(256) as f32 / 256.0);
        }
        if *fade_ms == 0 {
            self.bgm = [None, None];
        } else {
            Self::fade(&mut self.bgm[*previous as usize], *fade_ms);
        }
        self.bgm[*channel as usize] = Some(candidate);
        self.bgm[*channel as usize].as_ref().unwrap().sink.play();
        Ok(())
    }
    fn fade_out_bgm(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error>> {
        let channel = self
            .bgm
            .get_mut(usize::from(channel))
            .ok_or("BGM channel out of range")?;
        Self::fade(channel, ms);
        Ok(())
    }
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error>> {
        self.bgm = [None, None];
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
        let candidate = self.prepare(VorbisProgram::new(bytes, None, false)?, *volume, 0);
        self.movie = Some(candidate);
        self.last_movie_report_ms = 0;
        println!(
            "# native movie audio: {} offset={offset} size={size}",
            path.escape_ascii()
        );
        self.movie.as_ref().unwrap().sink.play();
        Ok(())
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error>> {
        self.movie = None;
        Ok(())
    }
    fn update(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(e) = self.device_fault.lock().unwrap().as_ref() {
            return Err(format!("audio device failed: {e}").into());
        }
        let voices = self.voice.each_ref().map(|c| VoiceActivity {
            playing: c
                .as_ref()
                .is_some_and(|c| !c.sink.is_paused() && !c.sink.empty()),
            fading_out: c.as_ref().is_some_and(|c| c.release_at_end),
        });
        let duck = self
            .voice_ducking
            .update(self.ducking_clock.elapsed().as_millis() as u32, &voices)
            as f32
            / 256.0;
        for channel in self.bgm.iter_mut().flatten() {
            channel.duck_gain = duck;
            Self::set_gain(channel, channel.gain);
        }
        for channel in self.se.iter_mut().chain(&mut self.voice) {
            Self::tick_channel(channel)?;
            if let Some(current) = channel
                && current.sink.empty()
            {
                *channel = None;
            }
        }
        for channel in self.preloaded.iter().flatten() {
            channel.check()?;
        }
        for channel in &mut self.bgm {
            Self::tick_channel(channel)?;
        }
        if let Some(movie) = &self.movie {
            movie.check()?;
            let position = movie.sink.get_pos().as_millis().min(u128::from(u32::MAX)) as u32;
            if self.last_movie_report_ms != u32::MAX && movie.sink.empty() {
                println!("# native movie audio ended");
                self.last_movie_report_ms = u32::MAX;
            } else if self.last_movie_report_ms != u32::MAX
                && position.saturating_sub(self.last_movie_report_ms) >= 10000
            {
                println!("# native movie audio source position_ms={position}");
                self.last_movie_report_ms = position;
            }
        }
        Ok(())
    }
    fn se_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        let channel = self
            .se
            .get(usize::from(channel))
            .ok_or("SE channel out of range")?;
        Self::position(channel.as_ref())
    }
    fn voice_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        let channel = self
            .voice
            .get(usize::from(channel))
            .ok_or("voice channel out of range")?;
        Self::position(channel.as_ref())
    }
    fn movie_clock(&self) -> Result<Option<MovieAudioClock>, Box<dyn Error>> {
        let Some(movie) = &self.movie else {
            return Ok(Some(MovieAudioClock::Ended));
        };
        movie.check()?;
        if movie.sink.empty() {
            Ok(Some(MovieAudioClock::Ended))
        } else {
            Ok(Some(MovieAudioClock::Playing {
                position_ms: movie.sink.get_pos().as_millis().min(u128::from(u32::MAX)) as u32,
            }))
        }
    }
}

#[cfg(test)]
mod output_gate_tests {
    use super::*;

    #[test]
    fn preloaded_volume_uses_directsound_logarithmic_gain_and_rejects_positive_db() {
        assert_eq!(preloaded_gain(256), Some(1.0));
        let half = preloaded_gain(128).unwrap();
        assert!((half - 0.25).abs() < 0.001);
        assert_eq!(preloaded_gain(0), Some(0.00001));
        assert_eq!(preloaded_gain(512), None);
    }

    #[test]
    fn host_and_game_mutes_are_independent_and_preserve_stream_consumption() {
        let bytes = include_bytes!("../../../tests/fixtures/audio/mono440.ogg");
        let gate = Arc::new(AtomicBool::new(true));
        let host = HostAudioControl {
            active: Arc::new(AtomicBool::new(true)),
        };
        let mut source = StreamSource {
            output_enabled: gate.clone(),
            host_control: host.clone(),
            program: VorbisProgram::new(bytes.to_vec(), None, false).unwrap(),
            compression: None,
            fault: Arc::default(),
            failed: false,
        };
        let mut reference = VorbisProgram::new(bytes.to_vec(), None, false).unwrap();
        let mut samples = vec![];
        while let Some(pcm) = reference.next_sample().unwrap() {
            samples.push(pcm);
        }
        assert!(samples.len() > 8);
        let length = samples.len();
        for (index, pcm) in samples.into_iter().enumerate() {
            // Exercise all four gate combinations and then resume both gates.
            let phase = index * 8 / length;
            let game_enabled = phase & 1 == 0;
            let host_enabled = phase & 2 == 0;
            gate.store(game_enabled, Ordering::Relaxed);
            host.set_active(host_enabled);
            let enabled = game_enabled && host_enabled;
            let expected = if enabled {
                f32::from((pcm * 32768.0).clamp(-32768.0, 32767.0) as i16) / 32768.0
            } else {
                0.0
            };
            assert_eq!(source.next().unwrap(), expected);
        }
    }
}

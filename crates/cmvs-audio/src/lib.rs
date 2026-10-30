//! Optional Rust Vorbis streaming and native output. Engine state stays in `audio`.
pub mod vorbis;
use crate::vorbis::Vorbis;
use cmvs_core::{
    MAX_PAYLOAD,
    audio::{
        AudioBackend, AudioRequest, MovieAudioClock, SE_CHANNELS, SePlayback, VOICE_CHANNELS,
        VoiceActivity, VoiceCompressor, VoiceDucking, VoiceDuckingSettings,
    },
    platform::resolve_under,
    resources::{ResourceArchives, ResourceKind, ResourceSource},
};
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
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

/// Holds compressed bytes and one decoded packet, never an entire PCM recording.
pub struct VorbisProgram {
    bytes: Vec<Arc<[u8]>>,
    reader: Vorbis,
    looped: Arc<AtomicBool>,
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
            Ok(sample) => sample.map(|x| f32::from(x) / 32768.0),
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
    fn channels(&self) -> u16 {
        self.program.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.program.sample_rate()
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
    sink: Sink,
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

/// Opened only with the desktop's explicit `--audio native` option.
pub struct NativeAudio {
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
    stream: OutputStream,
}
impl NativeAudio {
    pub fn new(root: &Path) -> Result<Self, Box<dyn Error>> {
        let fault: Fault = Arc::default();
        let copy = fault.clone();
        let mut stream = OutputStreamBuilder::from_default_device()?
            .with_error_callback(move |e| {
                *copy.lock().unwrap() = Some(e.to_string());
            })
            .open_stream_or_fallback()?;
        stream.log_on_drop(false);
        println!(
            "# native audio output: {} Hz, {} channels",
            stream.config().sample_rate(),
            stream.config().channel_count()
        );
        Ok(Self {
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
        let sink = Sink::connect_new(self.stream.mixer());
        sink.pause();
        let gain = volume.min(256) as f32 / 256.0;
        let envelope = (fade_ms != 0).then(|| Envelope::new(0.0, 1.0, fade_ms));
        sink.set_volume(if envelope.is_some() { 0.0 } else { gain });
        sink.append(StreamSource {
            program,
            compression,
            fault: fault.clone(),
            failed: false,
        });
        Channel {
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

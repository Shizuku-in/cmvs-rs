//! Optional Rust Vorbis streaming and native output. Engine state stays in `audio`.
pub mod vorbis;
use crate::vorbis::Vorbis;
use cmvs_core::{
    MAX_PAYLOAD,
    audio::{AudioBackend, AudioRequest, MovieAudioClock, SE_CHANNELS, VOICE_CHANNELS},
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
struct StreamSource {
    program: VorbisProgram,
    fault: Fault,
    failed: bool,
}
impl Iterator for StreamSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.failed {
            return None;
        }
        match self.program.next_sample() {
            // Original MOG output is saturated signed 16-bit PCM before device gain.
            Ok(sample) => sample.map(|x| (x * 32768.0).clamp(-32768.0, 32767.0).trunc() / 32768.0),
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
    device_fault: Fault,
    last_movie_report_ms: u32,
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
            device_fault: fault,
            last_movie_report_ms: 0,
            stream,
        })
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
        let fault: Fault = Arc::default();
        let looped = program.looped.clone();
        let sink = Sink::connect_new(self.stream.mixer());
        sink.pause();
        let gain = volume.min(256) as f32 / 256.0;
        let envelope = (fade_ms != 0).then(|| Envelope::new(0.0, 1.0, fade_ms));
        sink.set_volume(if envelope.is_some() { 0.0 } else { gain });
        sink.append(StreamSource {
            program,
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
        channel
            .sink
            .set_volume(gain * channel.envelope.as_ref().map_or(1.0, Envelope::value));
        channel.gain = gain;
    }
    fn tick_channel(channel: &mut Option<Channel>) -> Result<(), Box<dyn Error>> {
        let Some(current) = channel else {
            return Ok(());
        };
        current.check()?;
        if let Some(fade) = current.envelope {
            current.sink.set_volume(current.gain * fade.value());
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
    fn play_voice(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        let AudioRequest::PlayVoice {
            channel,
            name,
            volume,
            start,
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
        println!(
            "# native voice[{channel}] {}: {} Hz, {} channels, started={start}",
            name.escape_ascii(),
            program.sample_rate(),
            program.channels()
        );
        let candidate = self.prepare(program, *volume, 0);
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
        let candidate = self.prepare(program, *volume, *fade_ms);
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

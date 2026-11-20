//! Optional Rust Vorbis streaming, mixing and native output. Engine state stays in `audio`.
//! With only the `mixer` feature the host pulls mixed PCM through [`PcmOutput`].
use crate::AudioProgram;
use cmvs_core::{
    MAX_PAYLOAD, host::audio::AUX_SE_CHANNELS, host::audio::AudioBackend,
    host::audio::AudioRequest, host::audio::CVNS_SE_CHANNELS, host::audio::MovieAudioClock,
    host::audio::SE_CHANNELS, host::audio::SePlayback, host::audio::VOICE_CHANNELS,
    host::audio::VoiceActivity, host::audio::VoiceCompressor, host::audio::VoiceDucking,
    host::audio::VoiceDuckingSettings, host::filesystem::resolve_under,
    host::resources::ResourceArchives, host::resources::ResourceKind,
    host::resources::ResourceSource,
};
#[cfg(feature = "native")]
use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use rodio::{
    Player, Source,
    mixer::{Mixer, MixerSource},
    source::UniformSourceIterator,
};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
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
    /// Device-wide /256 software mixer gain shared by every channel.
    mixer_gain: Arc<AtomicU32>,
    host_control: HostAudioControl,
    program: AudioProgram,
    compression: Option<Compression>,
    fault: Fault,
    failed: bool,
}
impl StreamSource {
    /// Original MOG output is saturated signed 16-bit PCM before device gain.
    fn pcm(&mut self) -> Result<Option<i16>, Box<dyn Error + Send + Sync>> {
        Ok(self
            .program
            .next_sample()?
            .map(|x| (x * 32768.0).clamp(-32768.0, 32767.0).trunc() as i16))
    }
    fn compressed(&mut self) -> Result<Option<i16>, Box<dyn Error + Send + Sync>> {
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
                    let gain = self.mixer_gain.load(Ordering::Relaxed);
                    f32::from(x) / 32768.0 * gain as f32 / 256.0
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

/// Apply channel balance to stereo frames, including duplicated mono input.
struct StereoBalance {
    source: UniformSourceIterator<StreamSource>,
    pan: Arc<AtomicI32>,
    right: bool,
}
impl Iterator for StereoBalance {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.source.next()?;
        let pan = self.pan.load(Ordering::Relaxed);
        let attenuation = if self.right { -pan } else { pan }.max(0);
        self.right = !self.right;
        Some(sample * ((100 - attenuation) as f32 / 100.0))
    }
}
impl Source for StereoBalance {
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.source.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.source.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
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
    script_gain: f32,
    pan: Arc<AtomicI32>,
}
impl Channel {
    fn check(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
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

/// Engine audio channels shared by desktop, Android and pull-output hosts.
pub struct NativeAudio {
    preloaded: [Option<Channel>; cmvs_core::host::audio::PRELOADED_SOUNDS],
    output_enabled: Arc<AtomicBool>,
    mixer_gain: Arc<AtomicU32>,
    host_control: HostAudioControl,
    root: PathBuf,
    paths: BTreeMap<(u16, u32), Vec<u8>>,
    archives: ResourceArchives,
    se: Vec<Option<Channel>>,
    aux_se: [Option<Channel>; AUX_SE_CHANNELS],
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
    mixer: Mixer,
    // Channels must drop before the device stream.
    #[cfg(feature = "native")]
    _device: Option<MixerDeviceSink>,
}

/// Host side of a pull-mode mixer: interleaved samples at the requested rate and
/// channel count. Pull it from one thread; the engine backend may live elsewhere.
pub struct PcmOutput(MixerSource);
impl PcmOutput {
    /// Fills `out` completely. Gaps without any active channel are silence, so a
    /// device that keeps running never underflows because the game is quiet.
    pub fn fill_i16(&mut self, out: &mut [i16]) {
        for sample in out {
            *sample = self
                .0
                .next()
                .map_or(0, |x| (x * 32768.0).clamp(-32768.0, 32767.0) as i16);
        }
    }
}
impl NativeAudio {
    /// Plays through the default system output device.
    #[cfg(feature = "native")]
    pub fn new(root: &Path) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let fault: Fault = Arc::default();
        let copy = fault.clone();
        let mut stream = DeviceSinkBuilder::from_default_device()?
            .with_error_callback(move |e| {
                *copy.lock().unwrap() = Some(e.to_string());
            })
            .open_sink_or_fallback()?;
        stream.log_on_drop(false);
        let mut audio = Self::with_mixer(root, stream.mixer().clone(), fault)?;
        audio._device = Some(stream);
        Ok(audio)
    }
    /// Mixes into a host-pulled stream instead of a system device, for platforms the
    /// device layer does not support. `rate` and `channels` describe the host output.
    pub fn pull(
        root: &Path,
        rate: u32,
        channels: u16,
    ) -> Result<(Self, PcmOutput), Box<dyn Error + Send + Sync>> {
        let (mixer, source) = rodio::mixer::mixer(
            rodio::ChannelCount::new(channels).ok_or("output channel count is zero")?,
            rodio::SampleRate::new(rate).ok_or("output sample rate is zero")?,
        );
        Ok((
            Self::with_mixer(root, mixer, Arc::default())?,
            PcmOutput(source),
        ))
    }
    fn with_mixer(
        root: &Path,
        mixer: Mixer,
        fault: Fault,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self {
            preloaded: std::array::from_fn(|_| None),
            output_enabled: Arc::new(AtomicBool::new(true)),
            mixer_gain: Arc::new(AtomicU32::new(256)),
            host_control: HostAudioControl {
                active: Arc::new(AtomicBool::new(true)),
            },
            root: root.canonicalize()?,
            paths: BTreeMap::new(),
            archives: ResourceArchives::new(root)?,
            se: (0..CVNS_SE_CHANNELS).map(|_| None).collect(),
            aux_se: std::array::from_fn(|_| None),
            voice: std::array::from_fn(|_| None),
            bgm: std::array::from_fn(|_| None),
            movie: None,
            channel15: None,
            device_fault: fault,
            last_movie_report_ms: 0,
            voice_ducking: VoiceDucking::default(),
            ducking_clock: Instant::now(),
            mixer,
            #[cfg(feature = "native")]
            _device: None,
        })
    }
    pub fn host_control(&self) -> HostAudioControl {
        self.host_control.clone()
    }
    /// Observe BGM device channel 0 or 1 without changing playback or advancing time.
    /// Returns source consumption milliseconds, excluding device-buffer latency;
    /// an empty, paused or exhausted channel returns None. Call `update` separately
    /// to advance fades and report device failures. This is not a VM command result.
    pub fn bgm_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        let channel = self
            .bgm
            .get(usize::from(channel))
            .ok_or("BGM channel out of range")?;
        Self::position(channel.as_ref())
    }
    /// Current sink gain control, including fades and voice ducking. This does not
    /// measure device output amplitude or queued hardware buffers.
    pub fn bgm_gain(&self, channel: u8) -> Result<Option<f32>, Box<dyn Error + Send + Sync>> {
        Ok(self
            .bgm
            .get(usize::from(channel))
            .ok_or("BGM channel out of range")?
            .as_ref()
            .map(|c| c.sink.volume()))
    }
    fn load(
        &mut self,
        kind: ResourceKind,
        name: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
        if name.is_empty() {
            return Ok(None);
        }
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(b".ogg") && !lower.ends_with(b".wav") && !lower.ends_with(b".mv2") {
            return Err(format!(
                "unsupported audio resource: {} (expected Ogg Vorbis, PCM WAVE, or MV2)",
                String::from_utf8_lossy(name)
            )
            .into());
        }
        self.archives.with_paths(&self.paths, kind).read(name)
    }
    fn prepare(&self, program: AudioProgram, volume: u32, fade_ms: u32) -> Channel {
        self.prepare_with(program, None, volume, fade_ms)
    }
    fn prepare_with(
        &self,
        program: AudioProgram,
        compression: Option<Compression>,
        volume: u32,
        fade_ms: u32,
    ) -> Channel {
        let fault: Fault = Arc::default();
        let looped = program.looped.clone();
        // Queue the stream before connecting: the mixer fixes its resampler from the
        // player's reported format, and an empty player reports a placeholder rate.
        let (sink, queue) = Player::new();
        sink.pause();
        let gain = volume.min(256) as f32 / 256.0;
        let envelope = (fade_ms != 0).then(|| Envelope::new(0.0, 1.0, fade_ms));
        sink.set_volume(if envelope.is_some() { 0.0 } else { gain });
        let bytes = program.bytes.clone();
        let source = StreamSource {
            output_enabled: self.output_enabled.clone(),
            mixer_gain: self.mixer_gain.clone(),
            host_control: self.host_control.clone(),
            program,
            compression,
            fault: fault.clone(),
            failed: false,
        };
        let rate = source.sample_rate();
        let pan = Arc::new(AtomicI32::new(0));
        sink.append(StereoBalance {
            source: UniformSourceIterator::new(source, rodio::ChannelCount::new(2).unwrap(), rate),
            pan: Arc::clone(&pan),
            right: false,
        });
        self.mixer.add(queue);
        Channel {
            bytes,
            sink,
            fault,
            envelope,
            release_at_end: false,
            looped,
            gain,
            duck_gain: 1.0,
            script_gain: 1.0,
            pan,
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
            gain * channel.duck_gain
                * channel.script_gain
                * channel.envelope.as_ref().map_or(1.0, Envelope::value),
        );
        channel.gain = gain;
    }
    fn set_slot_script_gain(
        slots: &mut [Option<Channel>],
        channel: u8,
        kind: &str,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let slot = slots
            .get_mut(usize::from(channel))
            .ok_or_else(|| format!("{kind} channel out of range"))?;
        if let Some(sound) = slot {
            sound.script_gain = multiplier as f32 / 256.0;
            Self::set_gain(sound, sound.gain);
        }
        Ok(())
    }
    /// Playing means started (not paused) with source data left; `update` releases
    /// finished channels. The position is the source consumption clock, not a DAC
    /// timestamp, and includes no device-buffer latency.
    fn position(channel: Option<&Channel>) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
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
    fn tick_channel(channel: &mut Option<Channel>) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Some(current) = channel else {
            return Ok(());
        };
        current.check()?;
        if let Some(fade) = current.envelope {
            current
                .sink
                .set_volume(current.gain * current.duck_gain * current.script_gain * fade.value());
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
    fn set_se_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Self::set_slot_script_gain(&mut self.se, channel, "SE", multiplier)
    }
    fn preload(&mut self, slot: u8, name: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let index = usize::from(slot);
        if index >= self.preloaded.len() {
            return Err("preloaded sound slot out of range".into());
        }
        let sound = self
            .load(ResourceKind::SoundEffect, name)?
            .map(|data| AudioProgram::new(data, None, false))
            .transpose()?;
        let loaded = sound.is_some();
        self.preloaded[index] = sound.map(|program| self.prepare(program, 256, 0));
        Ok(loaded)
    }
    fn release_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let index = usize::from(slot);
        let current = self
            .preloaded
            .get(index)
            .ok_or("preloaded sound slot out of range")?;
        if let Some(current) = current {
            current.check()?;
            if current.sink.empty() {
                let program = AudioProgram::from_shared(current.bytes.clone(), looped)?;
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
    fn stop_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(current) = self
            .preloaded
            .get(usize::from(slot))
            .ok_or("preloaded sound slot out of range")?
        {
            current.sink.pause();
        }
        Ok(())
    }
    fn set_preloaded_volume(
        &mut self,
        slot: u8,
        volume: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    fn set_mixer_gain(&mut self, gain: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.mixer_gain.store(gain.min(256), Ordering::Relaxed);
        Ok(())
    }
    fn set_output_enabled(&mut self, enabled: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.output_enabled.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    fn configure_voice_ducking(
        &mut self,
        settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.voice_ducking.configure(settings);
        Ok(())
    }
    fn configure_se(
        &mut self,
        channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut prepared: Vec<Option<Channel>> = (0..SE_CHANNELS).map(|_| None).collect();
        // Load and validate every resource while old sounds keep playing. Candidates
        // remain paused, so a later decode error leaves the audible state unchanged.
        for (index, request) in channels.iter().enumerate() {
            if let Some(request) = request
                && let Some(data) = self.load(ResourceKind::SoundEffect, &request.name)?
            {
                let program = AudioProgram::new(data, None, request.looped)?;
                prepared[index] = Some(self.prepare(program, request.volume, 0));
            }
        }
        for slot in &mut self.se {
            *slot = None;
        }
        for (slot, candidate) in self.se.iter_mut().zip(prepared) {
            *slot = candidate;
        }
        for (channel, request) in self.se.iter().zip(channels) {
            if let (Some(channel), Some(request)) = (channel, request)
                && request.start
            {
                channel.sink.play();
            }
        }
        Ok(())
    }
    fn set_volumes(
        &mut self,
        channels: &[Option<u32>; 15],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.paths != *paths {
            self.paths = paths.clone();
        }
        Ok(())
    }
    fn configure_archive_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), cmvs_core::format::cpz::Cpz7Parameters>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.archives.configure_parameters(parameters);
        Ok(())
    }
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
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
        if usize::from(*channel) >= self.se.len() {
            return Err("SE channel out of range".into());
        }
        let data = self.load(ResourceKind::SoundEffect, name)?;
        let Some(data) = data else {
            self.se[*channel as usize] = None;
            return Ok(false);
        };
        let program = AudioProgram::new(data, None, *looped)?;
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
    fn start(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    fn restart_se(
        &mut self,
        channel: u8,
        looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let slot = usize::from(channel);
        let Some(current) = self.se.get(slot).ok_or("SE channel out of range")? else {
            return Ok(());
        };
        let program = AudioProgram::from_shared(current.bytes.clone(), looped)?;
        let mut candidate = self.prepare(program, (current.gain * 256.0) as u32, 0);
        candidate.script_gain = current.script_gain;
        candidate
            .pan
            .store(current.pan.load(Ordering::Relaxed), Ordering::Relaxed);
        Self::set_gain(&mut candidate, current.gain);
        // All fallible preparation precedes replacing the old device channel.
        self.se[slot] = Some(candidate);
        self.se[slot].as_ref().unwrap().sink.play();
        Ok(())
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        *self
            .voice
            .get_mut(usize::from(channel))
            .ok_or("voice channel out of range")? = None;
        Ok(())
    }
    fn fade_out_voice(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        let channel = self
            .voice
            .get_mut(usize::from(channel))
            .ok_or("voice channel out of range")?;
        Self::fade(channel, ms);
        Ok(())
    }
    fn play_voice(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.load_voice(request, None)
    }
    fn play_voice_with_len(
        &mut self,
        request: &AudioRequest,
    ) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        let mut bytes = 0;
        let loaded = self.load_voice(request, Some(&mut bytes))?;
        Ok(loaded.then_some(bytes))
    }
    fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        *self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")? = None;
        Ok(())
    }
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.channel15 = None;
        Ok(())
    }
    fn fade_out_se(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        let channel = self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")?;
        Self::fade(channel, ms);
        Ok(())
    }
    fn play_bgm(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.play_bgm_with_loop(request, 0)
    }
    fn play_bgm_loop_from(
        &mut self,
        request: &AudioRequest,
        loop_sample: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.play_bgm_with_loop(request, loop_sample)
    }
    fn bgm_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        NativeAudio::bgm_position(self, channel)
    }
    fn pause_se(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        let slot = self
            .se
            .get_mut(usize::from(channel))
            .ok_or("SE channel out of range")?;
        if let Some(current) = slot {
            current.sink.pause();
        }
        Ok(())
    }
    fn set_se_pan(&mut self, channel: u8, pan: i32) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(sound) = self
            .se
            .get(usize::from(channel))
            .ok_or("SE channel out of range")?
        {
            sound.pan.store(pan.clamp(-100, 100), Ordering::Relaxed);
        }
        Ok(())
    }
    fn start_voice(
        &mut self,
        channel: u8,
        looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let slot = self
            .voice
            .get_mut(usize::from(channel))
            .ok_or("voice channel out of range")?;
        if let Some(current) = slot {
            current.looped.store(looped, Ordering::Relaxed);
            current.sink.play();
        }
        Ok(())
    }
    fn set_voice_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Self::set_slot_script_gain(&mut self.voice, channel, "voice", multiplier)
    }
    fn set_voice_pan(&mut self, channel: u8, pan: i32) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(sound) = self
            .voice
            .get(usize::from(channel))
            .ok_or("voice channel out of range")?
        {
            sound.pan.store(pan.clamp(-100, 100), Ordering::Relaxed);
        }
        Ok(())
    }
    fn play_aux_se(
        &mut self,
        request: &AudioRequest,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        let AudioRequest::PlayAuxSe {
            channel,
            name,
            volume,
            fade_in_ms,
            looped,
            start,
        } = request
        else {
            return Err("expected PlayAuxSe".into());
        };
        let slot = usize::from(*channel);
        if slot >= self.aux_se.len() {
            return Err("auxiliary SE channel out of range".into());
        }
        let data = self.load(ResourceKind::SoundEffect, name)?;
        let Some(data) = data else {
            self.aux_se[slot] = None;
            return Ok(false);
        };
        let program = AudioProgram::new(data, None, *looped)?;
        let candidate = self.prepare(program, *volume, if *start { *fade_in_ms } else { 0 });
        self.aux_se[slot] = Some(candidate);
        if *start {
            self.aux_se[slot].as_ref().unwrap().sink.play();
        }
        Ok(true)
    }
    fn stop_aux_se(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        *self
            .aux_se
            .get_mut(usize::from(channel))
            .ok_or("auxiliary SE channel out of range")? = None;
        Ok(())
    }
    fn aux_se_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        let channel = self
            .aux_se
            .get(usize::from(channel))
            .ok_or("auxiliary SE channel out of range")?;
        Self::position(channel.as_ref())
    }
    fn set_aux_se_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Self::set_slot_script_gain(&mut self.aux_se, channel, "auxiliary SE", multiplier)
    }
    fn set_cvns_volumes(
        &mut self,
        bgm: Option<u32>,
        se: &[(u8, u32)],
        voice: Option<u32>,
        aux: Option<u32>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(volume) = bgm.filter(|&v| v <= 256) {
            for channel in self.bgm.iter_mut().flatten() {
                Self::set_gain(channel, volume as f32 / 256.0);
            }
        }
        for &(index, volume) in se {
            if volume <= 256
                && let Some(Some(channel)) = self.se.get_mut(usize::from(index))
            {
                Self::set_gain(channel, volume as f32 / 256.0);
            }
        }
        if let Some(volume) = voice.filter(|&v| v <= 256) {
            for channel in self.voice.iter_mut().flatten() {
                Self::set_gain(channel, volume as f32 / 256.0);
            }
        }
        if let Some(volume) = aux.filter(|&v| v <= 256) {
            for channel in self.aux_se.iter_mut().flatten() {
                Self::set_gain(channel, volume as f32 / 256.0);
            }
        }
        Ok(())
    }
    fn fade_out_bgm(&mut self, channel: u8, ms: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        let channel = self
            .bgm
            .get_mut(usize::from(channel))
            .ok_or("BGM channel out of range")?;
        Self::fade(channel, ms);
        Ok(())
    }
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.bgm = [None, None];
        Ok(())
    }
    fn play_movie_audio(
        &mut self,
        request: &AudioRequest,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
        let candidate = self.prepare(AudioProgram::new(bytes, None, false)?, *volume, 0);
        self.movie = Some(candidate);
        self.last_movie_report_ms = 0;
        println!(
            "# native movie audio: {} offset={offset} size={size}",
            path.escape_ascii()
        );
        self.movie.as_ref().unwrap().sink.play();
        Ok(())
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.movie = None;
        Ok(())
    }
    fn update(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
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
        for channel in self
            .se
            .iter_mut()
            .chain(&mut self.voice)
            .chain(&mut self.aux_se)
        {
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
    fn se_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        let channel = self
            .se
            .get(usize::from(channel))
            .ok_or("SE channel out of range")?;
        Self::position(channel.as_ref())
    }
    fn voice_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        let channel = self
            .voice
            .get(usize::from(channel))
            .ok_or("voice channel out of range")?;
        Self::position(channel.as_ref())
    }
    fn movie_clock(&self) -> Result<Option<MovieAudioClock>, Box<dyn Error + Send + Sync>> {
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

impl NativeAudio {
    fn load_voice(
        &mut self,
        request: &AudioRequest,
        pcm_bytes: Option<&mut u32>,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
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
        let program = AudioProgram::new(data, None, false)?;
        if let Some(length) = pcm_bytes {
            *length = program.primary_pcm_bytes()?;
        }
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

    fn play_bgm_with_loop(
        &mut self,
        request: &AudioRequest,
        loop_sample: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
        let program = AudioProgram::with_loop_frames(first, second, *looped, loop_sample)?;
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
}

#[cfg(test)]
mod output_gate_tests {
    use super::*;

    #[test]
    fn cvns_voice_length_and_channel_balance_reach_pull_output() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let voice = |name: &[u8]| AudioRequest::PlayVoice {
            channel: 0,
            name: name.to_vec(),
            volume: 256,
            start: false,
            rate_bits: 1.0f32.to_bits(),
            mixer: Default::default(),
        };
        for is_voice in [true, false] {
            let (mut audio, mut output) = NativeAudio::pull(&root, 8000, 2).unwrap();
            audio
                .configure_paths(&BTreeMap::from([
                    ((0x2012, 0), b"audio\\".to_vec()),
                    ((0x2014, 0), b"audio\\".to_vec()),
                ]))
                .unwrap();
            if is_voice {
                assert_eq!(
                    audio.play_voice_with_len(&voice(b"missing.ogg")).unwrap(),
                    None
                );
                assert_eq!(
                    audio.play_voice_with_len(&voice(b"mono440.ogg")).unwrap(),
                    Some(768 * 2)
                );
                audio.set_voice_pan(0, -100).unwrap();
                audio.start_voice(0, true).unwrap();
            } else {
                assert!(
                    audio
                        .play(&AudioRequest::PlaySe {
                            channel: 47,
                            name: b"mono440.ogg".to_vec(),
                            volume: 256,
                            looped: true,
                            flag: false,
                            fade_in_ms: 0,
                        })
                        .unwrap()
                );
                audio.set_se_pan(47, -100).unwrap();
                audio.start(47, true).unwrap();
            }
            let mut pcm = [0; 2048];
            output.fill_i16(&mut pcm);
            let frames = pcm.as_chunks::<2>().0;
            assert!(frames.iter().all(|f| f[1] == 0));
            assert!(frames.iter().any(|f| f[0] != 0));
            if is_voice {
                audio.set_voice_pan(0, 100).unwrap();
            } else {
                audio.set_se_pan(47, 100).unwrap();
            }
            output.fill_i16(&mut pcm);
            let frames = pcm.as_chunks::<2>().0;
            assert!(frames.iter().all(|f| f[0] == 0));
            assert!(frames.iter().any(|f| f[1] != 0));
        }
    }

    #[test]
    fn pull_output_resamples_channels_and_pads_with_silence() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
        let bytes = std::fs::read(root.join("mono440.ogg")).unwrap();
        let (audio, mut output) = NativeAudio::pull(&root, 48_000, 2).unwrap();
        let mut silence = [1i16; 64];
        output.fill_i16(&mut silence);
        assert!(silence.iter().all(|&x| x == 0));

        let mut reference = AudioProgram::new(bytes.clone(), None, false).unwrap();
        let mut frames = 0u64;
        while reference.next_sample().unwrap().is_some() {
            frames += 1;
        }
        let expected = frames * 48_000 / u64::from(reference.sample_rate());
        let channel = audio.prepare(AudioProgram::new(bytes, None, false).unwrap(), 256, 0);
        channel.sink.play();
        let mut pcm = vec![0i16; (expected as usize + 4800) * 2];
        output.fill_i16(&mut pcm);
        // Mono input is duplicated to both output channels.
        assert!(pcm.chunks(2).all(|frame| frame[0] == frame[1]));
        let audible = pcm.chunks(2).rposition(|f| f[0] != 0).unwrap() as u64 + 1;
        assert!(audible.abs_diff(expected) < 480, "{audible} vs {expected}");
        assert!(pcm[pcm.len() - 2000..].iter().all(|&x| x == 0));
        assert!(channel.sink.empty());
    }

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
        let bytes = include_bytes!("../tests/fixtures/audio/mono440.ogg");
        let gate = Arc::new(AtomicBool::new(true));
        let host = HostAudioControl {
            active: Arc::new(AtomicBool::new(true)),
        };
        let mut source = StreamSource {
            output_enabled: gate.clone(),
            mixer_gain: Arc::new(AtomicU32::new(256)),
            host_control: host.clone(),
            program: AudioProgram::new(bytes.to_vec(), None, false).unwrap(),
            compression: None,
            fault: Arc::default(),
            failed: false,
        };
        let mut reference = AudioProgram::new(bytes.to_vec(), None, false).unwrap();
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

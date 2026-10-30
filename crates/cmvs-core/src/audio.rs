//! Sound-effect channel and BGM commands with an explicit audio backend.
//! The recording backend keeps requests only; no sound is loaded or played.
pub mod ducking;
use crate::{evaluate::error, machine::Machine, vm::SystemResult};
pub use ducking::{VoiceActivity, VoiceDucking, VoiceDuckingSettings};
use std::error::Error;

/// Six SE channels at +1656, 52 bytes each (snapshot record 0x310); device channels 9..14.
pub const SE_CHANNELS: usize = 6;
/// Four voice channels map to device channels 5..8 (0x140070650).
pub const VOICE_CHANNELS: usize = 4;
/// The file name occupies +1656..+1688 of each channel record.
const NAME_CAPACITY: usize = 31;

/// Settings the original reads at startup (0x14000a3c0) from the registry `param` key or
/// INI `[SOUND_INFO]`; defaults here are the constructor 0x140014e60 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSettings {
    /// +1608, `SE_USE`; when false 0x20b0 records the channel without playing.
    pub se_enabled: bool,
    /// +1612 master volume on the 256 scale.
    pub master: u32,
    /// +1620 SE volume on the 256 scale.
    pub se: u32,
    /// +1600, copied into the BGM object +148 at startup (0x14000aa6e); when false,
    /// 0x20a0/0x20a4 only update the object state.
    pub bgm_enabled: bool,
    /// +1616 BGM volume on the 256 scale.
    pub bgm: u32,
    /// +1576 (menu item 0x807C). 0x140088b00 consults it only when the first BGM stream
    /// is missing, to allow a second lookup root; the meaning is not verified further.
    pub bgm_flag_1576: u32,
    /// +1628 movie volume on the 256 scale (0x20c0 uses (+1612 * +1628) >> 8).
    pub movie: u32,
    /// +1604; when false 0x20a6 only reports failure.
    pub voice_enabled: bool,
    /// +1624 voice volume on the 256 scale.
    pub voice: u32,
}
impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            se_enabled: true,
            master: 256,
            se: 256,
            bgm_enabled: true,
            bgm: 256,
            bgm_flag_1576: 0,
            movie: 256,
            voice_enabled: true,
            voice: 256,
        }
    }
}

/// BGM object at engine +4464 (0xa8 bytes, constructor 0x140089fc0).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BgmState {
    /// +0: first file; for 0x20a4 the decoder opens it together with `secondary`.
    pub primary: Vec<u8>,
    /// +64: second file (empty for 0x20a0); how the decoder combines the two is unverified.
    pub secondary: Vec<u8>,
    /// +152: device channel 0 or 1 of the newest track.
    pub channel: u8,
    /// +156.
    pub playing: bool,
}

/// Channel record kept even while SE is disabled, so re-enabling can restart it (0x140017580).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeChannel {
    pub name: Vec<u8>,
    /// +1688.
    pub looped: bool,
    /// +1692 raw script volume; positive values scale the effective volume by /256.
    pub volume: i32,
    /// +1704: 0x20b0's immediate-start flag; 0x20b1 sets this to true, 0x20b3 clears it.
    pub flag: bool,
}

/// Prepared device sound for a whole-group SE configuration change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SePlayback {
    pub name: Vec<u8>,
    pub volume: u32,
    pub looped: bool,
    pub start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioRequest {
    ConfigureVoiceDucking {
        settings: VoiceDuckingSettings,
    },
    /// Atomically replace SE device channels 9..14; None releases that slot.
    ConfigureSe {
        channels: [Option<SePlayback>; SE_CHANNELS],
    },
    /// Atomic gain update of device channels 0..14; None preserves that channel.
    SetVolumes {
        channels: [Option<u32>; 15],
    },
    /// 0x140088390 on device channel `channel + 9`.
    PlaySe {
        channel: u8,
        name: Vec<u8>,
        volume: u32,
        looped: bool,
        /// Prepare the sound and start it immediately when true.
        flag: bool,
        /// 0x20ba SP-16 (0x20b0 passes 0): when starting immediately and nonzero,
        /// 0x140088240 fades the sound in over this many milliseconds first.
        fade_in_ms: u32,
    },
    /// 0x140088030: start a prepared device channel; an already active channel is unchanged.
    StartSe {
        channel: u8,
        looped: bool,
    },
    /// 0x140087910: stop and release the device channel's sound, prepared or active.
    StopSe {
        channel: u8,
    },
    /// 0x2241: stop and release device channel 15, separate from SE, voice and BGM.
    StopChannel15,
    /// 0x20b6: 0x1400882d0 on device channel `channel + 9` fades the sound out over
    /// `fade_ms` and releases it at the end; a no-op for an empty or already fading slot.
    FadeOutSe {
        channel: u8,
        fade_ms: u32,
    },
    /// 0x20a8: stop and release voice channel 0..3 (device channel +5).
    StopVoice {
        channel: u8,
    },
    /// 0x20a9: fade device channel +5 out, releasing it at the end.
    FadeOutVoice {
        channel: u8,
        fade_ms: u32,
    },
    /// 0x20a6: 0x140088390 on device channel `channel + 5` from the 0x2014 roots, without
    /// loop, fade-in or fallback root; it releases the channel first and starts the sound
    /// when `start` is set.
    PlayVoice {
        channel: u8,
        name: Vec<u8>,
        volume: u32,
        start: bool,
    },
    /// 0x14008a240 with BGM enabled. With `fade_ms` 0 both BGM device channels 0 and 1
    /// stop at once first; otherwise `previous` fades out over `fade_ms`. The track then
    /// loads and starts on `channel` (fading in over `fade_ms`), and both channels get
    /// `volume`. Load failures are not reported to the VM.
    PlayBgm {
        channel: u8,
        previous: u8,
        primary: Vec<u8>,
        secondary: Vec<u8>,
        volume: u32,
        fade_ms: u32,
        looped: bool,
        flag_1576: u32,
    },
    /// 0x14008a1c0 with BGM enabled: `channel` fades out over `fade_ms` (0x1400882d0, a no-op
    /// when the slot is empty or already fading) and is released when the fade ends.
    FadeOutBgm {
        channel: u8,
        fade_ms: u32,
    },
    /// 0x14008a0a0 with BGM enabled: stop and release BGM device channels 0 and 1 at once
    /// (0x140087910 each), the same as a 0x20a0/0x20a4 request without fade does first.
    StopBgm,
    /// 0x140088870 on device channel 2: the movie player starts the Ogg resource stored at
    /// `offset` (absolute) with `size` bytes in the CMV file at `path`, replacing the channel.
    PlayMovieAudio {
        path: Vec<u8>,
        offset: u64,
        size: u32,
        volume: u32,
    },
    /// 0x140087910 on device channel 2, from the player destructor 0x140037890.
    StopMovieAudio,
}

pub trait AudioBackend {
    /// Queue a full parameter change atomically; active cycles keep their old values.
    fn configure_voice_ducking(
        &mut self,
        _settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement voice ducking".into())
    }
    /// Prepare all replacement sounds before changing any SE channel. Err preserves
    /// every old channel; missing sounds replace their slot with None, as in playback.
    fn configure_se(
        &mut self,
        _channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement SE configuration changes".into())
    }
    /// Apply the whole gain update or leave all channels unchanged on error.
    fn set_volumes(&mut self, _channels: &[Option<u32>; 15]) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement live volume changes".into())
    }
    /// Supply the currently registered resource roots before loading a sound.
    fn configure_paths(
        &mut self,
        _paths: &std::collections::BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    /// Maintain output/fades and surface asynchronous decoder/device failures.
    fn update(&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    /// None selects the explicit virtual clock; real backends report channel 2 state.
    fn movie_clock(&self) -> Result<Option<MovieAudioClock>, Box<dyn Error>> {
        Ok(None)
    }
    /// Prepare a PlaySe request, optionally starting it according to its flag.
    /// Ok(true) means loading succeeded (the original's return), Ok(false) means it failed;
    /// Err leaves VM and channel state unchanged for retry.
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>>;
    /// Apply start-if-not-playing on channel + 9, including no-op for a missing sound.
    /// Err must leave the device unchanged, so the VM request can be retried.
    fn start(&mut self, _channel: u8, _looped: bool) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement starting a prepared SE channel".into())
    }
    /// Stop channel + 9 at once (no fade) and release its sound; an empty channel is a no-op.
    /// The original waits until its mixer thread has freed the slot, so Ok means released.
    fn stop(&mut self, _channel: u8) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement stopping an SE channel".into())
    }
    /// 0x1400713a0 calls 0x140087910 on device channel 15. Ok means released;
    /// an empty slot is a no-op, and Err leaves the device unchanged for retry.
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement stopping device channel 15".into())
    }
    /// Start the fade-out of channel + 9 (released when it ends); an empty or already
    /// fading channel is unchanged. Err leaves the device unchanged for retry.
    fn fade_out_se(&mut self, _channel: u8, _fade_ms: u32) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement SE fade-out".into())
    }
    /// Stop and release a voice channel (0..3, device +5), including an empty slot.
    /// Err leaves the device unchanged so the VM can retry the pending command.
    fn stop_voice(&mut self, _channel: u8) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement stopping a voice channel".into())
    }
    /// An empty or already fading voice is unchanged. Err preserves the device.
    fn fade_out_voice(&mut self, _channel: u8, _fade_ms: u32) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement voice fade-out".into())
    }
    /// Apply a PlayVoice request: Ok(true) when the sound loaded, Ok(false) when it did not
    /// (the channel is released either way). Err leaves the device unchanged for retry.
    fn play_voice(&mut self, _request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Err("audio backend does not implement voice playback".into())
    }
    /// Apply a PlayBgm request in full before returning Ok; Err leaves the device unchanged.
    fn play_bgm(&mut self, _request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement BGM playback".into())
    }
    /// Start the fade-out of a BGM device channel; Err leaves the device unchanged.
    fn fade_out_bgm(&mut self, _channel: u8, _fade_ms: u32) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement BGM fade-out".into())
    }
    /// Stop and release both BGM device channels before returning Ok.
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement stopping BGM".into())
    }
    /// Apply a PlayMovieAudio request before returning Ok; Err leaves the device unchanged.
    fn play_movie_audio(&mut self, _request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement movie audio".into())
    }
    /// Stop and release device channel 2 before returning Ok.
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error>> {
        Err("audio backend does not implement stopping movie audio".into())
    }
    /// 0x140087870 on device channel `channel + 9`: Some(milliseconds since the sound
    /// started) while it plays, None when the slot is empty, only prepared, being released
    /// or finished (0x1400892f0 resets +172 to 0 once a stream that does not loop ends).
    fn se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Err("audio backend does not implement SE position queries".into())
    }
    /// The same query on voice channel 0..3 (device channel +5).
    fn voice_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Err("audio backend does not implement voice position queries".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovieAudioClock {
    Playing { position_ms: u32 },
    Ended,
}

#[derive(Debug, Default)]
pub struct RecordingAudio {
    requests: Vec<AudioRequest>,
}
impl RecordingAudio {
    pub fn requests(&self) -> &[AudioRequest] {
        &self.requests
    }
    fn record(&mut self, request: AudioRequest) -> Result<(), Box<dyn Error>> {
        if self.requests.len() >= 4096 {
            return Err("recorded audio requests exceed research limit".into());
        }
        self.requests.push(request);
        Ok(())
    }
}
impl AudioBackend for RecordingAudio {
    fn fade_out_voice(&mut self, channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::FadeOutVoice { channel, fade_ms })
    }
    fn configure_voice_ducking(
        &mut self,
        settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::ConfigureVoiceDucking { settings })
    }
    fn configure_se(
        &mut self,
        channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::ConfigureSe {
            channels: channels.clone(),
        })
    }
    fn set_volumes(&mut self, channels: &[Option<u32>; 15]) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::SetVolumes {
            channels: *channels,
        })
    }
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        self.record(request.clone())?;
        Ok(false)
    }
    fn start(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StartSe { channel, looped })
    }
    fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopSe { channel })
    }
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopChannel15)
    }
    fn fade_out_se(&mut self, channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::FadeOutSe { channel, fade_ms })
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopVoice { channel })
    }
    fn play_voice(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        self.record(request.clone())?;
        Ok(false)
    }
    fn play_bgm(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        self.record(request.clone())
    }
    fn fade_out_bgm(&mut self, channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::FadeOutBgm { channel, fade_ms })
    }
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopBgm)
    }
    fn play_movie_audio(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error>> {
        self.record(request.clone())
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopMovieAudio)
    }
    /// Nothing is ever loaded (play reports failure), so every slot is empty.
    fn se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(None)
    }
    fn voice_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(None)
    }
}

#[derive(Debug)]
pub struct AudioHost {
    settings: AudioSettings,
    /// Device +2424/+2428/+2432/+2436, including the most recent queued update.
    voice_ducking: VoiceDuckingSettings,
    channels: [Option<SeChannel>; SE_CHANNELS],
    /// Channel record +40 (+1696): 0x20b0/0x20ba store 256 with each play, the constructor
    /// zeroes it, and stopping keeps it. Only the snapshot reads it here.
    played: [u32; SE_CHANNELS],
    bgm: BgmState,
}

impl Default for AudioHost {
    fn default() -> Self {
        Self::new(AudioSettings::default())
    }
}

impl AudioHost {
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            settings,
            voice_ducking: VoiceDuckingSettings::default(),
            channels: Default::default(),
            played: [0; SE_CHANNELS],
            bgm: BgmState::default(),
        }
    }
    pub fn bgm(&self) -> &BgmState {
        &self.bgm
    }
    pub fn settings(&self) -> AudioSettings {
        self.settings
    }
    pub fn voice_ducking(&self) -> VoiceDuckingSettings {
        self.voice_ducking
    }
    /// Read-only startup configuration (0x14000a3c0); registry overrides and
    /// configuration persistence are separate backend work. Call before playback.
    pub fn from_ini(text: &[u8]) -> Self {
        let mut host = Self::default();
        let int = |key, default| crate::save_menu::profile_int(text, "SOUND_INFO", key, default);
        host.settings.bgm_enabled = int("MUSIC_USE", 1) != 0;
        host.settings.se_enabled = int("SE_USE", 1) != 0;
        host.settings.voice_enabled = int("VOICE_USE", 1) != 0;
        // 0x140017750 delegates to menu helpers that store the incoming raw word.
        // Unlike the VM setters, startup does not clamp the volume parameters.
        host.settings.master = int("MASTER_VOLUME", 100) as u32;
        host.settings.bgm = int("MUSIC_VOLUME", 100) as u32;
        host.settings.se = int("SE_VOLUME", 100) as u32;
        host.settings.voice = int("VOICE_VOLUME", 100) as u32;
        host.settings.movie = int("VIDEO_VOLUME", 100) as u32;
        host.voice_ducking = VoiceDuckingSettings {
            percent: (int("VOICE_FADE_VOLOME", 100) as u32).min(100),
            fade_ms: (int("VOICE_FADE_TIMER", 500) as u32).min(2000),
            wait_ms: (int("VOICE_FADE_WAIT", 1000) as u32).min(3000),
            include_fading: int("VOICE_FADE_USE", 1) != 0,
        };
        host
    }
    pub fn channel(&self, index: usize) -> Option<&SeChannel> {
        self.channels.get(index).and_then(Option::as_ref)
    }
    /// 0x20b0 / 0x140070b70: SP-4 channel, SP-8 name, SP-12 volume, SP-16 loop, SP-20 flag.
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        match vm.pending_system() {
            Some((_, command @ (0x239 | 0x23b | 0x23d | 0x23f))) => {
                // Device constructor 0x140087440, queried by 0x140087d00.
                let value = match command {
                    0x239 => self.voice_ducking.percent,
                    0x23b => self.voice_ducking.fade_ms,
                    0x23d => self.voice_ducking.wait_ms,
                    _ => u32::from(self.voice_ducking.include_fading),
                };
                vm.complete_system(SystemResult(0x4000))?;
                vm.state_mut()
                    .set_system_integer(0, value)
                    .expect("slot 0 exists");
                return Ok(true);
            }
            Some((pc, command @ (0x238 | 0x23a | 0x23c | 0x23e))) => {
                let raw = vm
                    .stack()
                    .peek_u32(4)
                    .map_err(|e| error(pc, command + 0x2000, e.0))?;
                let mut settings = self.voice_ducking;
                match command {
                    0x238 => settings.percent = (raw as i32).clamp(0, 100) as u32,
                    0x23a => settings.fade_ms = (raw as i32).min(2000) as u32,
                    0x23c => settings.wait_ms = (raw as i32).min(3000) as u32,
                    _ => settings.include_fading = raw != 0,
                }
                settings.percent = settings.percent.min(100);
                backend.configure_voice_ducking(settings)?;
                vm.complete_system(SystemResult(0x4004))?;
                self.voice_ducking = settings;
                return Ok(true);
            }
            Some((_, 0x2d6..=0x2dd)) => return self.try_settings_query(vm, backend),
            Some((_, 0xa8)) => return self.try_voice_stop(vm, backend),
            Some((_, 0xa9)) => return self.try_voice_fade_out(vm, backend),
            Some((_, 0xa6)) => return self.try_voice_play(vm, backend),
            Some((_, 0xb1)) => return self.try_start(vm, backend),
            Some((_, 0xb3)) => return self.try_stop(vm, backend),
            Some((_, 0xb5)) => return self.try_stop_all(vm, backend),
            Some((_, 0x241)) => return self.try_channel15_stop(vm, backend),
            Some((_, 0xb6)) => return self.try_fade_out(vm, backend),
            Some((_, 0xa0 | 0xa4)) => return self.try_bgm(vm, backend),
            Some((_, 0xa2)) => return self.try_bgm_fade_out(vm, backend),
            Some((_, 0xa1)) => return self.try_bgm_stop(vm, backend),
            Some((_, 0xaa | 0xb7)) => return self.try_position(vm, backend),
            _ => {}
        }
        // 0x20b0 / 0x140070b70 and 0x20ba / 0x140070cd0 differ only in 0x20ba's SP-16 fade-in
        // time (passed as 0x140088390's sixth argument), which shifts loop and flag by 4.
        let Some((pc, command @ (0xb0 | 0xba))) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let shift = if command == 0xba { 4 } else { 0 };
        let channel = arg(4)?;
        let (name, volume) = (arg(8)?, arg(12)? as i32);
        let fade_in_ms = if command == 0xba { arg(16)? } else { 0 };
        let (looped, flag) = (arg(16 + shift)? != 0, arg(20 + shift)? != 0);
        if channel as usize >= SE_CHANNELS {
            // The original only sets +12056 bit 0x200, which is not modeled.
            return Err(error(pc, op, "SE channel outside 0..5 sets an unmodeled flag").into());
        }
        let name = vm.resolve_string(name)?;
        if name.len() > NAME_CAPACITY {
            return Err(error(pc, op, "SE file name exceeds the 32-byte record").into());
        }
        let started = if self.settings.se_enabled {
            // Unsigned 32-bit products, as in the original.
            let mut effective = self.settings.master.wrapping_mul(self.settings.se) >> 8;
            if volume > 0 {
                effective = (volume as u32).wrapping_mul(effective) >> 8;
            }
            Some(backend.play(&AudioRequest::PlaySe {
                channel: channel as u8,
                name: name.clone(),
                volume: effective,
                looped,
                flag,
                fade_in_ms,
            })?)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4014 + shift as u16))?;
        if let Some(started) = started {
            vm.state_mut()
                .set_system_integer(0, u32::from(started))
                .expect("slot 0 exists");
        }
        self.channels[channel as usize] = Some(SeChannel {
            name,
            looped,
            volume,
            flag,
        });
        self.played[channel as usize] = 256;
        Ok(true)
    }
    /// Configuration reads and live volume writes use the same playback settings.
    /// Commit writes only after the backend accepts the channel gain transaction.
    fn try_settings_query(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, command) = vm.pending_system().expect("pending configuration command");
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, command + 0x2000, e.0))
        };
        if arg(4)? != 0 {
            if command >= 0x2d9 {
                let value = (arg(8)? as i32).clamp(0, 256) as u32;
                let mut settings = self.settings;
                match command {
                    0x2d9 => settings.master = value,
                    0x2da => settings.bgm = value,
                    0x2db => settings.se = value,
                    0x2dc => settings.voice = value,
                    _ => settings.movie = value,
                }
                let mut channels = [None; 15];
                let product = |volume| settings.master.wrapping_mul(volume) >> 8;
                if matches!(command, 0x2d9 | 0x2da) && settings.bgm_enabled {
                    channels[0..2].fill(Some(product(settings.bgm)));
                }
                if matches!(command, 0x2d9 | 0x2dd) {
                    channels[2..5].fill(Some(product(settings.movie)));
                }
                if matches!(command, 0x2d9 | 0x2dc) {
                    channels[5..9].fill(Some(product(settings.voice)));
                }
                if matches!(command, 0x2d9 | 0x2db) {
                    for (index, channel) in channels[9..15].iter_mut().enumerate() {
                        let mut volume = product(settings.se);
                        // Master adjustment deliberately omits per-SE gain in the original.
                        if command == 0x2db
                            && let Some(se) = &self.channels[index]
                            && se.volume > 0
                        {
                            volume = (se.volume as u32).wrapping_mul(volume) >> 8;
                        }
                        *channel = Some(volume);
                    }
                }
                backend.set_volumes(&channels)?;
                vm.complete_system(SystemResult(0x4008))?;
                self.settings = settings;
                return Ok(true);
            }
            let enabled = arg(8)? != 0;
            let current = match command {
                0x2d6 => self.settings.bgm_enabled,
                0x2d7 => self.settings.se_enabled,
                _ => self.settings.voice_enabled,
            };
            if current != enabled {
                self.set_enabled(command, enabled, pc, backend)?;
            }
            vm.complete_system(SystemResult(0x4008))?;
            return Ok(true);
        }
        if command >= 0x2d9 {
            arg(8)?;
        }
        let value = match command {
            0x2d6 => u32::from(self.settings.bgm_enabled),
            0x2d7 => u32::from(self.settings.se_enabled),
            0x2d8 => u32::from(self.settings.voice_enabled),
            0x2d9 => self.settings.master,
            0x2da => self.settings.bgm,
            0x2db => self.settings.se,
            0x2dc => self.settings.voice,
            0x2dd => self.settings.movie,
            _ => unreachable!(),
        };
        vm.complete_system(SystemResult(0x4008))?;
        vm.state_mut()
            .set_system_integer(0, value)
            .expect("slot 0 exists");
        Ok(true)
    }

    /// 0x140017430/17580/17720: USE switches are not equivalent to muting gains.
    fn set_enabled(
        &mut self,
        command: u16,
        enabled: bool,
        pc: usize,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        match command {
            0x2d6 => {
                if !enabled && self.bgm.playing {
                    backend.stop_bgm()?;
                    self.bgm.channel = 0;
                    self.bgm.playing = false;
                } else if enabled && !self.bgm.primary.is_empty() {
                    backend.play_bgm(&AudioRequest::PlayBgm {
                        channel: 1,
                        previous: self.bgm.channel,
                        primary: self.bgm.primary.clone(),
                        secondary: self.bgm.secondary.clone(),
                        volume: self.settings.master.wrapping_mul(self.settings.bgm) >> 8,
                        fade_ms: 0,
                        looped: true,
                        flag_1576: self.settings.bgm_flag_1576,
                    })?;
                    self.bgm.channel = 1;
                    self.bgm.playing = true;
                }
                self.settings.bgm_enabled = enabled;
            }
            0x2d7 => {
                let mut channels = std::array::from_fn(|_| None);
                if enabled {
                    for (index, target) in channels.iter_mut().enumerate() {
                        if let Some(se) = &self.channels[index]
                            && !se.name.is_empty()
                        {
                            if self.played[index] < 256 {
                                return Err(error(
                                    pc,
                                    command + 0x2000,
                                    "SE pitch restoration is not implemented",
                                )
                                .into());
                            }
                            let mut volume =
                                self.settings.master.wrapping_mul(self.settings.se) >> 8;
                            if se.volume > 0 {
                                volume = (se.volume as u32).wrapping_mul(volume) >> 8;
                            }
                            *target = Some(SePlayback {
                                name: se.name.clone(),
                                volume,
                                looped: se.looped,
                                start: se.looped && se.flag,
                            });
                        }
                    }
                }
                backend.configure_se(&channels)?;
                if enabled {
                    for se in self.channels.iter_mut().flatten() {
                        if !se.name.is_empty() && !se.looped {
                            se.flag = false;
                        }
                    }
                }
                self.settings.se_enabled = enabled;
            }
            0x2d8 => self.settings.voice_enabled = enabled,
            _ => unreachable!(),
        }
        Ok(())
    }

    /// 0x20a9 / 0x140070740: SP-4 voice channel, SP-8 duration; no enable flag gate.
    fn try_voice_fade_out(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let channel = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x20a9, e.0))?;
        let fade_ms = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x20a9, e.0))?;
        if channel as usize >= VOICE_CHANNELS {
            return Err(error(
                pc,
                0x20a9,
                "voice channel outside 0..3 sets an unmodeled flag",
            )
            .into());
        }
        backend.fade_out_voice(channel as u8, fade_ms)?;
        vm.complete_system(SystemResult(0x4008))?;
        Ok(true)
    }

    /// 0x20a8 / 0x140070650: SP-4 voice channel, device +5; no enable flag gate.
    fn try_voice_stop(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let channel = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x20a8, e.0))?;
        if channel as usize >= VOICE_CHANNELS {
            return Err(error(
                pc,
                0x20a8,
                "voice channel outside 0..3 sets an unmodeled flag",
            )
            .into());
        }
        backend.stop_voice(channel as u8)?;
        vm.complete_system(SystemResult(0x4004))?;
        Ok(true)
    }

    /// 0x20a6 / 0x1400703b0: SP-4 voice channel (unsigned, above 3 only sets +12056 bit
    /// 0x400), SP-8 name, SP-12 volume, SP-16 start flag; returns 0x4010. With +1604 off,
    /// system result 0 = 0. Otherwise result 0 reports the load, and result 4 receives
    /// MOG_GetPcmLength through the output pointer (uninitialized stack on failure); its
    /// unit is unverified, so the model marks it unknown. No channel record is kept.
    fn try_voice_play(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, 0x20a6, why);
        let arg = |distance| vm.stack().peek_u32(distance).map_err(|e| fail(e.0));
        let (channel, name, volume, start) = (arg(4)?, arg(8)?, arg(12)? as i32, arg(16)? != 0);
        if channel as usize >= VOICE_CHANNELS {
            return Err(fail("voice channel outside 0..3 sets an unmodeled flag").into());
        }
        let name = vm.resolve_string(name)?;
        let loaded = if self.settings.voice_enabled {
            // 0x140088390 releases the channel and then refuses names of 32 bytes or more;
            // that failure path is not modeled.
            if name.len() >= 32 {
                return Err(fail("voice file name exceeds the 32-byte device limit").into());
            }
            // Unsigned 32-bit products, as for sound effects.
            let mut effective = self.settings.master.wrapping_mul(self.settings.voice) >> 8;
            if volume > 0 {
                effective = (volume as u32).wrapping_mul(effective) >> 8;
            }
            Some(backend.play_voice(&AudioRequest::PlayVoice {
                channel: channel as u8,
                name,
                volume: effective,
                start,
            })?)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4010))?;
        let state = vm.state_mut();
        state
            .set_system_integer(0, u32::from(loaded == Some(true)))
            .expect("slot 0 exists");
        if loaded.is_some() {
            state.forget_system_integer(4).expect("slot 4 exists");
        }
        Ok(true)
    }

    /// 0x20b1 / 0x140070e40: SP-4 channel, SP-8 loop; returns 0x4008, no result write.
    fn try_start(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, 0x20b1, why);
        let channel = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let looped = vm.stack().peek_u32(8).map_err(|e| fail(e.0))? != 0;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        if self.settings.se_enabled {
            backend.start(channel as u8, looped)?;
        }
        vm.complete_system(SystemResult(0x4008))?;
        // The constructor zeroes all six 52-byte records, even without a prior 0x20b0.
        let record = self.channels[channel as usize].get_or_insert_with(SeChannel::default);
        record.looped = looped;
        record.flag = true;
        Ok(true)
    }

    /// 0x20b3 / 0x140070f60: SP-4 channel; returns 0x4004, no result write.
    fn try_stop(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, 0x20b3, why);
        let channel = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        // Unlike 0x20b0/0x20b1, the original stops the device channel without checking SE_USE.
        backend.stop(channel as u8)?;
        vm.complete_system(SystemResult(0x4004))?;
        // An empty name keeps the SE menu toggle (0x140017580) from replaying the channel.
        let record = self.channels[channel as usize].get_or_insert_with(SeChannel::default);
        record.name.clear();
        record.flag = false;
        Ok(true)
    }

    /// 0x2241 / 0x1400713a0: SP-4 must be zero; stop device channel 15.
    /// No SE_USE check or channel-record writes; returns 0x4004, no result write.
    fn try_channel15_stop(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, 0x2241, why);
        let slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        if slot != 0 {
            return Err(fail(
                "device channel 15 slot must be 0; other values set an unmodeled flag",
            )
            .into());
        }
        backend.stop_channel15()?;
        vm.complete_system(SystemResult(0x4004))?;
        Ok(true)
    }

    /// 0x20b5 / 0x140071090: stop all six SE channels, clearing only their names.
    /// No SE_USE check, arguments, or result write; returns 0x4000. Each successful
    /// stop commits its name clear; a backend failure preserves that prefix and the
    /// pending VM command. Retrying safely stops the already empty channels again.
    fn try_stop_all(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        for (channel, record) in self.channels.iter_mut().enumerate() {
            backend.stop(channel as u8)?;
            record.get_or_insert_with(SeChannel::default).name.clear();
        }
        vm.complete_system(SystemResult(0x4000))?;
        Ok(true)
    }

    /// 0x20b6 / 0x1400710f0: SP-4 channel (unsigned, above 5 only sets +12056 bit 0x200),
    /// SP-8 fade milliseconds; returns 0x4008, no result write. Like 0x20b3 it skips the
    /// SE_USE check; it clears only the record's name, keeping loop, volume and flag.
    fn try_fade_out(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, 0x20b6, why);
        let channel = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let fade_ms = vm.stack().peek_u32(8).map_err(|e| fail(e.0))?;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        backend.fade_out_se(channel as u8, fade_ms)?;
        vm.complete_system(SystemResult(0x4008))?;
        self.channels[channel as usize]
            .get_or_insert_with(SeChannel::default)
            .name
            .clear();
        Ok(true)
    }

    /// 0x20b7 / 0x140071170 (SE channel 0..5) and 0x20aa / 0x140070790 (voice channel
    /// 0..3): system result 0 = the 0x140087870 playback position, or −1; pops 4 and
    /// returns 0x4004. Neither checks SE_USE or the voice setting; larger channels only set
    /// +12056 bits 0x200 / 0x400 (unmodeled, rejected here).
    fn try_position(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, command) = vm.pending_system().expect("checked pending command");
        let fail = |why| error(pc, command + 0x2000, why);
        let channel = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let position = if command == 0xb7 {
            if channel as usize >= SE_CHANNELS {
                return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
            }
            backend.se_position(channel as u8)?
        } else {
            if channel as usize >= VOICE_CHANNELS {
                return Err(fail("voice channel outside 0..3 sets an unmodeled flag").into());
            }
            backend.voice_position(channel as u8)?
        };
        vm.complete_system(SystemResult(0x4004))?;
        vm.state_mut()
            .set_system_integer(0, position.unwrap_or(u32::MAX))
            .expect("slot 0 exists");
        Ok(true)
    }

    /// 0x20a0 / 0x1400701b0: SP-4 name, SP-8 fade, SP-12 loop; returns 0x400c.
    /// 0x20a4 / 0x140070250: SP-4 first and SP-8 second name, SP-12 fade, SP-16 loop;
    /// returns 0x4010. Both call 0x14008a240 and write no system result.
    fn try_bgm(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, command) = vm.pending_system().expect("checked pending command");
        let op = command + 0x2000;
        let pair = command == 0xa4;
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let primary = vm.resolve_string(arg(4)?)?;
        let secondary = if pair {
            vm.resolve_string(arg(8)?)?
        } else {
            Vec::new()
        };
        let base = if pair { 8 } else { 4 };
        let (fade_ms, looped) = (arg(base + 4)?, arg(base + 8)? != 0);
        // 0x140088390/0x140088b00 refuse names of 32 bytes or more after stopping the
        // channel; that failure path is not modeled.
        if primary.len() >= 32 || secondary.len() >= 32 {
            return Err(error(pc, op, "BGM file name exceeds the 32-byte device limit").into());
        }
        let enabled = self.settings.bgm_enabled;
        let previous = self.bgm.channel;
        // 0x14008a0a0 resets the channel to 0 only while BGM is enabled.
        let current = if fade_ms == 0 && enabled { 0 } else { previous };
        let channel = current.wrapping_sub(1) & 1;
        if enabled {
            backend.play_bgm(&AudioRequest::PlayBgm {
                channel,
                previous,
                primary: primary.clone(),
                secondary: secondary.clone(),
                // Unsigned 32-bit product, as in the original.
                volume: self.settings.master.wrapping_mul(self.settings.bgm) >> 8,
                fade_ms,
                looped,
                flag_1576: self.settings.bgm_flag_1576,
            })?;
        }
        vm.complete_system(SystemResult(if pair { 0x4010 } else { 0x400c }))?;
        // When disabled, the second 0x14008a0a0 clears +156 again; the time stamp at +160
        // is not modeled.
        self.bgm = BgmState {
            primary,
            secondary,
            channel,
            playing: enabled,
        };
        Ok(true)
    }

    /// 0x20a2 / 0x140070320 → 0x14008a1c0: SP-4 fade milliseconds; returns 0x4004 with no
    /// result write. Names and the playing flag are cleared; with BGM enabled the current
    /// channel fades out and the channel flips to (channel - 1) & 1.
    fn try_bgm_fade_out(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let (pc, _) = vm.pending_system().expect("checked pending command");
        let fade_ms = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x20a2, e.0))?;
        let mut channel = self.bgm.channel;
        if self.settings.bgm_enabled {
            backend.fade_out_bgm(channel, fade_ms)?;
            channel = channel.wrapping_sub(1) & 1;
        }
        vm.complete_system(SystemResult(0x4004))?;
        self.bgm = BgmState {
            channel,
            ..BgmState::default()
        };
        Ok(true)
    }

    /// 0x20a1: the dispatcher calls 0x14008a0a0 inline and returns 0x4000 (nothing popped).
    /// Names and the playing flag are cleared; with BGM enabled both channels stop and the
    /// channel resets to 0.
    fn try_bgm_stop(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let mut channel = self.bgm.channel;
        if self.settings.bgm_enabled {
            backend.stop_bgm()?;
            channel = 0;
        }
        vm.complete_system(SystemResult(0x4000))?;
        self.bgm = BgmState {
            channel,
            ..BgmState::default()
        };
        Ok(true)
    }
}

/// Record 0x310 length: six 52-byte channel records at +1656.
pub const SAVED_CHANNELS: usize = SE_CHANNELS * 52;

impl AudioHost {
    /// Record 0x304: the BGM names at +4464 and +4464+64.
    pub(crate) fn saved_bgm(&self) -> [Vec<u8>; 2] {
        [self.bgm.primary.clone(), self.bgm.secondary.clone()]
    }

    /// Record 0x310: name (+0, 32 bytes), loop (+32), volume (+36), +40, +44, flag (+48) per
    /// channel. 0x20b0/0x20ba store 256 and 0 at +40/+44; only unimplemented commands change
    /// them. A stopped channel keeps its old name behind a cleared first byte in the
    /// original; Rust writes the cleared name as zeros.
    pub(crate) fn saved_channels(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SAVED_CHANNELS);
        for (channel, played) in self.channels.iter().zip(self.played) {
            let mut raw = [0u8; 52];
            if let Some(c) = channel {
                raw[..c.name.len()].copy_from_slice(&c.name);
                for (at, value) in [
                    (32, u32::from(c.looped)),
                    (36, c.volume as u32),
                    (48, u32::from(c.flag)),
                ] {
                    raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            raw[40..44].copy_from_slice(&played.to_le_bytes());
            out.extend(raw);
        }
        out
    }

    /// Loader 0x140081960 then reader 0x140069fd0: stop BGM (0x14008a0a0) and every device
    /// sound (0x1400879e0 / 0x140089060), clear the channel records, then restart a saved
    /// BGM without fade and with looping (0x14008a240), and replay each named SE channel
    /// (0x140088390 on channel + 9, starting at once only when looped and flagged). SE_USE
    /// is not consulted by the loader.
    pub(crate) fn restore(
        &mut self,
        bgm: &[Vec<u8>; 2],
        record: &[u8],
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        if record.len() != SAVED_CHANNELS {
            return Err("SE channel record has unexpected length".into());
        }
        let mut channels: [Option<SeChannel>; SE_CHANNELS] = Default::default();
        let mut played = [0; SE_CHANNELS];
        for (i, slot) in channels.iter_mut().enumerate() {
            let raw = &record[52 * i..52 * (i + 1)];
            let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
            let end = raw[..32]
                .iter()
                .position(|&b| b == 0)
                .ok_or("SE name lacks its terminator")?;
            let name = raw[..end].to_vec();
            if !name.is_empty() && (word(40) != 256 || word(44) != 0) {
                return Err("saved SE pitch or pan settings are not modeled".into());
            }
            played[i] = word(40);
            if raw.iter().any(|&b| b != 0) {
                *slot = Some(SeChannel {
                    name,
                    looped: word(32) != 0,
                    volume: word(36) as i32,
                    flag: word(48) != 0,
                });
            }
        }
        if bgm.iter().any(|name| name.len() >= 32) {
            return Err("saved BGM name exceeds the device limit".into());
        }
        let enabled = self.settings.bgm_enabled;
        if enabled {
            backend.stop_bgm()?;
        }
        for channel in 0..SE_CHANNELS as u8 {
            backend.stop(channel)?;
        }
        for channel in 0..VOICE_CHANNELS as u8 {
            backend.stop_voice(channel)?;
        }
        backend.stop_channel15()?;
        let mut current = if enabled { 0 } else { self.bgm.channel };
        self.bgm = BgmState {
            channel: current,
            ..BgmState::default()
        };
        if !bgm[0].is_empty() {
            let channel = current.wrapping_sub(1) & 1;
            if enabled {
                backend.play_bgm(&AudioRequest::PlayBgm {
                    channel,
                    previous: current,
                    primary: bgm[0].clone(),
                    secondary: bgm[1].clone(),
                    volume: self.settings.master.wrapping_mul(self.settings.bgm) >> 8,
                    fade_ms: 0,
                    looped: true,
                    flag_1576: self.settings.bgm_flag_1576,
                })?;
            }
            current = channel;
            self.bgm = BgmState {
                primary: bgm[0].clone(),
                secondary: bgm[1].clone(),
                channel: current,
                playing: enabled,
            };
        }
        for (i, channel) in channels.iter().enumerate() {
            let Some(c) = channel.as_ref().filter(|c| !c.name.is_empty()) else {
                continue;
            };
            let mut volume = self.settings.master.wrapping_mul(self.settings.se) >> 8;
            if c.volume > 0 {
                volume = (c.volume as u32).wrapping_mul(volume) >> 8;
            }
            let start = c.looped && c.flag;
            backend.play(&AudioRequest::PlaySe {
                channel: i as u8,
                name: c.name.clone(),
                volume,
                looped: c.looped,
                flag: start,
                fade_in_ms: 0,
            })?;
        }
        self.channels = channels;
        self.played = played;
        Ok(())
    }
}

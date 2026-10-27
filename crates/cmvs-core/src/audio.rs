//! Sound-effect channel and BGM commands with an explicit audio backend.
//! The recording backend keeps requests only; no sound is loaded or played.
use crate::{evaluate::error, machine::Machine, vm::SystemResult};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioRequest {
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
    StartSe { channel: u8, looped: bool },
    /// 0x140087910: stop and release the device channel's sound, prepared or active.
    StopSe { channel: u8 },
    /// 0x20b6: 0x1400882d0 on device channel `channel + 9` fades the sound out over
    /// `fade_ms` and releases it at the end; a no-op for an empty or already fading slot.
    FadeOutSe { channel: u8, fade_ms: u32 },
    /// 0x20a8: stop and release voice channel 0..3 (device channel +5).
    StopVoice { channel: u8 },
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
    FadeOutBgm { channel: u8, fade_ms: u32 },
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
    fn fade_out_se(&mut self, channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::FadeOutSe { channel, fade_ms })
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
        self.record(AudioRequest::StopVoice { channel })
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
}

#[derive(Debug, Default)]
pub struct AudioHost {
    settings: AudioSettings,
    channels: [Option<SeChannel>; SE_CHANNELS],
    bgm: BgmState,
}

impl AudioHost {
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            settings,
            channels: Default::default(),
            bgm: BgmState::default(),
        }
    }
    pub fn bgm(&self) -> &BgmState {
        &self.bgm
    }
    pub fn settings(&self) -> AudioSettings {
        self.settings
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
            Some((_, 0xa8)) => return self.try_voice_stop(vm, backend),
            Some((_, 0xb1)) => return self.try_start(vm, backend),
            Some((_, 0xb3)) => return self.try_stop(vm, backend),
            Some((_, 0xb6)) => return self.try_fade_out(vm, backend),
            Some((_, 0xa0 | 0xa4)) => return self.try_bgm(vm, backend),
            Some((_, 0xa2)) => return self.try_bgm_fade_out(vm, backend),
            Some((_, 0xa1)) => return self.try_bgm_stop(vm, backend),
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

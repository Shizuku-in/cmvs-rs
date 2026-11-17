//! Sound-effect channel and BGM commands with an explicit audio backend.
//! The recording backend keeps requests only; no sound is loaded or played.
pub mod compress;
pub mod ducking;
use crate::{FormatError, dispatch::Call, evaluate::error};
pub use compress::{VoiceCompressor, VoiceMixer};
pub use ducking::{VoiceActivity, VoiceDucking, VoiceDuckingSettings};
use std::error::Error;

/// Six SE channels at +1656, 52 bytes each (snapshot record 0x310); device channels 9..14.
pub const SE_CHANNELS: usize = 6;
/// Four voice channels map to device channels 5..8 (0x140070650).
pub const VOICE_CHANNELS: usize = 4;
/// Independent prepared-sound pool in the inspected 2007 device.
pub const PRELOADED_SOUNDS: usize = 24;
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
    /// Device-wide software mixer gain on the 256 scale (cmvs-20070119 0x20a5).
    SetMixerGain {
        gain: u32,
    },
    SetSeGain {
        channel: u8,
        multiplier: u32,
    },
    Preload {
        slot: u8,
        name: Vec<u8>,
    },
    ReleasePreloaded {
        slot: u8,
    },
    PlayPreloaded {
        slot: u8,
        looped: bool,
        volume: u32,
    },
    StopPreloaded {
        slot: u8,
    },
    SetPreloadedVolume {
        slot: u8,
        volume: u32,
    },
    /// Silence output while consuming audio normally; playback and gains are retained.
    SetOutputEnabled {
        enabled: bool,
    },
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
    /// Hash-bound cmvs-20071117 0x20b4: restart even an active sound from its beginning.
    RestartSe {
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
    /// 0x20a6/0x20af: 0x140088390 on device channel `channel + 5` from the 0x2014 roots,
    /// without loop, fade-in or fallback root; it releases the channel first and starts
    /// the sound when `start` is set.
    PlayVoice {
        channel: u8,
        name: Vec<u8>,
        volume: u32,
        start: bool,
        /// f32 bits of the playback rate (1.0 for 0x20a6). Mono sounds at 1.1..=3.0 run
        /// through `VoiceCompressor` in chunks of `mixer`.
        rate_bits: u32,
        mixer: VoiceMixer,
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
    /// CVNS 0x20a7 (0x459540): BGM playback looping from PCM sample frame `loop_sample`.
    PlayBgmLoopFrom {
        channel: u8,
        previous: u8,
        primary: Vec<u8>,
        secondary: Vec<u8>,
        volume: u32,
        fade_ms: u32,
        looped: bool,
        flag_1576: u32,
        loop_sample: u32,
    },
    /// CVNS 0x20b2 (0x45a690): pause an SE channel without releasing or clearing its name.
    PauseSe {
        channel: u8,
    },
    /// CVNS 0x20af (0x45aa80): per-channel SE stereo pan (-100..=100).
    SetSePan {
        channel: u8,
        pan: i32,
    },
    /// CVNS 0x20bd (0x45b4a0): voice channel gain multiplier (/256 scale).
    SetVoiceGain {
        channel: u8,
        multiplier: u32,
    },
    /// CVNS 0x20be (0x45b550): voice channel stereo pan (-100..=100).
    SetVoicePan {
        channel: u8,
        pan: i32,
    },
    /// CVNS 0x20b9 (0x45b180): start or resume a prepared voice channel.
    StartVoice {
        channel: u8,
        looped: bool,
    },
    /// CVNS 0x20c5 (0x45ab50): auxiliary SE playback (device slots 60..63 -> 0..3).
    PlayAuxSe {
        channel: u8,
        name: Vec<u8>,
        volume: u32,
        fade_in_ms: u32,
        looped: bool,
        start: bool,
    },
    /// CVNS 0x20c7 (0x45ad20): stop auxiliary SE channel (0..3).
    StopAuxSe {
        channel: u8,
    },
    /// CVNS 0x20ef (cvns-20050216 0x42d4f0): auxiliary SE gain multiplier (/256 scale).
    SetAuxSeGain {
        channel: u8,
        multiplier: u32,
    },
    /// CVNS `sub_41FCC0` live volume update across all CVNS SE (0..48), BGM, voice, and aux slots.
    UpdateCvnsVolumes {
        bgm: Option<u32>,
        se: Vec<(u8, u32)>,
        voice: Option<u32>,
        aux: Option<u32>,
    },
}

/// CVNS allocates 48 script SE slots (device channels 12..60, `v4 < 0x30`).
pub const CVNS_SE_CHANNELS: usize = 48;
/// CVNS auxiliary master-scaled SE slots (`0x20c5..=0x20c7`, device channels 60..63).
pub const AUX_SE_CHANNELS: usize = 4;

pub trait AudioBackend: Send {
    /// Device-wide /256 gain applied by the software mixer to every channel's samples,
    /// on top of each channel's own volume. Err leaves the gain unchanged.
    fn set_mixer_gain(&mut self, _gain: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement a device mixer gain".into())
    }
    /// Independent raw /256 multiplier; preserve playback, base volume and envelopes.
    fn set_se_gain(
        &mut self,
        _channel: u8,
        _multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement SE gain multipliers".into())
    }
    /// Replace an independent prepared sound without starting it. Missing resources
    /// clear the slot and return false; Err preserves the slot for retry.
    fn preload(&mut self, _slot: u8, _name: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement preloading".into())
    }
    fn release_preloaded(&mut self, _slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement releasing preloaded sounds".into())
    }
    fn play_preloaded(
        &mut self,
        _slot: u8,
        _looped: bool,
        _volume: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement playing preloaded sounds".into())
    }
    fn stop_preloaded(&mut self, _slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping preloaded sounds".into())
    }
    fn set_preloaded_volume(
        &mut self,
        _slot: u8,
        _volume: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement preloaded sound volumes".into())
    }
    /// Apply a device-wide output gate without stopping streams or changing their gains.
    /// Err leaves the gate unchanged.
    fn set_output_enabled(&mut self, _enabled: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement output gating".into())
    }

    /// Queue a full parameter change atomically; active cycles keep their old values.
    fn configure_voice_ducking(
        &mut self,
        _settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice ducking".into())
    }
    /// Prepare all replacement sounds before changing any SE channel. Err preserves
    /// every old channel; missing sounds replace their slot with None, as in playback.
    fn configure_se(
        &mut self,
        _channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement SE configuration changes".into())
    }
    /// Apply the whole gain update or leave all channels unchanged on error.
    fn set_volumes(
        &mut self,
        _channels: &[Option<u32>; 15],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement live volume changes".into())
    }
    /// Supply the currently registered resource roots before loading a sound.
    fn configure_paths(
        &mut self,
        _paths: &std::collections::BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    /// Supply per-slot CPZ7 keys before loading a sound. Recording backends need no I/O.
    fn configure_archive_parameters(
        &mut self,
        _parameters: &std::collections::BTreeMap<(u16, u32), crate::cpz::Cpz7Parameters>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    /// Maintain output/fades and surface asynchronous decoder/device failures.
    fn update(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
    /// None selects the explicit virtual clock; real backends report channel 2 state.
    fn movie_clock(&self) -> Result<Option<MovieAudioClock>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    /// Prepare a PlaySe request, optionally starting it according to its flag.
    /// Ok(true) means loading succeeded (the original's return), Ok(false) means it failed;
    /// Err leaves VM and channel state unchanged for retry.
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>>;
    /// Apply start-if-not-playing on channel + 9, including no-op for a missing sound.
    /// Err must leave the device unchanged, so the VM request can be retried.
    fn start(&mut self, _channel: u8, _looped: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement starting a prepared SE channel".into())
    }
    /// Restart an existing sound, including an active sound; missing sounds are unchanged.
    /// Err preserves the device for retry. This is distinct from start-if-not-playing.
    fn restart_se(
        &mut self,
        _channel: u8,
        _looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement restarting an SE channel".into())
    }
    /// Stop channel + 9 at once (no fade) and release its sound; an empty channel is a no-op.
    /// The original waits until its mixer thread has freed the slot, so Ok means released.
    fn stop(&mut self, _channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping an SE channel".into())
    }
    /// 0x1400713a0 calls 0x140087910 on device channel 15. Ok means released;
    /// an empty slot is a no-op, and Err leaves the device unchanged for retry.
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping device channel 15".into())
    }
    /// Start the fade-out of channel + 9 (released when it ends); an empty or already
    /// fading channel is unchanged. Err leaves the device unchanged for retry.
    fn fade_out_se(
        &mut self,
        _channel: u8,
        _fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement SE fade-out".into())
    }
    /// Stop and release a voice channel (0..3, device +5), including an empty slot.
    /// Err leaves the device unchanged so the VM can retry the pending command.
    fn stop_voice(&mut self, _channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping a voice channel".into())
    }
    /// An empty or already fading voice is unchanged. Err preserves the device.
    fn fade_out_voice(
        &mut self,
        _channel: u8,
        _fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice fade-out".into())
    }
    /// Apply a PlayVoice request: Ok(true) when the sound loaded, Ok(false) when it did not
    /// (the channel is released either way). Err leaves the device unchanged for retry.
    fn play_voice(
        &mut self,
        _request: &AudioRequest,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice playback".into())
    }
    /// Apply a PlayBgm request in full before returning Ok; Err leaves the device unchanged.
    fn play_bgm(&mut self, _request: &AudioRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement BGM playback".into())
    }
    /// Start the fade-out of a BGM device channel; Err leaves the device unchanged.
    fn fade_out_bgm(
        &mut self,
        _channel: u8,
        _fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement BGM fade-out".into())
    }
    /// Stop and release both BGM device channels before returning Ok.
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping BGM".into())
    }
    /// Apply a PlayMovieAudio request before returning Ok; Err leaves the device unchanged.
    fn play_movie_audio(
        &mut self,
        _request: &AudioRequest,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement movie audio".into())
    }
    /// Stop and release device channel 2 before returning Ok.
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping movie audio".into())
    }
    /// 0x140087870 on device channel `channel + 9`: Some(milliseconds since the sound
    /// started) while it plays, None when the slot is empty, only prepared, being released
    /// or finished (0x1400892f0 resets +172 to 0 once a stream that does not loop ends).
    fn se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement SE position queries".into())
    }
    /// The same query on voice channel 0..3 (device channel +5).
    fn voice_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice position queries".into())
    }
    /// CVNS 0x20a4 (0x4596d0 -> 0x40f680): active BGM playback position in ms, or None.
    fn bgm_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement BGM position queries".into())
    }
    /// CVNS 0x20a7 (0x459540): `play_bgm` with loop restart skipping to PCM frame `loop_sample`.
    fn play_bgm_loop_from(
        &mut self,
        request: &AudioRequest,
        loop_sample: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if loop_sample == 0 {
            return self.play_bgm(request);
        }
        Err("audio backend does not implement BGM loop start offsets".into())
    }
    /// CVNS 0x20b2 (0x45a690): pause SE channel without clearing or freeing.
    fn pause_se(&mut self, _channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement pausing an SE channel".into())
    }
    /// CVNS 0x20af (0x45aa80): set SE channel stereo balance (-100..=100).
    fn set_se_pan(&mut self, _channel: u8, _pan: i32) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement SE panning".into())
    }
    /// CVNS 0x20b9 (0x45b180): start or resume a prepared voice channel.
    fn start_voice(
        &mut self,
        _channel: u8,
        _looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement starting a voice channel".into())
    }
    /// CVNS 0x20bd (0x45b4a0): set voice channel gain multiplier (/256 scale).
    fn set_voice_gain(
        &mut self,
        _channel: u8,
        _multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice gain multipliers".into())
    }
    /// CVNS 0x20be (0x45b550): set voice channel stereo balance (-100..=100).
    fn set_voice_pan(
        &mut self,
        _channel: u8,
        _pan: i32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement voice panning".into())
    }
    /// CVNS 0x20b8 (0x45af90): load/play voice and return decoded PCM byte length when loaded.
    fn play_voice_with_len(
        &mut self,
        request: &AudioRequest,
    ) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        self.play_voice(request).map(|ok| ok.then_some(0))
    }
    /// CVNS 0x20c5 (0x45ab50): load/play auxiliary SE channel 0..3.
    fn play_aux_se(
        &mut self,
        _request: &AudioRequest,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement auxiliary SE playback".into())
    }
    /// CVNS 0x20c7 (0x45ad20): stop auxiliary SE channel 0..3.
    fn stop_aux_se(&mut self, _channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement stopping auxiliary SE".into())
    }
    /// CVNS 0x20c6 (0x45adb0): query auxiliary SE position in ms, or None.
    fn aux_se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement auxiliary SE position queries".into())
    }
    /// CVNS 0x20ef (cvns-20050216 0x42d4f0): set auxiliary SE gain multiplier (/256 scale).
    fn set_aux_se_gain(
        &mut self,
        _channel: u8,
        _multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement auxiliary SE gain multipliers".into())
    }
    /// CVNS `sub_41FCC0`: update BGM, 48-slot SE, voice, and auxiliary SE gains together.
    fn set_cvns_volumes(
        &mut self,
        _bgm: Option<u32>,
        _se: &[(u8, u32)],
        _voice: Option<u32>,
        _aux: Option<u32>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("audio backend does not implement CVNS multi-channel volume updates".into())
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
    fn record(&mut self, request: AudioRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.requests.len() >= 4096 {
            return Err("recorded audio requests exceed research limit".into());
        }
        self.requests.push(request);
        Ok(())
    }
}
impl AudioBackend for RecordingAudio {
    fn set_mixer_gain(&mut self, gain: u32) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetMixerGain { gain })
    }
    fn set_se_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetSeGain {
            channel,
            multiplier,
        })
    }
    fn play_preloaded(
        &mut self,
        slot: u8,
        looped: bool,
        volume: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::PlayPreloaded {
            slot,
            looped,
            volume,
        })
    }
    fn stop_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopPreloaded { slot })
    }
    fn set_preloaded_volume(
        &mut self,
        slot: u8,
        volume: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetPreloadedVolume { slot, volume })
    }
    fn preload(&mut self, slot: u8, name: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::Preload {
            slot,
            name: name.to_vec(),
        })?;
        Ok(false)
    }
    fn release_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::ReleasePreloaded { slot })
    }
    fn set_output_enabled(&mut self, enabled: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetOutputEnabled { enabled })
    }

    fn fade_out_voice(
        &mut self,
        channel: u8,
        fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::FadeOutVoice { channel, fade_ms })
    }
    fn configure_voice_ducking(
        &mut self,
        settings: VoiceDuckingSettings,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::ConfigureVoiceDucking { settings })
    }
    fn configure_se(
        &mut self,
        channels: &[Option<SePlayback>; SE_CHANNELS],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::ConfigureSe {
            channels: channels.clone(),
        })
    }
    fn set_volumes(
        &mut self,
        channels: &[Option<u32>; 15],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetVolumes {
            channels: *channels,
        })
    }
    fn play(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.record(request.clone())?;
        Ok(false)
    }
    fn start(&mut self, channel: u8, looped: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StartSe { channel, looped })
    }
    fn restart_se(
        &mut self,
        channel: u8,
        looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::RestartSe { channel, looped })
    }
    fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopSe { channel })
    }
    fn stop_channel15(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopChannel15)
    }
    fn fade_out_se(
        &mut self,
        channel: u8,
        fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::FadeOutSe { channel, fade_ms })
    }
    fn stop_voice(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopVoice { channel })
    }
    fn play_voice(&mut self, request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.record(request.clone())?;
        Ok(false)
    }
    fn play_bgm(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(request.clone())
    }
    fn fade_out_bgm(
        &mut self,
        channel: u8,
        fade_ms: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::FadeOutBgm { channel, fade_ms })
    }
    fn stop_bgm(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopBgm)
    }
    fn play_movie_audio(
        &mut self,
        request: &AudioRequest,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(request.clone())
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopMovieAudio)
    }
    /// Nothing is ever loaded (play reports failure), so every slot is empty.
    fn se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    fn voice_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    fn bgm_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    fn play_bgm_loop_from(
        &mut self,
        request: &AudioRequest,
        loop_sample: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if loop_sample == 0 {
            return self.play_bgm(request);
        }
        let AudioRequest::PlayBgm {
            channel,
            previous,
            primary,
            secondary,
            volume,
            fade_ms,
            looped,
            flag_1576,
        } = request
        else {
            return Err("expected PlayBgm".into());
        };
        self.record(AudioRequest::PlayBgmLoopFrom {
            channel: *channel,
            previous: *previous,
            primary: primary.clone(),
            secondary: secondary.clone(),
            volume: *volume,
            fade_ms: *fade_ms,
            looped: *looped,
            flag_1576: *flag_1576,
            loop_sample,
        })
    }
    fn pause_se(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::PauseSe { channel })
    }
    fn set_se_pan(&mut self, channel: u8, pan: i32) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetSePan { channel, pan })
    }
    fn start_voice(
        &mut self,
        channel: u8,
        looped: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StartVoice { channel, looped })
    }
    fn set_voice_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetVoiceGain {
            channel,
            multiplier,
        })
    }
    fn set_voice_pan(&mut self, channel: u8, pan: i32) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetVoicePan { channel, pan })
    }
    fn play_aux_se(
        &mut self,
        request: &AudioRequest,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.record(request.clone())?;
        Ok(false)
    }
    fn stop_aux_se(&mut self, channel: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::StopAuxSe { channel })
    }
    fn aux_se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    fn set_aux_se_gain(
        &mut self,
        channel: u8,
        multiplier: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::SetAuxSeGain {
            channel,
            multiplier,
        })
    }
    fn set_cvns_volumes(
        &mut self,
        bgm: Option<u32>,
        se: &[(u8, u32)],
        voice: Option<u32>,
        aux: Option<u32>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.record(AudioRequest::UpdateCvnsVolumes {
            bgm,
            se: se.to_vec(),
            voice,
            aux,
        })
    }
}

fn volume_channel_updates(
    command: u16,
    settings: &AudioSettings,
    se_channels: &[Option<SeChannel>; SE_CHANNELS],
) -> [Option<u32>; 15] {
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
                && let Some(se) = &se_channels[index]
                && se.volume > 0
            {
                volume = (se.volume as u32).wrapping_mul(volume) >> 8;
            }
            *channel = Some(volume);
        }
    }
    channels
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CvnsAudioState {
    enabled: bool,
    extra_channels: Vec<Option<SeChannel>>,
    gains: Vec<u32>,
    pans: Vec<i32>,
    voice_gains: [u32; VOICE_CHANNELS],
    voice_pans: [i32; VOICE_CHANNELS],
    aux_channels: [Option<SeChannel>; AUX_SE_CHANNELS],
    aux_gains: [u32; AUX_SE_CHANNELS],
    bgm_loop_sample: u32,
}

#[derive(Debug)]
pub struct AudioHost {
    background_output: bool,
    output_active: bool,
    output_enabled: bool,
    settings: AudioSettings,
    /// CVNS percentage-scale volumes (`MASTER/MUSIC/SE/VOICE/VIDEO_VOLUME`, `10..=100` on VM writes).
    cvns_volumes: [u32; 5],
    /// Device +2424/+2428/+2432/+2436, including the most recent queued update.
    voice_ducking: VoiceDuckingSettings,
    channels: [Option<SeChannel>; SE_CHANNELS],
    mixer: VoiceMixer,
    /// Channel record +40 (+1696): 0x20b0/0x20ba store 256 with each play, the constructor
    /// zeroes it, and stopping keeps it. HAROUSE 0x20b8 writes the raw gain multiplier.
    played: [u32; SE_CHANNELS],
    bgm: BgmState,
    cvns: CvnsAudioState,
}

impl Default for AudioHost {
    fn default() -> Self {
        Self::new(AudioSettings::default())
    }
}

impl AudioHost {
    /// Configure this host for CVNS (48 SE channels, 100-based volume & gain scaling, aux SE).
    pub fn enable_cvns(&mut self) {
        if !self.cvns.enabled {
            self.cvns.enabled = true;
            self.cvns
                .extra_channels
                .resize_with(CVNS_SE_CHANNELS - SE_CHANNELS, || None);
            self.cvns.gains.resize(CVNS_SE_CHANNELS, 100);
            self.cvns.pans.resize(CVNS_SE_CHANNELS, 0);
            self.cvns.voice_gains = [100; VOICE_CHANNELS];
            self.cvns.voice_pans = [0; VOICE_CHANNELS];
            self.cvns.aux_gains = [100; AUX_SE_CHANNELS];
        }
    }

    fn cvns_se_slot(&self, index: usize) -> Option<&SeChannel> {
        if index < SE_CHANNELS {
            self.channels[index].as_ref()
        } else {
            self.cvns
                .extra_channels
                .get(index - SE_CHANNELS)
                .and_then(|s| s.as_ref())
        }
    }

    fn cvns_se_slot_mut(&mut self, index: usize) -> &mut Option<SeChannel> {
        self.enable_cvns();
        if index < SE_CHANNELS {
            &mut self.channels[index]
        } else {
            &mut self.cvns.extra_channels[index - SE_CHANNELS]
        }
    }

    /// cmvs-20071117 0x442fb0/0x443050, selected only by the hash-bound variant.
    pub(crate) fn handle_preload(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let slot = call.argument(4, "preloaded sound slot")?;
        if slot >= PRELOADED_SOUNDS as u32 {
            return Err(error(call.pc, call.command, "preloaded sound slot outside 0..23").into());
        }
        if call.command == 0x248 {
            let reference = call.argument(8, "sound name")?;
            let name = call.vm.resolve_string(reference)?;
            // The original drops the old sound before rejecting a too-long name.
            let loaded = if name.len() > NAME_CAPACITY {
                backend.release_preloaded(slot as u8)?;
                false
            } else {
                backend.preload(slot as u8, &name)?
            };
            call.complete(0x4008)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(loaded))?;
        } else if call.command == 0x249 {
            backend.release_preloaded(slot as u8)?;
            call.complete(0x4004)?;
        } else if call.command == 0x24a {
            let scale = call.argument(8, "volume")? as i32;
            let looped = call.argument(12, "loop")? != 0;
            let mut volume = self.settings.master.wrapping_mul(self.settings.se) >> 8;
            if scale > 0 {
                volume = volume.wrapping_mul(scale as u32) >> 8;
            }
            if self.settings.se_enabled {
                backend.play_preloaded(slot as u8, looped, volume)?;
            }
            call.complete(0x400c)?;
        } else if call.command == 0x24b {
            backend.stop_preloaded(slot as u8)?;
            call.complete(0x4004)?;
        } else {
            let volume = self.settings.master.wrapping_mul(self.settings.se) >> 8;
            if self.settings.se_enabled {
                backend.set_preloaded_volume(slot as u8, volume)?;
            }
            call.complete(0x4004)?;
        }
        Ok(())
    }
    /// QUIT/RESET: 0x14008a0a0 followed by 0x1400879e0. RESET also zeroes
    /// engine +1656..+1968, the six SE records. Mixer preferences survive.
    pub(crate) fn stop_for_lifecycle(
        &mut self,
        backend: &mut dyn AudioBackend,
        reset: bool,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.settings.bgm_enabled {
            backend.stop_bgm()?;
        }
        let se_count = if self.cvns.enabled {
            CVNS_SE_CHANNELS
        } else {
            SE_CHANNELS
        };
        for channel in 0..se_count as u8 {
            backend.stop(channel)?;
        }
        for channel in 0..VOICE_CHANNELS as u8 {
            backend.stop_voice(channel)?;
        }
        if self.cvns.enabled {
            for channel in 0..AUX_SE_CHANNELS as u8 {
                backend.stop_aux_se(channel)?;
            }
        } else {
            backend.stop_channel15()?;
        }
        self.bgm = BgmState {
            channel: if self.settings.bgm_enabled {
                0
            } else {
                self.bgm.channel
            },
            ..BgmState::default()
        };
        self.cvns.bgm_loop_sample = 0;
        if reset {
            self.channels = Default::default();
            self.played = [0; SE_CHANNELS];
            if self.cvns.enabled {
                self.cvns.extra_channels.fill(None);
                self.cvns.gains.fill(100);
                self.cvns.pans.fill(0);
                self.cvns.voice_gains = [100; VOICE_CHANNELS];
                self.cvns.voice_pans = [0; VOICE_CHANNELS];
                self.cvns.aux_channels = Default::default();
                self.cvns.aux_gains = [100; AUX_SE_CHANNELS];
            }
        }
        Ok(())
    }
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            background_output: true,
            output_active: true,
            output_enabled: true,
            settings,
            cvns_volumes: [100; 5],
            voice_ducking: VoiceDuckingSettings::default(),
            channels: Default::default(),
            mixer: VoiceMixer::default(),
            played: [0; SE_CHANNELS],
            bgm: BgmState::default(),
            cvns: CvnsAudioState::default(),
        }
    }
    pub fn background_output(&self) -> bool {
        self.background_output
    }
    pub fn output_enabled(&self) -> bool {
        self.output_enabled
    }
    /// The device combines its foreground flag with the script's background switch.
    pub fn sync_output_activity(
        &mut self,
        active: bool,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let enabled = active || self.background_output;
        if enabled != self.output_enabled {
            backend.set_output_enabled(enabled)?;
        }
        self.output_active = active;
        self.output_enabled = enabled;
        Ok(())
    }
    /// Identity-bound 0x22ed research entry; the root build has no handler.
    pub(crate) fn handle_background_output(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let write = call.argument(4, "write")? != 0;
        let background = if write {
            call.argument(8, "background output")? != 0
        } else {
            call.ignore_argument(8, "unused background output")?;
            self.background_output
        };
        if write {
            let enabled = self.output_active || background;
            if enabled != self.output_enabled {
                backend.set_output_enabled(enabled)?;
            }
            call.complete(0x4008)?;
            self.background_output = background;
            self.output_enabled = enabled;
        } else {
            call.complete(0x4008)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(background))?;
        }
        Ok(())
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
    /// Read-only startup configuration (0x14000a3c0) bound to `profile`.
    pub fn for_profile(profile: &crate::profile::EngineProfile, text: &[u8]) -> Self {
        let mut host = Self::from_ini(text);
        if profile.is_cvns() {
            host.enable_cvns();
        }
        host
    }

    /// Read-only startup configuration (0x14000a3c0); registry overrides and
    /// configuration persistence are separate backend work. Call before playback.
    pub fn from_ini(text: &[u8]) -> Self {
        let mut host = Self::default();
        let int = |key, default| crate::save_menu::profile_int(text, "SOUND_INFO", key, default);
        // Aoitori 0x14000d1d7: INI defaults to 1 and any nonzero word enables output.
        host.background_output = int("INACTIVE_SOUND_PLAY", 1) != 0;
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
        host.cvns_volumes = [
            host.settings.master,
            host.settings.bgm,
            host.settings.se,
            host.settings.voice,
            host.settings.movie,
        ];
        host.voice_ducking = VoiceDuckingSettings {
            percent: (int("VOICE_FADE_VOLOME", 100) as u32).min(100),
            fade_ms: (int("VOICE_FADE_TIMER", 500) as u32).min(2000),
            wait_ms: (int("VOICE_FADE_WAIT", 1000) as u32).min(3000),
            include_fading: int("VOICE_FADE_USE", 1) != 0,
        };
        host.mixer = VoiceMixer::from_ini(int("SOUND_QUALITY", 0), int("SOUND_BUFFER", 0));
        host
    }
    pub fn se_channel_count(&self) -> usize {
        if self.cvns.enabled {
            CVNS_SE_CHANNELS
        } else {
            SE_CHANNELS
        }
    }
    pub fn channel(&self, index: usize) -> Option<&SeChannel> {
        if self.cvns.enabled {
            self.cvns_se_slot(index)
        } else {
            self.channels.get(index).and_then(Option::as_ref)
        }
    }

    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        match call.command {
            command @ (0x239 | 0x23b | 0x23d | 0x23f) => {
                // Device constructor 0x140087440, queried by 0x140087d00.
                let value = match command {
                    0x239 => self.voice_ducking.percent,
                    0x23b => self.voice_ducking.fade_ms,
                    0x23d => self.voice_ducking.wait_ms,
                    _ => u32::from(self.voice_ducking.include_fading),
                };
                call.complete(0x4000)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, value)
                    .expect("slot 0 exists");
                return Ok(());
            }
            command @ (0x238 | 0x23a | 0x23c | 0x23e) => {
                let raw = call.argument(4, "value")?;
                let mut settings = self.voice_ducking;
                match command {
                    0x238 => settings.percent = (raw as i32).clamp(0, 100) as u32,
                    0x23a => settings.fade_ms = (raw as i32).min(2000) as u32,
                    0x23c => settings.wait_ms = (raw as i32).min(3000) as u32,
                    _ => settings.include_fading = raw != 0,
                }
                settings.percent = settings.percent.min(100);
                backend.configure_voice_ducking(settings)?;
                call.complete(0x4004)?;
                self.voice_ducking = settings;
                return Ok(());
            }
            0x2d6..=0x2dd => return self.try_settings_query(call, backend),
            0xa8 => return self.try_voice_stop(call, backend),
            0xa9 => return self.try_voice_fade_out(call, backend),
            0xa6 | 0xaf => return self.try_voice_play(call, backend),
            0xb1 | 0xb4 => return self.try_start(call, backend),
            0xb3 => return self.try_stop(call, backend),
            0xb5 => return self.try_stop_all(call, backend),
            0x241 => return self.try_channel15_stop(call, backend),
            0xb6 => return self.try_fade_out(call, backend),
            0xa0 | 0xa4 => return self.try_bgm(call, backend),
            0xa2 => return self.try_bgm_fade_out(call, backend),
            0xa1 => return self.try_bgm_stop(call, backend),
            0xaa | 0xb7 => return self.try_position(call, backend),
            _ => {}
        }
        // 0x20b0 / 0x140070b70: channel, name, volume, loop, start in callee order.
        // 0x20ba / 0x140070cd0 differs only in its SP-16 fade-in
        // time (passed as 0x140088390's sixth argument), which shifts loop and flag by 4.
        let command = call.command;
        let pc = call.pc;
        debug_assert!(matches!(command, 0xb0 | 0xba));
        let op = command + 0x2000;
        let shift = if command == 0xba { 4 } else { 0 };
        let channel = call.argument(4, "channel")?;
        let (name, volume) = (
            call.argument(8, "name")?,
            call.argument(12, "volume")? as i32,
        );
        let fade_in_ms = if command == 0xba {
            call.argument(16, "fade milliseconds")?
        } else {
            0
        };
        let (looped, flag) = (
            call.argument(16 + shift, "loop")? != 0,
            call.argument(20 + shift, "start")? != 0,
        );
        if channel as usize >= SE_CHANNELS {
            // The original only sets +12056 bit 0x200, which is not modeled.
            return Err(error(pc, op, "SE channel outside 0..5 sets an unmodeled flag").into());
        }
        let name = call.vm.resolve_string(name)?;
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
        call.complete(0x4014 + shift as u16)?;
        if let Some(started) = started {
            call.vm
                .state_mut()
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
        Ok(())
    }
    /// Configuration reads and live volume writes use the same playback settings.
    /// Commit writes only after the backend accepts the channel gain transaction.
    fn try_settings_query(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, command) = (call.pc, call.command);
        if call.argument(4, "mode")? != 0 {
            if command >= 0x2d9 {
                let value = (call.argument(8, "value")? as i32).clamp(0, 256) as u32;
                let mut settings = self.settings;
                match command {
                    0x2d9 => settings.master = value,
                    0x2da => settings.bgm = value,
                    0x2db => settings.se = value,
                    0x2dc => settings.voice = value,
                    _ => settings.movie = value,
                }
                let channels = volume_channel_updates(command, &settings, &self.channels);
                backend.set_volumes(&channels)?;
                call.complete(0x4008)?;
                self.settings = settings;
                return Ok(());
            }
            let enabled = call.argument(8, "value")? != 0;
            self.cvns_set_enabled(command, enabled, pc, backend)?;
            call.complete(0x4008)?;
            return Ok(());
        }
        if command >= 0x2d9 {
            call.argument(8, "value")?;
        } else {
            call.ignore_argument(8, "value")?;
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
        call.complete(0x4008)?;
        call.vm
            .state_mut()
            .set_system_integer(0, value)
            .expect("slot 0 exists");
        Ok(())
    }

    pub(crate) fn cvns_volume(&self, kind: usize) -> u32 {
        self.cvns_volumes[kind]
    }

    pub(crate) fn cvns_movie_volume(&self) -> u32 {
        Self::cvns_percent_to_gain(self.cvns_volumes[0] * self.cvns_volumes[4] / 100)
    }

    fn cvns_percent_to_gain(percent: u32) -> u32 {
        (percent.min(100) * 256 / 100).min(256)
    }

    fn cvns_bgm_volume_for(volumes: &[u32; 5]) -> u32 {
        Self::cvns_percent_to_gain(volumes[0] * volumes[1] / 100)
    }

    fn cvns_se_volume_for(volumes: &[u32; 5], call_vol: i32) -> u32 {
        let mut percent = volumes[0] * volumes[2] / 100;
        if call_vol > 0 {
            percent = (call_vol as u32).saturating_mul(percent) / 100;
        }
        Self::cvns_percent_to_gain(percent)
    }

    fn cvns_voice_volume_for(volumes: &[u32; 5], call_vol: i32) -> u32 {
        let mut percent = volumes[0] * volumes[3] / 100;
        if call_vol > 0 {
            percent = (call_vol as u32).saturating_mul(percent) / 100;
        }
        Self::cvns_percent_to_gain(percent)
    }

    pub(crate) fn stage_cvns_volume(
        &self,
        kind: usize,
        raw_value: i32,
        backend: &mut dyn AudioBackend,
    ) -> Result<(u32, AudioSettings), Box<dyn Error + Send + Sync>> {
        let percent = raw_value.clamp(10, 100) as u32;
        let scaled = percent * 256 / 100;
        let mut settings = self.settings;
        let command = 0x2d9 + kind as u16;
        match kind {
            0 => settings.master = scaled,
            1 => settings.bgm = scaled,
            2 => settings.se = scaled,
            3 => settings.voice = scaled,
            _ => settings.movie = scaled,
        }
        if self.cvns.enabled {
            let mut next = self.cvns_volumes;
            next[kind] = percent;
            let bgm = matches!(kind, 0 | 1).then(|| Self::cvns_bgm_volume_for(&next));
            let voice =
                matches!(kind, 0 | 3).then(|| Self::cvns_percent_to_gain(next[0] * next[3] / 100));
            let aux =
                matches!(kind, 0 | 4).then(|| Self::cvns_percent_to_gain(next[0] * next[4] / 100));
            let se: Vec<(u8, u32)> = if matches!(kind, 0 | 2) {
                (0..CVNS_SE_CHANNELS)
                    .map(|idx| {
                        let call_vol = if kind == 2 {
                            self.cvns_se_slot(idx).map_or(0, |s| s.volume)
                        } else {
                            // `sub_41FCC0` case 0 uses `master * SE_VOLUME / 100` without per-channel scale.
                            0
                        };
                        (idx as u8, Self::cvns_se_volume_for(&next, call_vol))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            backend.set_cvns_volumes(bgm, &se, voice, aux)?;
        } else {
            let channels = volume_channel_updates(command, &settings, &self.channels);
            backend.set_volumes(&channels)?;
        }
        Ok((percent, settings))
    }

    pub(crate) fn commit_cvns_volume(
        &mut self,
        kind: usize,
        percent: u32,
        settings: AudioSettings,
    ) {
        self.cvns_volumes[kind] = percent;
        self.settings = settings;
    }

    pub(crate) fn cvns_set_enabled(
        &mut self,
        command: u16,
        enabled: bool,
        pc: usize,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let current = match command {
            0x2d6 => self.settings.bgm_enabled,
            0x2d7 => self.settings.se_enabled,
            _ => self.settings.voice_enabled,
        };
        if current != enabled {
            if self.cvns.enabled && matches!(command, 0x2d6 | 0x2d7) {
                self.cvns_apply_enabled(command, enabled, backend)?;
            } else {
                self.set_enabled(command, enabled, pc, backend)?;
            }
        }
        Ok(())
    }

    fn cvns_apply_enabled(
        &mut self,
        command: u16,
        enabled: bool,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if command == 0x2d6 {
            if !enabled && self.bgm.playing {
                backend.stop_bgm()?;
                self.bgm.channel = 0;
                self.bgm.playing = false;
            } else if enabled && !self.bgm.primary.is_empty() {
                let req = AudioRequest::PlayBgm {
                    channel: 1,
                    previous: self.bgm.channel,
                    primary: self.bgm.primary.clone(),
                    secondary: self.bgm.secondary.clone(),
                    volume: Self::cvns_bgm_volume_for(&self.cvns_volumes),
                    fade_ms: 0,
                    looped: true,
                    flag_1576: self.settings.bgm_flag_1576,
                };
                backend.play_bgm_loop_from(&req, self.cvns.bgm_loop_sample)?;
                self.bgm.channel = 1;
                self.bgm.playing = true;
            }
            self.settings.bgm_enabled = enabled;
            return Ok(());
        }
        if !enabled {
            for ch in 0..CVNS_SE_CHANNELS as u8 {
                backend.stop(ch)?;
            }
        } else {
            let volumes = self.cvns_volumes;
            for idx in 0..CVNS_SE_CHANNELS {
                let Some(se) = self.cvns_se_slot(idx).cloned() else {
                    continue;
                };
                if se.name.is_empty() {
                    continue;
                }
                let volume = Self::cvns_se_volume_for(&volumes, se.volume);
                backend.play(&AudioRequest::PlaySe {
                    channel: idx as u8,
                    name: se.name,
                    volume,
                    looped: se.looped,
                    flag: false,
                    fade_in_ms: 0,
                })?;
                if self.cvns.gains[idx] < 100 {
                    let mult = (u64::from(self.cvns.gains[idx]) * 256 / 100) as u32;
                    backend.set_se_gain(idx as u8, mult)?;
                }
                if self.cvns.pans[idx] != 0 {
                    backend.set_se_pan(idx as u8, self.cvns.pans[idx])?;
                }
                if se.looped && se.flag {
                    backend.start(idx as u8, true)?;
                } else if let Some(slot) = self.cvns_se_slot_mut(idx).as_mut() {
                    slot.flag = false;
                }
            }
        }
        self.settings.se_enabled = enabled;
        Ok(())
    }

    /// 0x140017430/17580/17720: USE switches are not equivalent to muting gains.
    fn set_enabled(
        &mut self,
        command: u16,
        enabled: bool,
        pc: usize,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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
                            if self.played[index] != 256 {
                                return Err(error(
                                    pc,
                                    command + 0x2000,
                                    "SE gain multiplier restoration is not implemented",
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

    /// 0x20a9 / 0x140070740 and CVNS 0x20bb / 0x45b2d0: SP-4 voice channel, SP-8 duration;
    /// no enable flag gate.
    fn try_voice_fade_out(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, op) = (call.pc, call.command + 0x2000);
        let channel = call.argument(4, "channel")?;
        let fade_ms = call.argument(8, "fade milliseconds")?;
        if channel as usize >= VOICE_CHANNELS {
            return Err(error(pc, op, "voice channel outside 0..3 sets an unmodeled flag").into());
        }
        backend.fade_out_voice(channel as u8, fade_ms)?;
        call.complete(0x4008)?;
        Ok(())
    }

    /// 0x20a8 / 0x140070650 and CVNS 0x20ba / 0x45b240: SP-4 voice channel, device +5;
    /// no enable flag gate.
    fn try_voice_stop(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, op) = (call.pc, call.command + 0x2000);
        let channel = call.argument(4, "channel")?;
        if channel as usize >= VOICE_CHANNELS {
            return Err(error(pc, op, "voice channel outside 0..3 sets an unmodeled flag").into());
        }
        backend.stop_voice(channel as u8)?;
        call.complete(0x4004)?;
        Ok(())
    }

    /// 0x20a6 / 0x1400703b0: SP-4 voice channel (unsigned, above 3 only sets +12056 bit
    /// 0x400), SP-8 name, SP-12 volume, SP-16 start flag; returns 0x4010. With +1604 off,
    /// system result 0 = 0. Otherwise result 0 reports the load, and result 4 receives
    /// MOG_GetPcmLength through the output pointer (uninitialized stack on failure); its
    /// unit is unverified, so the model marks it unknown. No channel record is kept.
    /// 0x20af / 0x1400704c0 adds an SP-20 f32 rate, replaced by 1.0 when below 1.0 or
    /// above 3.0 (NaN is kept), and returns 0x4014.
    fn try_voice_play(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = call.pc;
        let op = 0x2000 + call.command;
        let fail = |why| error(pc, op, why);
        let (channel, name, volume, start) = (
            call.argument(4, "channel")?,
            call.argument(8, "name")?,
            call.argument(12, "volume")? as i32,
            call.argument(16, "start")? != 0,
        );
        let rate = if op == 0x20af {
            let rate = f32::from_bits(call.argument(20, "rate")?);
            // 0x14007051e..0x140070534 keeps NaN, which a range check would replace.
            #[allow(clippy::manual_range_contains)]
            let out_of_range = 1.0 > rate || rate > 3.0;
            if out_of_range { 1.0 } else { rate }
        } else {
            1.0
        };
        if channel as usize >= VOICE_CHANNELS {
            return Err(fail("voice channel outside 0..3 sets an unmodeled flag").into());
        }
        let name = call.vm.resolve_string(name)?;
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
                rate_bits: rate.to_bits(),
                mixer: self.mixer,
            })?)
        } else {
            None
        };
        call.complete(if op == 0x20af { 0x4014 } else { 0x4010 })?;
        let state = call.vm.state_mut();
        state
            .set_system_integer(0, u32::from(loaded == Some(true)))
            .expect("slot 0 exists");
        if loaded.is_some() {
            state.forget_system_integer(4).expect("slot 4 exists");
        }
        Ok(())
    }

    /// cmvs-20070119 0x20a5 -> 0x434aa0 -> 0x4427e0: SP-4 is clamped (unsigned) to 256 and
    /// stored at device +0x904; the mixer 0x441b50 multiplies every channel by it >> 8.
    /// Returns 0x4004 without a result write.
    pub(crate) fn set_mixer_gain(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let gain = call.argument(4, "mixer gain")?.min(256);
        backend.set_mixer_gain(gain)?;
        call.complete(0x4004)?;
        Ok(())
    }

    /// CVNS 0x20ac (cvns-20090203 0x4597e0 -> CDvSound 0x412820): SP-4 clamped to 100 is
    /// the software mixer's master percentage (0x412bf0 scales every channel by it).
    /// The backend gain is /256, so percentages other than multiples of 25 are rounded down.
    pub(crate) fn set_master_percent(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let percent = call.argument(4, "master volume percent")?.min(100);
        backend.set_mixer_gain(percent * 256 / 100)?;
        call.complete(0x4004)?;
        Ok(())
    }

    /// HAROUSE 0x447750 -> 0x456290, device slot channel+9 and record +40.
    pub(crate) fn set_se_gain(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let channel = call.argument(4, "SE channel")? as usize;
        let multiplier = call.argument(8, "SE gain multiplier")?;
        if channel >= SE_CHANNELS {
            return Err(call
                .error("SE channel outside 0..5 sets an unmodeled flag")
                .into());
        }
        backend.set_se_gain(channel as u8, multiplier)?;
        call.complete(0x4008)?;
        self.played[channel] = multiplier;
        Ok(())
    }

    /// 0x20b1 / 0x140070e40: SP-4 channel, SP-8 loop; returns 0x4008, no result write.
    /// Hash-bound cmvs-20071117 0x20b4 / 0x4434d0 uses restart instead of start-if-not-playing.
    fn try_start(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = call.pc;
        let op = call.command + 0x2000;
        let fail = |why| error(pc, op, why);
        let channel = call.argument(4, "channel")?;
        let looped = call.argument(8, "loop")? != 0;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        if self.settings.se_enabled {
            if call.command == 0xb4 {
                backend.restart_se(channel as u8, looped)?;
            } else {
                backend.start(channel as u8, looped)?;
            }
        }
        call.complete(0x4008)?;
        // The constructor zeroes all six 52-byte records, even without a prior 0x20b0.
        let record = self.channels[channel as usize].get_or_insert_with(SeChannel::default);
        record.looped = looped;
        record.flag = true;
        Ok(())
    }

    /// 0x20b3 / 0x140070f60: SP-4 channel; returns 0x4004, no result write.
    fn try_stop(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = call.pc;
        let fail = |why| error(pc, 0x20b3, why);
        let channel = call.argument(4, "channel")?;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        // Unlike 0x20b0/0x20b1, the original stops the device channel without checking SE_USE.
        backend.stop(channel as u8)?;
        call.complete(0x4004)?;
        // An empty name keeps the SE menu toggle (0x140017580) from replaying the channel.
        let record = self.channels[channel as usize].get_or_insert_with(SeChannel::default);
        record.name.clear();
        record.flag = false;
        Ok(())
    }

    /// 0x2241 / 0x1400713a0: SP-4 must be zero; stop device channel 15.
    /// No SE_USE check or channel-record writes; returns 0x4004, no result write.
    fn try_channel15_stop(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = call.pc;
        let fail = |why| error(pc, 0x2241, why);
        let slot = call.argument(4, "slot")?;
        if slot != 0 {
            return Err(fail(
                "device channel 15 slot must be 0; other values set an unmodeled flag",
            )
            .into());
        }
        backend.stop_channel15()?;
        call.complete(0x4004)?;
        Ok(())
    }

    /// 0x20b5 / 0x140071090: stop all six SE channels, clearing only their names.
    /// No SE_USE check, arguments, or result write; returns 0x4000. Each successful
    /// stop commits its name clear; a backend failure preserves that prefix and the
    /// pending VM command. Retrying safely stops the already empty channels again.
    fn try_stop_all(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        for (channel, record) in self.channels.iter_mut().enumerate() {
            backend.stop(channel as u8)?;
            record.get_or_insert_with(SeChannel::default).name.clear();
        }
        call.complete(0x4000)?;
        Ok(())
    }

    /// 0x20b6 / 0x1400710f0: SP-4 channel (unsigned, above 5 only sets +12056 bit 0x200),
    /// SP-8 fade milliseconds; returns 0x4008, no result write. Like 0x20b3 it skips the
    /// SE_USE check; it clears only the record's name, keeping loop, volume and flag.
    fn try_fade_out(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = call.pc;
        let fail = |why| error(pc, 0x20b6, why);
        let channel = call.argument(4, "channel")?;
        let fade_ms = call.argument(8, "fade milliseconds")?;
        if channel as usize >= SE_CHANNELS {
            return Err(fail("SE channel outside 0..5 sets an unmodeled flag").into());
        }
        backend.fade_out_se(channel as u8, fade_ms)?;
        call.complete(0x4008)?;
        self.channels[channel as usize]
            .get_or_insert_with(SeChannel::default)
            .name
            .clear();
        Ok(())
    }

    /// 0x20b7 / 0x140071170 (SE channel 0..5) and 0x20aa / 0x140070790 (voice channel
    /// 0..3): system result 0 = the 0x140087870 playback position, or −1; pops 4 and
    /// returns 0x4004. Neither checks SE_USE or the voice setting; larger channels only set
    /// +12056 bits 0x200 / 0x400 (unmodeled, rejected here).
    fn try_position(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, command) = (call.pc, call.command);
        let fail = |why| error(pc, command + 0x2000, why);
        let channel = call.argument(4, "channel")?;
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
        call.complete(0x4004)?;
        call.vm
            .state_mut()
            .set_system_integer(0, position.unwrap_or(u32::MAX))
            .expect("slot 0 exists");
        Ok(())
    }

    /// 0x20a0 / 0x1400701b0: SP-4 name, SP-8 fade, SP-12 loop; returns 0x400c.
    /// 0x20a4 / 0x140070250: SP-4 first and SP-8 second name, SP-12 fade, SP-16 loop;
    /// returns 0x4010. Both call 0x14008a240 and write no system result.
    fn try_bgm(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        let pair = command == 0xa4;
        let primary_ref = call.argument(4, "primary name")?;
        let primary = call.vm.resolve_string(primary_ref)?;
        let secondary = if pair {
            let reference = call.argument(8, "secondary name")?;
            call.vm.resolve_string(reference)?
        } else {
            Vec::new()
        };
        let base = if pair { 8 } else { 4 };
        let (fade_ms, looped) = (
            call.argument(base + 4, "fade milliseconds")?,
            call.argument(base + 8, "loop")? != 0,
        );
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
        call.complete(if pair { 0x4010 } else { 0x400c })?;
        // When disabled, the second 0x14008a0a0 clears +156 again; the time stamp at +160
        // is not modeled.
        self.bgm = BgmState {
            primary,
            secondary,
            channel,
            playing: enabled,
        };
        Ok(())
    }

    /// 0x20a2 / 0x140070320 → 0x14008a1c0: SP-4 fade milliseconds; returns 0x4004 with no
    /// result write. Names and the playing flag are cleared; with BGM enabled the current
    /// channel fades out and the channel flips to (channel - 1) & 1.
    fn try_bgm_fade_out(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let fade_ms = call.argument(4, "fade milliseconds")?;
        let mut channel = self.bgm.channel;
        if self.settings.bgm_enabled {
            backend.fade_out_bgm(channel, fade_ms)?;
            channel = channel.wrapping_sub(1) & 1;
        }
        call.complete(0x4004)?;
        self.bgm = BgmState {
            channel,
            ..BgmState::default()
        };
        Ok(())
    }

    /// 0x20a1: the dispatcher calls 0x14008a0a0 inline and returns 0x4000 (nothing popped).
    /// Names and the playing flag are cleared; with BGM enabled both channels stop and the
    /// channel resets to 0.
    fn try_bgm_stop(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut channel = self.bgm.channel;
        if self.settings.bgm_enabled {
            backend.stop_bgm()?;
            channel = 0;
        }
        call.complete(0x4000)?;
        self.bgm = BgmState {
            channel,
            ..BgmState::default()
        };
        Ok(())
    }

    /// CVNS audio dispatch (`0x20a0..=0x20c7`), using 48 SE slots, percentage scaling, and
    /// 64-byte names (`cvns.exe` `0x459440..0x45b694`).
    pub(crate) fn handle_cvns(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.enable_cvns();
        let command = call.command;
        match command {
            0x4a0 | 0x4a7 => self.try_cvns_bgm(call, backend),
            0x4a4 => self.try_cvns_bgm_position(call, backend),
            0x4ad | 0x4b0 => self.try_cvns_se_play(call, backend),
            0x4ae | 0x4b1 => self.try_cvns_se_start(call, backend),
            0x4af => self.try_cvns_se_pan(call, backend),
            0x4b2 => self.try_cvns_se_pause(call, backend),
            0x4b3 => self.try_cvns_se_stop(call, backend),
            0x4b4 => self.try_cvns_se_stop_all(call, backend),
            0x4b5 => self.try_cvns_se_fade_out(call, backend),
            0x4b6 => self.try_cvns_se_position(call, backend),
            0x4b7 => self.try_cvns_se_gain(call, backend),
            0x4b8 => self.try_cvns_voice_play(call, backend),
            0x4b9 => self.try_cvns_voice_start(call, backend),
            0x4ba => self.try_voice_stop(call, backend),
            0x4bb => self.try_voice_fade_out(call, backend),
            0x4bc => self.try_cvns_voice_position(call, backend),
            0x4bd => self.try_cvns_voice_gain(call, backend),
            0x4be => self.try_cvns_voice_pan(call, backend),
            0x4bf => self.try_cvns_voice_level(call),
            0x4c5..=0x4c7 | 0x4ef | 0x4ff => self.try_cvns_aux_se(call, backend),
            _ => Err(call.error("unregistered CVNS audio command").into()),
        }
    }

    fn try_cvns_bgm(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (pc, command) = (call.pc, call.command);
        let op = if command == 0x4a7 { 0x20a7 } else { 0x20a0 };
        let name_ref = call.argument(4, "BGM name")?;
        let primary = call.vm.resolve_string(name_ref)?;
        let fade_ms = call.argument(8, "fade milliseconds")?;
        let looped = call.argument(12, "loop")? != 0;
        let loop_sample = if command == 0x4a7 {
            call.argument(16, "loop sample")?
        } else {
            0
        };
        if primary.len() >= 64 {
            return Err(error(pc, op, "BGM file name exceeds the 64-byte device limit").into());
        }
        let enabled = self.settings.bgm_enabled;
        let previous = self.bgm.channel;
        let current = if fade_ms == 0 && enabled { 0 } else { previous };
        let channel = current.wrapping_sub(1) & 1;
        if enabled {
            let req = AudioRequest::PlayBgm {
                channel,
                previous,
                primary: primary.clone(),
                secondary: Vec::new(),
                volume: Self::cvns_bgm_volume_for(&self.cvns_volumes),
                fade_ms,
                looped,
                flag_1576: self.settings.bgm_flag_1576,
            };
            backend.play_bgm_loop_from(&req, loop_sample)?;
        }
        call.complete(if command == 0x4a7 { 0x4010 } else { 0x400c })?;
        self.bgm = BgmState {
            primary,
            secondary: Vec::new(),
            channel,
            playing: enabled,
        };
        self.cvns.bgm_loop_sample = loop_sample;
        Ok(())
    }

    fn try_cvns_bgm_position(
        &mut self,
        call: &mut Call<'_>,
        backend: &dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let position = backend.bgm_position(self.bgm.channel)?;
        call.complete(0x4000)?;
        call.vm
            .state_mut()
            .set_system_integer(0, position.unwrap_or(u32::MAX))
            .expect("slot 0 exists");
        Ok(())
    }

    fn check_cvns_se_channel(
        call: &mut Call<'_>,
        op: u16,
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let channel = call.argument(4, "channel")?;
        if channel as usize >= CVNS_SE_CHANNELS {
            return Err(error(
                call.pc,
                op,
                "CVNS SE channel outside 0..47 sets an unmodeled flag",
            )
            .into());
        }
        Ok(channel as usize)
    }

    fn check_cvns_voice_channel(
        call: &mut Call<'_>,
        op: u16,
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let channel = call.argument(4, "channel")?;
        if channel as usize >= VOICE_CHANNELS {
            return Err(error(
                call.pc,
                op,
                "CVNS voice channel outside 0..3 sets an unmodeled flag",
            )
            .into());
        }
        Ok(channel as usize)
    }

    fn try_cvns_se_play(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let with_fade = call.command == 0x4ad;
        let op = if with_fade { 0x20ad } else { 0x20b0 };
        let shift = if with_fade { 4 } else { 0 };
        let ch = Self::check_cvns_se_channel(call, op)?;
        let name_ref = call.argument(8, "name")?;
        let volume = call.argument(12, "volume")? as i32;
        let fade_in_ms = if with_fade {
            call.argument(16, "fade milliseconds")?
        } else {
            0
        };
        let looped = call.argument(16 + shift, "loop")? != 0;
        let flag = call.argument(20 + shift, "start")? != 0;
        let name = call.vm.resolve_string(name_ref)?;
        if name.len() >= 64 {
            return Err(error(call.pc, op, "SE file name exceeds the 64-byte record").into());
        }
        let started = if self.settings.se_enabled {
            Some(backend.play(&AudioRequest::PlaySe {
                channel: ch as u8,
                name: name.clone(),
                volume: Self::cvns_se_volume_for(&self.cvns_volumes, volume),
                looped,
                flag,
                fade_in_ms,
            })?)
        } else {
            None
        };
        call.complete(0x4014 + shift as u16)?;
        if let Some(started) = started {
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(started))
                .expect("slot 0 exists");
        }
        *self.cvns_se_slot_mut(ch) = Some(SeChannel {
            name,
            looped,
            volume,
            flag,
        });
        self.cvns.gains[ch] = 100;
        self.cvns.pans[ch] = 0;
        Ok(())
    }

    fn try_cvns_se_start(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let restart = call.command == 0x4ae;
        let op = if restart { 0x20ae } else { 0x20b1 };
        let ch = Self::check_cvns_se_channel(call, op)?;
        let looped = call.argument(8, "loop")? != 0;
        if self.settings.se_enabled {
            if restart {
                backend.restart_se(ch as u8, looped)?;
            } else {
                backend.start(ch as u8, looped)?;
            }
        }
        call.complete(0x4008)?;
        let slot = self
            .cvns_se_slot_mut(ch)
            .get_or_insert_with(SeChannel::default);
        slot.looped = looped;
        slot.flag = true;
        Ok(())
    }

    fn try_cvns_se_pause(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20b2)?;
        backend.pause_se(ch as u8)?;
        call.complete(0x4004)?;
        self.cvns_se_slot_mut(ch)
            .get_or_insert_with(SeChannel::default)
            .flag = false;
        Ok(())
    }

    fn try_cvns_se_stop(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20b3)?;
        backend.stop(ch as u8)?;
        call.complete(0x4004)?;
        let slot = self
            .cvns_se_slot_mut(ch)
            .get_or_insert_with(SeChannel::default);
        slot.name.clear();
        slot.flag = false;
        Ok(())
    }

    fn try_cvns_se_stop_all(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        for ch in 0..CVNS_SE_CHANNELS {
            backend.stop(ch as u8)?;
            self.cvns_se_slot_mut(ch)
                .get_or_insert_with(SeChannel::default)
                .name
                .clear();
        }
        call.complete(0x4000)?;
        Ok(())
    }

    fn try_cvns_se_fade_out(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20b5)?;
        let fade_ms = call.argument(8, "fade milliseconds")?;
        backend.fade_out_se(ch as u8, fade_ms)?;
        call.complete(0x4008)?;
        self.cvns_se_slot_mut(ch)
            .get_or_insert_with(SeChannel::default)
            .name
            .clear();
        Ok(())
    }

    fn try_cvns_se_position(
        &mut self,
        call: &mut Call<'_>,
        backend: &dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20b6)?;
        let position = backend.se_position(ch as u8)?;
        call.complete(0x4004)?;
        call.vm
            .state_mut()
            .set_system_integer(0, position.unwrap_or(u32::MAX))
            .expect("slot 0 exists");
        Ok(())
    }

    fn try_cvns_se_gain(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20b7)?;
        let multiplier = call.argument(8, "multiplier")?;
        backend.set_se_gain(ch as u8, (u64::from(multiplier) * 256 / 100) as u32)?;
        call.complete(0x4008)?;
        self.cvns.gains[ch] = multiplier;
        Ok(())
    }

    fn try_cvns_se_pan(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_se_channel(call, 0x20af)?;
        let pan = call.argument(8, "pan")? as i32;
        backend.set_se_pan(ch as u8, pan)?;
        call.complete(0x4008)?;
        self.cvns.pans[ch] = pan;
        Ok(())
    }

    fn try_cvns_voice_play(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_voice_channel(call, 0x20b8)?;
        let name_ref = call.argument(8, "name")?;
        let volume = call.argument(12, "volume")? as i32;
        let _looped = call.argument(16, "loop")? != 0;
        let start = call.argument(20, "start")? != 0;
        let name = call.vm.resolve_string(name_ref)?;
        let length = if self.settings.voice_enabled {
            if name.len() >= 64 {
                return Err(error(
                    call.pc,
                    0x20b8,
                    "voice file name exceeds the 64-byte device limit",
                )
                .into());
            }
            Some(backend.play_voice_with_len(&AudioRequest::PlayVoice {
                channel: ch as u8,
                name,
                volume: Self::cvns_voice_volume_for(&self.cvns_volumes, volume),
                start,
                rate_bits: 1.0f32.to_bits(),
                mixer: self.mixer,
            })?)
        } else {
            None
        };
        call.complete(0x4014)?;
        let state = call.vm.state_mut();
        match length {
            Some(Some(bytes)) => {
                state.set_system_integer(0, 1).expect("slot 0 exists");
                state.set_system_integer(4, bytes).expect("slot 4 exists");
            }
            Some(None) => {
                state.set_system_integer(0, 0).expect("slot 0 exists");
                // Common CVNS script wrappers unconditionally copy slot 4 after 0x20b8.
                state.set_system_integer(4, 0).expect("slot 4 exists");
            }
            None => {
                state.set_system_integer(0, 0).expect("slot 0 exists");
                if state.system_integer(4).is_none() {
                    state.set_system_integer(4, 0).expect("slot 4 exists");
                }
            }
        }
        Ok(())
    }

    fn try_cvns_voice_start(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_voice_channel(call, 0x20b9)?;
        let looped = call.argument(8, "loop")? != 0;
        backend.start_voice(ch as u8, looped)?;
        call.complete(0x4008)?;
        Ok(())
    }

    fn try_cvns_voice_position(
        &mut self,
        call: &mut Call<'_>,
        backend: &dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_voice_channel(call, 0x20bc)?;
        let position = backend.voice_position(ch as u8)?;
        call.complete(0x4004)?;
        call.vm
            .state_mut()
            .set_system_integer(0, position.unwrap_or(u32::MAX))
            .expect("slot 0 exists");
        Ok(())
    }

    fn try_cvns_voice_gain(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_voice_channel(call, 0x20bd)?;
        let multiplier = call.argument(8, "multiplier")?;
        backend.set_voice_gain(ch as u8, (u64::from(multiplier) * 256 / 100) as u32)?;
        call.complete(0x4008)?;
        self.cvns.voice_gains[ch] = multiplier;
        Ok(())
    }

    fn try_cvns_voice_pan(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let ch = Self::check_cvns_voice_channel(call, 0x20be)?;
        let pan = call.argument(8, "pan")? as i32;
        backend.set_voice_pan(ch as u8, pan)?;
        call.complete(0x4008)?;
        self.cvns.voice_pans[ch] = pan;
        Ok(())
    }

    fn try_cvns_voice_level(
        &mut self,
        call: &mut Call<'_>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let _ch = Self::check_cvns_voice_channel(call, 0x20bf)?;
        call.complete(0x4004)?;
        let state = call.vm.state_mut();
        state.set_system_integer(4, 0).expect("slot 4 exists");
        state.set_system_integer(5, 0).expect("slot 5 exists");
        Ok(())
    }

    fn try_cvns_aux_se(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let op = call.command + 0x1c00;
        let channel = call.argument(4, "channel")?;
        if channel as usize >= AUX_SE_CHANNELS {
            return Err(error(
                call.pc,
                op,
                "CVNS aux SE channel outside 0..3 sets an unmodeled flag",
            )
            .into());
        }
        let ch = channel as usize;
        match call.command {
            0x4c5 => {
                let name_ref = call.argument(8, "name")?;
                let volume = call.argument(12, "volume")? as i32;
                let fade_in_ms = call.argument(16, "fade milliseconds")?;
                let looped = call.argument(20, "loop")? != 0;
                let start = call.argument(24, "start")? != 0;
                let name = call.vm.resolve_string(name_ref)?;
                if name.len() >= 64 {
                    return Err(error(call.pc, op, "aux SE file name exceeds 64 bytes").into());
                }
                let eff = Self::cvns_percent_to_gain(
                    (volume.max(0) as u32).saturating_mul(self.cvns_volumes[0]) / 100,
                );
                let loaded = backend.play_aux_se(&AudioRequest::PlayAuxSe {
                    channel: ch as u8,
                    name: name.clone(),
                    volume: eff,
                    fade_in_ms,
                    looped,
                    start,
                })?;
                call.complete(0x4018)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(loaded))
                    .expect("slot 0 exists");
                self.cvns.aux_channels[ch] = Some(SeChannel {
                    name,
                    looped,
                    volume,
                    flag: start,
                });
                self.cvns.aux_gains[ch] = 100;
            }
            0x4c6 => {
                let position = backend.aux_se_position(ch as u8)?;
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, position.unwrap_or(u32::MAX))
                    .expect("slot 0 exists");
            }
            0x4ef => {
                let multiplier = call.argument(8, "multiplier")?;
                backend.set_aux_se_gain(ch as u8, (u64::from(multiplier) * 256 / 100) as u32)?;
                call.complete(0x4008)?;
                self.cvns.aux_gains[ch] = multiplier;
            }
            0x4ff => {
                call.complete(0x4004)?;
                let state = call.vm.state_mut();
                state.set_system_integer(4, 0).expect("slot 4 exists");
                state.set_system_integer(5, 0).expect("slot 5 exists");
            }
            _ => {
                backend.stop_aux_se(ch as u8)?;
                call.complete(0x4004)?;
                self.cvns.aux_channels[ch] = None;
            }
        }
        Ok(())
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
    /// channel. 0x20b0/0x20ba store 256 and 0 at +40/+44; the inspected 0x20b8 changes +40.
    /// A stopped channel keeps its old name behind a cleared first byte in the
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
}

pub(crate) struct SavedAudio<'a> {
    bgm: &'a [Vec<u8>; 2],
    channels: [Option<SeChannel>; SE_CHANNELS],
    played: [u32; SE_CHANNELS],
}

impl<'a> SavedAudio<'a> {
    pub(crate) fn decode(bgm: &'a [Vec<u8>; 2], record: &[u8]) -> Result<Self, &'static str> {
        if record.len() != SAVED_CHANNELS {
            return Err("SE channel record has unexpected length");
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
                return Err("saved SE gain multiplier or pan settings are not modeled");
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
            return Err("saved BGM name exceeds the device limit");
        }
        Ok(Self {
            bgm,
            channels,
            played,
        })
    }
}

impl AudioHost {
    /// Loader 0x140081960 then reader 0x140069fd0: stop BGM (0x14008a0a0) and every device
    /// sound (0x1400879e0 / 0x140089060), clear the channel records, then restart a saved
    /// BGM without fade and with looping (0x14008a240), and replay each named SE channel
    /// (0x140088390 on channel + 9, starting at once only when looped and flagged). SE_USE
    /// is not consulted by the loader.
    pub(crate) fn restore(
        &mut self,
        saved: SavedAudio<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let SavedAudio {
            bgm,
            channels,
            played,
        } = saved;
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

    /// Serializes CVNS BGM (+ loop sample), 48 SE channels (+ gain/pan), and 4 aux SE slots (+ gain).
    pub(crate) fn saved_cvns(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let write_bytes = |out: &mut Vec<u8>, bytes: &[u8]| {
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        };
        let write_se = |out: &mut Vec<u8>, slot: Option<&SeChannel>| match slot {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                write_bytes(out, &c.name);
                out.push(u8::from(c.looped));
                out.extend_from_slice(&c.volume.to_le_bytes());
                out.push(u8::from(c.flag));
            }
        };
        write_bytes(&mut out, &self.bgm.primary);
        write_bytes(&mut out, &self.bgm.secondary);
        out.extend_from_slice(&self.cvns.bgm_loop_sample.to_le_bytes());
        for i in 0..CVNS_SE_CHANNELS {
            write_se(&mut out, self.cvns_se_slot(i));
            let gain = self.cvns.gains.get(i).copied().unwrap_or(100);
            let pan = self.cvns.pans.get(i).copied().unwrap_or(0);
            out.extend_from_slice(&gain.to_le_bytes());
            out.extend_from_slice(&pan.to_le_bytes());
        }
        for aux in &self.cvns.aux_channels {
            write_se(&mut out, aux.as_ref());
        }
        for gain in self.cvns.aux_gains {
            out.extend_from_slice(&gain.to_le_bytes());
        }
        out
    }

    pub(crate) fn restore_cvns(
        &mut self,
        saved: SavedCvnsAudio,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.enable_cvns();
        let enabled = self.settings.bgm_enabled;
        if enabled {
            backend.stop_bgm()?;
        }
        for channel in 0..CVNS_SE_CHANNELS as u8 {
            backend.stop(channel)?;
        }
        for channel in 0..VOICE_CHANNELS as u8 {
            backend.stop_voice(channel)?;
        }
        for channel in 0..AUX_SE_CHANNELS as u8 {
            backend.stop_aux_se(channel)?;
        }
        let mut current = if enabled { 0 } else { self.bgm.channel };
        self.bgm = BgmState {
            channel: current,
            ..BgmState::default()
        };
        if !saved.bgm[0].is_empty() {
            let channel = current.wrapping_sub(1) & 1;
            if enabled {
                let req = AudioRequest::PlayBgm {
                    channel,
                    previous: current,
                    primary: saved.bgm[0].clone(),
                    secondary: saved.bgm[1].clone(),
                    volume: Self::cvns_bgm_volume_for(&self.cvns_volumes),
                    fade_ms: 0,
                    looped: true,
                    flag_1576: self.settings.bgm_flag_1576,
                };
                backend.play_bgm_loop_from(&req, saved.bgm_loop_sample)?;
            }
            current = channel;
            let [primary, secondary] = saved.bgm;
            self.bgm = BgmState {
                primary,
                secondary,
                channel: current,
                playing: enabled,
            };
        }
        self.cvns.bgm_loop_sample = saved.bgm_loop_sample;
        for (i, ((channel, gain), pan)) in saved
            .channels
            .iter()
            .zip(&saved.gains)
            .zip(&saved.pans)
            .enumerate()
        {
            if let Some(c) = channel.as_ref().filter(|c| !c.name.is_empty()) {
                let start = c.looped && c.flag;
                backend.play(&AudioRequest::PlaySe {
                    channel: i as u8,
                    name: c.name.clone(),
                    volume: Self::cvns_se_volume_for(&self.cvns_volumes, c.volume),
                    looped: c.looped,
                    flag: start,
                    fade_in_ms: 0,
                })?;
                if *gain < 100 {
                    let mult = (u64::from(*gain) * 256 / 100) as u32;
                    backend.set_se_gain(i as u8, mult)?;
                }
                if *pan != 0 {
                    backend.set_se_pan(i as u8, *pan)?;
                }
            }
            *self.cvns_se_slot_mut(i) = channel.clone();
            self.cvns.gains[i] = *gain;
            self.cvns.pans[i] = *pan;
        }
        for (i, aux) in saved.aux_channels.iter().enumerate() {
            if let Some(c) = aux.as_ref().filter(|c| !c.name.is_empty()) {
                let eff = Self::cvns_percent_to_gain(
                    (c.volume.max(0) as u32).saturating_mul(self.cvns_volumes[0]) / 100,
                );
                backend.play_aux_se(&AudioRequest::PlayAuxSe {
                    channel: i as u8,
                    name: c.name.clone(),
                    volume: eff,
                    fade_in_ms: 0,
                    looped: c.looped,
                    start: c.flag,
                })?;
                let gain = saved.aux_gains[i];
                if gain != 100 {
                    backend.set_aux_se_gain(i as u8, (u64::from(gain) * 256 / 100) as u32)?;
                }
            }
        }
        self.cvns.aux_channels = saved.aux_channels;
        self.cvns.aux_gains = saved.aux_gains;
        self.cvns.voice_gains = [100; VOICE_CHANNELS];
        self.cvns.voice_pans = [0; VOICE_CHANNELS];
        self.played = [0; SE_CHANNELS];
        Ok(())
    }
}

pub(crate) struct SavedCvnsAudio {
    bgm: [Vec<u8>; 2],
    bgm_loop_sample: u32,
    channels: [Option<SeChannel>; CVNS_SE_CHANNELS],
    gains: [u32; CVNS_SE_CHANNELS],
    pans: [i32; CVNS_SE_CHANNELS],
    aux_channels: [Option<SeChannel>; AUX_SE_CHANNELS],
    aux_gains: [u32; AUX_SE_CHANNELS],
}

impl SavedCvnsAudio {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], FormatError> {
            let end = at
                .checked_add(n)
                .filter(|&e| e <= bytes.len())
                .ok_or(FormatError("truncated CVNS audio snapshot"))?;
            let slice = &bytes[*at..end];
            *at = end;
            Ok(slice)
        };
        let read_u32 = |at: &mut usize| -> Result<u32, FormatError> {
            Ok(u32::from_le_bytes(take(at, 4)?.try_into().unwrap()))
        };
        let read_i32 = |at: &mut usize| -> Result<i32, FormatError> {
            Ok(i32::from_le_bytes(take(at, 4)?.try_into().unwrap()))
        };
        let read_bytes = |at: &mut usize, limit: usize| -> Result<Vec<u8>, FormatError> {
            let len = read_u32(at)? as usize;
            if len >= limit {
                return Err(FormatError("CVNS audio name exceeds device limit"));
            }
            let data = take(at, len)?.to_vec();
            if data.contains(&0) {
                return Err(FormatError("CVNS audio name contains NUL"));
            }
            Ok(data)
        };
        let read_se = |at: &mut usize| -> Result<Option<SeChannel>, FormatError> {
            let present = take(at, 1)?[0];
            if present == 0 {
                return Ok(None);
            }
            if present != 1 {
                return Err(FormatError("invalid CVNS SE presence flag"));
            }
            let name = read_bytes(at, 64)?;
            let looped = take(at, 1)?[0] != 0;
            let volume = read_i32(at)?;
            let flag = take(at, 1)?[0] != 0;
            Ok(Some(SeChannel {
                name,
                looped,
                volume,
                flag,
            }))
        };
        let bgm = [read_bytes(&mut at, 64)?, read_bytes(&mut at, 64)?];
        let bgm_loop_sample = read_u32(&mut at)?;
        let mut channels: [Option<SeChannel>; CVNS_SE_CHANNELS] = std::array::from_fn(|_| None);
        let mut gains = [100u32; CVNS_SE_CHANNELS];
        let mut pans = [0i32; CVNS_SE_CHANNELS];
        for i in 0..CVNS_SE_CHANNELS {
            channels[i] = read_se(&mut at)?;
            gains[i] = read_u32(&mut at)?.min(100);
            pans[i] = read_i32(&mut at)?.clamp(-100, 100);
        }
        let mut aux_channels: [Option<SeChannel>; AUX_SE_CHANNELS] = Default::default();
        for slot in &mut aux_channels {
            *slot = read_se(&mut at)?;
        }
        // Older snapshots end after the auxiliary channels and imply full gain.
        let mut aux_gains = [100; AUX_SE_CHANNELS];
        if at != bytes.len() {
            for gain in &mut aux_gains {
                *gain = read_u32(&mut at)?;
            }
        }
        if at != bytes.len() {
            return Err(FormatError("trailing bytes in CVNS audio snapshot"));
        }
        Ok(Self {
            bgm,
            bgm_loop_sample,
            channels,
            gains,
            pans,
            aux_channels,
            aux_gains,
        })
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::machine::Machine;

    #[test]
    fn cvns_snapshot_restores_aux_gains_and_accepts_legacy_records() {
        let mut host = AudioHost::default();
        host.enable_cvns();
        host.cvns.aux_channels = std::array::from_fn(|_| {
            Some(SeChannel {
                name: b"loop".to_vec(),
                volume: 75,
                looped: true,
                flag: true,
            })
        });
        host.cvns.aux_gains = [0, 50, 100, 150];
        let bytes = host.saved_cvns();
        let mut restored = AudioHost::default();
        let mut backend = RecordingAudio::default();
        restored
            .restore_cvns(SavedCvnsAudio::decode(&bytes).unwrap(), &mut backend)
            .unwrap();
        assert_eq!(restored.saved_cvns(), bytes);
        let requests: Vec<_> = backend
            .requests()
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    AudioRequest::PlayAuxSe { .. } | AudioRequest::SetAuxSeGain { .. }
                )
            })
            .cloned()
            .collect();
        let mut expected = Vec::new();
        for (channel, multiplier) in [0, 128, 256, 384].into_iter().enumerate() {
            expected.push(AudioRequest::PlayAuxSe {
                channel: channel as u8,
                name: b"loop".to_vec(),
                volume: 192,
                fade_in_ms: 0,
                looped: true,
                start: true,
            });
            if multiplier != 256 {
                expected.push(AudioRequest::SetAuxSeGain {
                    channel: channel as u8,
                    multiplier,
                });
            }
        }
        assert_eq!(requests, expected);

        let legacy_len = bytes.len() - AUX_SE_CHANNELS * 4;
        restored
            .restore_cvns(
                SavedCvnsAudio::decode(&bytes[..legacy_len]).unwrap(),
                &mut backend,
            )
            .unwrap();
        assert_eq!(restored.cvns.aux_gains, [100; AUX_SE_CHANNELS]);
        for end in legacy_len + 1..bytes.len() {
            assert!(SavedCvnsAudio::decode(&bytes[..end]).is_err());
        }
    }

    #[test]
    fn se_gain_updates_saved_multiplier_only_and_backend_failure_is_retryable() {
        let mut code = vec![];
        for value in [32u32, 1] {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend(0x20b8u16.to_le_bytes());
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(&code);
        let mut raw = b"PS2A".to_vec();
        for n in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            0,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        let script = crate::ps3::Script::decode(&raw).unwrap();
        let mut vm = Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        let mut host = AudioHost::default();
        host.channels[1] = Some(SeChannel {
            name: b"sound".to_vec(),
            looped: true,
            volume: 128,
            flag: true,
        });
        host.played[1] = 256;
        let before = host.saved_channels();
        let mut full = RecordingAudio {
            requests: vec![AudioRequest::StopBgm; 4096],
        };
        assert!(
            host.set_se_gain(&mut Call::new(&mut vm, pc, 0xb8), &mut full)
                .is_err()
        );
        assert_eq!(host.saved_channels(), before);
        assert_eq!(vm.stack().len_bytes(), 8);
        assert_eq!(vm.pending_system(), Some((pc, 0xb8)));
        host.set_se_gain(
            &mut Call::new(&mut vm, pc, 0xb8),
            &mut RecordingAudio::default(),
        )
        .unwrap();
        let mut expected = before;
        expected[52 + 40..52 + 44].copy_from_slice(&32u32.to_le_bytes());
        assert_eq!(host.saved_channels(), expected);
        assert_eq!(vm.stack().len_bytes(), 0);
    }

    #[test]
    fn lifecycle_stops_all_supported_channels_and_reset_clears_se_records() {
        for reset in [false, true] {
            let mut host = AudioHost::default();
            host.bgm.primary = b"bgm".to_vec();
            host.bgm.playing = true;
            host.channels[0] = Some(SeChannel {
                name: b"se".to_vec(),
                ..SeChannel::default()
            });
            host.played[0] = 256;
            let settings = host.settings;
            let mut backend = RecordingAudio::default();
            host.stop_for_lifecycle(&mut backend, reset).unwrap();
            let mut expected = vec![AudioRequest::StopBgm];
            expected.extend((0..6).map(|channel| AudioRequest::StopSe { channel }));
            expected.extend((0..4).map(|channel| AudioRequest::StopVoice { channel }));
            expected.push(AudioRequest::StopChannel15);
            assert_eq!(backend.requests(), expected);
            assert_eq!(host.bgm, BgmState::default());
            assert_eq!(host.settings, settings);
            assert_eq!(host.channels[0].is_none(), reset);
            assert_eq!(host.played[0], if reset { 0 } else { 256 });
        }
    }

    #[test]
    fn failed_lifecycle_stop_keeps_records_for_retry() {
        let mut host = AudioHost::default();
        host.bgm.primary = b"bgm".to_vec();
        host.bgm.playing = true;
        let mut full = RecordingAudio {
            requests: vec![AudioRequest::StopBgm; 4090],
        };
        assert!(host.stop_for_lifecycle(&mut full, true).is_err());
        assert_eq!(host.bgm.primary, b"bgm");
        assert!(host.bgm.playing);
        host.stop_for_lifecycle(&mut RecordingAudio::default(), true)
            .unwrap();
        assert_eq!(host.bgm, BgmState::default());
    }

    #[test]
    fn cvns_48_channel_se_and_100_percent_volume_and_bgm_loop_frames() {
        fn call_cvns(
            host: &mut AudioHost,
            backend: &mut RecordingAudio,
            command: u16,
            args: &[u32],
            strings: &[&[u8]],
        ) -> (Option<u32>, Option<u32>) {
            let mut code = vec![];
            for &value in args.iter().rev() {
                code.extend(0x440u16.to_le_bytes());
                code.extend(4u16.to_le_bytes());
                code.extend(value.to_le_bytes());
            }
            code.extend((command + 0x1c00).to_le_bytes());
            let mut payload = 0u32.to_le_bytes().to_vec();
            payload.extend(&code);
            let mut raw = b"PS2A".to_vec();
            for n in [
                48,
                0,
                0,
                1,
                code.len() as u32,
                0,
                0,
                0,
                0,
                payload.len() as u32,
                0,
            ] {
                raw.extend(n.to_le_bytes());
            }
            raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
            let script = crate::ps3::Script::decode(&raw).unwrap();
            let mut vm = Machine::new(&script).unwrap();
            vm.state_mut().set_string_slots(strings.len() as u32);
            for (index, text) in strings.iter().enumerate() {
                vm.state_mut().set_string(index as u32, text).unwrap();
            }
            while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
            let pc = vm.pc();
            host.handle_cvns(&mut Call::new(&mut vm, pc, command), backend)
                .unwrap();
            assert_eq!(vm.stack().len_bytes(), 0);
            let state = vm.state_mut();
            (state.system_integer(0), state.system_integer(4))
        }

        let mut host = AudioHost::default();
        host.enable_cvns();
        let mut backend = RecordingAudio::default();

        assert_eq!(AudioHost::cvns_percent_to_gain(100), 256);

        call_cvns(
            &mut host,
            &mut backend,
            0x4a7,
            &[0x8000_0000, 0, 1, 12345],
            &[b"bgm01"],
        );
        assert_eq!(
            backend.requests(),
            &[AudioRequest::PlayBgmLoopFrom {
                channel: 1,
                previous: 0,
                primary: b"bgm01".to_vec(),
                secondary: Vec::new(),
                volume: 256,
                fade_ms: 0,
                looped: true,
                flag_1576: 0,
                loop_sample: 12345,
            }]
        );
        assert_eq!(host.cvns.bgm_loop_sample, 12345);

        backend = RecordingAudio::default();
        call_cvns(
            &mut host,
            &mut backend,
            0x4b0,
            &[47, 0x8000_0000, 100, 0, 1],
            &[b"se47"],
        );
        assert_eq!(
            backend.requests(),
            &[AudioRequest::PlaySe {
                channel: 47,
                name: b"se47".to_vec(),
                volume: 256,
                looped: false,
                flag: true,
                fade_in_ms: 0,
            }]
        );
        assert_eq!(
            host.cvns_se_slot(47).map(|s| s.name.as_slice()),
            Some(b"se47".as_slice())
        );

        backend = RecordingAudio::default();
        let (v0, v4) = call_cvns(
            &mut host,
            &mut backend,
            0x4b8,
            &[2, 0x8000_0000, 100, 0, 1],
            &[b"vo01"],
        );
        assert_eq!(v0, Some(0));
        assert_eq!(v4, Some(0));

        backend = RecordingAudio::default();
        let (aux0, _) = call_cvns(
            &mut host,
            &mut backend,
            0x4c5,
            &[3, 0x8000_0000, 100, 25, 1, 1],
            &[b"aux3"],
        );
        assert_eq!(aux0, Some(0));
        call_cvns(&mut host, &mut backend, 0x4ef, &[3, 50], &[]);
        let (_, aux_peak_r) = call_cvns(&mut host, &mut backend, 0x4ff, &[3], &[]);
        assert_eq!(aux_peak_r, Some(0));
        call_cvns(&mut host, &mut backend, 0x4c7, &[3], &[]);
        assert_eq!(
            backend.requests(),
            &[
                AudioRequest::PlayAuxSe {
                    channel: 3,
                    name: b"aux3".to_vec(),
                    volume: 256,
                    fade_in_ms: 25,
                    looped: true,
                    start: true,
                },
                AudioRequest::SetAuxSeGain {
                    channel: 3,
                    multiplier: 128,
                },
                AudioRequest::StopAuxSe { channel: 3 },
            ]
        );

        // Reusing a muted channel must not restore the previous sound's gain.
        call_cvns(
            &mut host,
            &mut backend,
            0x4c5,
            &[3, 0x8000_0000, 100, 0, 1, 1],
            &[b"aux3"],
        );
        call_cvns(&mut host, &mut backend, 0x4ef, &[3, 0], &[]);
        call_cvns(
            &mut host,
            &mut backend,
            0x4c5,
            &[3, 0x8000_0000, 100, 0, 1, 1],
            &[b"replacement"],
        );
        let mut restored_backend = RecordingAudio::default();
        AudioHost::default()
            .restore_cvns(
                SavedCvnsAudio::decode(&host.saved_cvns()).unwrap(),
                &mut restored_backend,
            )
            .unwrap();
        assert_eq!(
            restored_backend.requests().last(),
            backend.requests().last()
        );

        backend = RecordingAudio::default();
        host.stop_for_lifecycle(&mut backend, true).unwrap();
        let mut expected = vec![AudioRequest::StopBgm];
        expected
            .extend((0..CVNS_SE_CHANNELS as u8).map(|channel| AudioRequest::StopSe { channel }));
        expected
            .extend((0..VOICE_CHANNELS as u8).map(|channel| AudioRequest::StopVoice { channel }));
        expected
            .extend((0..AUX_SE_CHANNELS as u8).map(|channel| AudioRequest::StopAuxSe { channel }));
        assert_eq!(backend.requests(), expected);
        assert!(host.cvns_se_slot(47).is_none());
    }
}

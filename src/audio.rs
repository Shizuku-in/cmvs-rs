//! Sound-effect channel commands with an explicit audio backend.
//! The recording backend keeps requests only; no sound is loaded or played.
use crate::{evaluate::error, machine::Machine, vm::SystemResult};
use std::error::Error;

/// Six SE channels at +1656, 52 bytes each (snapshot record 0x310); device channels 9..14.
pub const SE_CHANNELS: usize = 6;
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
}
impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            se_enabled: true,
            master: 256,
            se: 256,
        }
    }
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
    },
    /// 0x140088030: start a prepared device channel; an already active channel is unchanged.
    StartSe { channel: u8, looped: bool },
    /// 0x140087910: stop and release the device channel's sound, prepared or active.
    StopSe { channel: u8 },
}

pub trait AudioBackend {
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
}

#[derive(Debug, Default)]
pub struct AudioHost {
    settings: AudioSettings,
    channels: [Option<SeChannel>; SE_CHANNELS],
}

impl AudioHost {
    pub fn new(settings: AudioSettings) -> Self {
        Self {
            settings,
            channels: Default::default(),
        }
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
            Some((_, 0xb1)) => return self.try_start(vm, backend),
            Some((_, 0xb3)) => return self.try_stop(vm, backend),
            _ => {}
        }
        let Some((pc, 0xb0)) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x20b0, e.0))
        };
        let channel = arg(4)?;
        let (name, volume, looped, flag) = (arg(8)?, arg(12)? as i32, arg(16)? != 0, arg(20)? != 0);
        if channel as usize >= SE_CHANNELS {
            // The original only sets +12056 bit 0x200, which is not modeled.
            return Err(error(pc, 0x20b0, "SE channel outside 0..5 sets an unmodeled flag").into());
        }
        let name = vm.resolve_string(name)?;
        if name.len() > NAME_CAPACITY {
            return Err(error(pc, 0x20b0, "SE file name exceeds the 32-byte record").into());
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
            })?)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4014))?;
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
}

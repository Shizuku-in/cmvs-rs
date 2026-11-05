//! Stand-ins for the libnx services off Horizon, so the complete host type-checks
//! and its portable parts test on development machines. Nothing here runs a game.
use cmvs_native::SwitchInput;

const UNAVAILABLE: &str = "Switch host requires Horizon";

pub struct SwitchWindow;
impl SwitchWindow {
    pub fn open() -> Result<Self, &'static str> {
        Err(UNAVAILABLE)
    }
    pub fn poll(&mut self) -> SwitchInput {
        SwitchInput {
            exit: true,
            ..SwitchInput::default()
        }
    }
    pub fn present(&mut self, _: &[u32]) -> Result<(), &'static str> {
        Err(UNAVAILABLE)
    }
}

pub struct SwitchAudioOut;
impl SwitchAudioOut {
    pub fn open() -> Result<Self, &'static str> {
        Err(UNAVAILABLE)
    }
    pub fn sample_rate(&self) -> u32 {
        48_000
    }
    pub fn channels(&self) -> u16 {
        2
    }
    pub fn block_len(&self) -> usize {
        2048
    }
    pub fn prefer_audio_core(&self) -> bool {
        false
    }
    pub fn submit(&mut self, _: &[i16]) -> Result<(), &'static str> {
        Err(UNAVAILABLE)
    }
}

pub fn shared_font() -> Option<Vec<u8>> {
    None
}

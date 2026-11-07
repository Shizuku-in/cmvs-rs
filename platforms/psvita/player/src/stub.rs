//! Stand-ins for the Vita system services off the device, so the complete host
//! type-checks and its portable parts test on development machines. Nothing here
//! runs a game.
use cmvs_native::PadInput;

const UNAVAILABLE: &str = "Vita host requires the PlayStation Vita";

pub struct VitaWindow;
impl VitaWindow {
    pub fn open() -> Result<Self, &'static str> {
        Err(UNAVAILABLE)
    }
    pub fn poll(&mut self) -> PadInput {
        PadInput::default()
    }
    pub fn present(
        &mut self,
        _: &[u8],
        _: [u32; 2],
        _: [f32; 4],
        _: bool,
        _: Option<[f32; 2]>,
    ) -> Result<(), &'static str> {
        Err(UNAVAILABLE)
    }
}

pub struct VitaAudioOut;
impl VitaAudioOut {
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
    pub fn submit(&mut self, _: &[i16]) -> Result<(), &'static str> {
        Err(UNAVAILABLE)
    }
}

pub fn confirm_is_cross() -> bool {
    true
}

pub fn prefer_core(_: u32) -> bool {
    false
}

pub fn set_max_clocks() -> bool {
    false
}

pub fn show_message(_: &str) -> bool {
    false
}

pub fn ime_dialog(
    _: &str,
    _: &str,
    _: &str,
    _: u32,
    _: impl FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, &'static str> {
    Err(UNAVAILABLE)
}

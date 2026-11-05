//! Stand-ins for the libnx services off Horizon, so the complete host type-checks
//! and its portable parts test on development machines. Nothing here runs a game.
use cmvs_native::{GpuError, GpuTexture, SwitchInput};

const UNAVAILABLE: &str = "Switch host requires Horizon";

pub struct SwitchWindow;
impl SwitchWindow {
    pub fn open() -> Result<Self, &'static str> {
        Err(UNAVAILABLE)
    }
    pub fn open_gpu() -> Result<(Self, SwitchGpu), String> {
        Err(UNAVAILABLE.into())
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

pub struct SwitchGpu;
fn gpu_unavailable<T>() -> Result<T, GpuError> {
    Err(GpuError::Device(UNAVAILABLE.into()))
}
impl SwitchGpu {
    pub fn begin_frame(&mut self) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn create_texture(&mut self, _: u32, _: u32, _: &[u8]) -> Result<GpuTexture, GpuError> {
        gpu_unavailable()
    }
    pub fn release_texture(&mut self, _: GpuTexture) {}
    pub fn begin_canvas(&mut self, _: [u32; 2], _: [u8; 4]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn draw(&mut self, _: u32, _: &[f32], _: [GpuTexture; 3]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn scale(&mut self, _: GpuTexture, _: [f32; 8]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn present(&mut self, _: [f32; 8], _: &[[f32; 24]]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn read(&mut self, _: [u32; 2], _: &mut [u8]) -> Result<(), GpuError> {
        gpu_unavailable()
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

pub fn set_cpu_boost(_: bool) -> bool {
    false
}

pub fn show_system_error(_: &str, _: &str) -> bool {
    false
}

pub fn shared_font() -> Option<Vec<u8>> {
    None
}

pub fn software_keyboard(
    _: &str,
    _: &str,
    _: &str,
    _: u32,
    _: impl FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, &'static str> {
    Err(UNAVAILABLE)
}

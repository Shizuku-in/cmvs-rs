//! Stand-ins for the Vita system services off the device, so the complete host
//! type-checks and its portable parts test on development machines. Nothing here
//! runs a game.
use cmvs_native::{GpuDevice, GpuError, GpuTexture, PadInput};

const UNAVAILABLE: &str = "Vita host requires the PlayStation Vita";

pub struct VitaWindow;
impl VitaWindow {
    pub fn open() -> Result<Self, &'static str> {
        Err(UNAVAILABLE)
    }
    pub fn open_gpu(&mut self) -> Result<VitaGpu, String> {
        Err(UNAVAILABLE.into())
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

pub struct VitaGpu;
fn gpu_unavailable<T>() -> Result<T, GpuError> {
    Err(GpuError::Device(UNAVAILABLE.into()))
}
impl GpuDevice for VitaGpu {
    fn begin_frame(&mut self) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    fn create_texture(&mut self, _: u32, _: u32, _: &[u8]) -> Result<GpuTexture, GpuError> {
        gpu_unavailable()
    }
    fn release_texture(&mut self, _: GpuTexture) {}
    fn begin_canvas(&mut self, _: [u32; 2], _: [u8; 4]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    fn draw(&mut self, _: u32, _: &[f32], _: [GpuTexture; 3]) -> Result<(), GpuError> {
        gpu_unavailable()
    }
}
impl VitaGpu {
    pub fn present(&mut self, _: [f32; 4], _: bool, _: Option<[f32; 2]>) -> Result<(), GpuError> {
        gpu_unavailable()
    }
    pub fn read(&mut self, _: [u32; 2], _: &mut [u8]) -> Result<(), GpuError> {
        gpu_unavailable()
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

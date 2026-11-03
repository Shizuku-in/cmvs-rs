use cmvs_core::{
    platform::{FontDialogResult, PlatformBackend, PlatformRequest},
    raster::RgbaImage,
};
use std::error::Error;

pub struct BrowserBackend {
    pub size: [u32; 2],
    pub frame: Option<RgbaImage>,
    #[cfg(target_arch = "wasm32")]
    pub gpu: Option<crate::gpu::BrowserGpu>,
    #[cfg(target_arch = "wasm32")]
    pub read_requested: bool,
    fullscreen: bool,
    aspect: u32,
    scaling: bool,
    render: u32,
}
impl BrowserBackend {
    pub fn movie_canvas(&self, image: &RgbaImage) -> Result<RgbaImage, cmvs_core::FormatError> {
        if [image.width(), image.height()] == self.size {
            return Ok(image.clone());
        }
        let [width, height] = self.size;
        let mut target = vec![0; (width * height) as usize];
        image.draw_scaled(&mut target, width, height, 1, self.render == 1)?;
        let pixels = target
            .into_iter()
            .flat_map(|word| {
                let [b, g, r, _] = word.to_le_bytes();
                [r, g, b, 255]
            })
            .collect();
        RgbaImage::new(width, height, pixels)
    }
    pub fn preferences(&self) -> cmvs_core::settings::DisplayPreferences {
        cmvs_core::settings::DisplayPreferences {
            fullscreen: self.fullscreen,
            aspect_mode: self.aspect,
            aspect_scaling: self.scaling,
            render_mode: self.render,
        }
    }
    pub fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            frame: None,
            #[cfg(target_arch = "wasm32")]
            gpu: None,
            #[cfg(target_arch = "wasm32")]
            read_requested: false,
            fullscreen: false,
            aspect: 1,
            scaling: true,
            render: 0,
        }
    }
}
impl PlatformBackend for BrowserBackend {
    fn select_fullscreen_monitor(&mut self, mode: u32) -> Result<bool, Box<dyn Error>> {
        Ok(mode == 5)
    }
    fn display_count(&self) -> Result<u32, Box<dyn Error>> {
        Ok(1)
    }
    fn logical_size(&self) -> [u32; 2] {
        self.size
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        #[cfg(target_arch = "wasm32")]
        if self.frame.is_none() && self.gpu.as_ref().is_some_and(|gpu| gpu.frame.is_some()) {
            self.read_requested = true;
            return Err("browser GPU readback pending".into());
        }
        self.frame
            .clone()
            .ok_or_else(|| "no completed browser frame".into())
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.size)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.size)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.fullscreen)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
        Ok(0)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.render)
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error>> {
        Ok(false)
    }
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.aspect)
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.scaling)
    }
    fn poll_font_dialog(&mut self, _: &[u8]) -> Result<FontDialogResult, Box<dyn Error>> {
        Ok(FontDialogResult::Cancelled)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        match request {
            PlatformRequest::Fullscreen(value) => self.fullscreen = *value,
            PlatformRequest::FullscreenMonitor(_) => {}
            PlatformRequest::AspectMode(value) if *value <= 3 => self.aspect = *value,
            PlatformRequest::AspectScaling(value) => self.scaling = *value,
            PlatformRequest::RenderMode(value) if *value <= 1 => self.render = *value,
            PlatformRequest::WindowSize(size) if *size == self.size => {}
            PlatformRequest::Title(_)
            | PlatformRequest::Cursor { .. }
            | PlatformRequest::Icon { .. } => {}
            _ => return Err("unsupported browser display request".into()),
        }
        Ok(())
    }
}

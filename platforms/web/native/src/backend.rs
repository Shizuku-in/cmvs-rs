use cmvs_core::{
    TextEncoding,
    host::platform::DeviceFrameResult,
    host::platform::FontDialogResult,
    host::platform::PlatformBackend,
    host::platform::PlatformRequest,
    host::platform::{TextDialogRequest, TextDialogResult},
    render::raster::RgbaImage,
};
use cmvs_host::CommentInput;
use std::error::Error;

pub struct BrowserBackend {
    pub size: [u32; 2],
    pub frame: Option<RgbaImage>,
    pub comment_input: CommentInput,
    encoding: TextEncoding,
    #[cfg(target_arch = "wasm32")]
    pub gpu: Option<crate::gpu::BrowserGpu>,
    fullscreen: bool,
    aspect: u32,
    scaling: bool,
    render: u32,
}
impl BrowserBackend {
    pub fn preferences(&self) -> cmvs_core::host::settings::DisplayPreferences {
        cmvs_core::host::settings::DisplayPreferences {
            fullscreen: self.fullscreen,
            aspect_mode: self.aspect,
            aspect_scaling: self.scaling,
            render_mode: self.render,
        }
    }
    fn new(size: [u32; 2], encoding: TextEncoding) -> Self {
        Self {
            size,
            frame: None,
            comment_input: CommentInput::default(),
            encoding,
            #[cfg(target_arch = "wasm32")]
            gpu: None,
            fullscreen: false,
            aspect: 1,
            scaling: true,
            render: 0,
        }
    }
    pub fn with_preferences(
        size: [u32; 2],
        preferences: cmvs_core::host::settings::DisplayPreferences,
        encoding: TextEncoding,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let mut backend = Self::new(size, encoding);
        backend.fullscreen = preferences.fullscreen;
        backend.aspect = preferences.aspect_mode;
        backend.scaling = preferences.aspect_scaling;
        backend.render =
            cmvs_core::host::platform::supported_render_mode(&backend, preferences.render_mode)?;
        Ok(backend)
    }
}
impl PlatformBackend for BrowserBackend {
    fn select_fullscreen_monitor(
        &mut self,
        mode: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(mode == 5)
    }
    fn display_count(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(1)
    }
    fn logical_size(&self) -> [u32; 2] {
        self.size
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        self.frame
            .clone()
            .ok_or_else(|| "no completed browser frame".into())
    }
    fn poll_device_frame(&mut self) -> Result<DeviceFrameResult, Box<dyn Error + Send + Sync>> {
        #[cfg(target_arch = "wasm32")]
        if self.frame.is_none() && self.gpu.as_ref().is_some_and(|gpu| gpu.frame.is_some()) {
            return Ok(DeviceFrameResult::Pending);
        }
        self.read_device_frame().map(DeviceFrameResult::Ready)
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Ok(self.size)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Ok(self.size)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.fullscreen)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error + Send + Sync>> {
        Ok(0)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(self.render)
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(false)
    }
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(self.aspect)
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.scaling)
    }
    fn poll_text_dialog(
        &mut self,
        request: &TextDialogRequest,
    ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
        self.comment_input.poll(request, self.encoding)
    }
    fn cancel_text_dialog(&mut self) {
        self.comment_input.cancel();
    }
    fn poll_font_dialog(
        &mut self,
        _: &[u8],
    ) -> Result<FontDialogResult, Box<dyn Error + Send + Sync>> {
        Ok(FontDialogResult::Cancelled)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
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

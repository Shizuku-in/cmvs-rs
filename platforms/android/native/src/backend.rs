use android_activity::AndroidApp;
use cmvs_core::{
    platform::{FontDialogResult, PlatformBackend, PlatformRequest, resolve_under},
    raster::RgbaImage,
    settings::DisplayPreferences,
};
use cmvs_font::font::FontRenderer;
use cmvs_wgpu::GpuFrame;
use std::{cell::RefCell, error::Error, fs::File, io::Read, path::PathBuf, rc::Rc, sync::Arc};

pub(super) struct MobileBackend {
    pub display: DisplayPreferences,
    screen_size: [u32; 2],
    pub frame: Option<Arc<GpuFrame>>,
    pub font: Rc<RefCell<FontRenderer>>,
    pub font_pending: bool,
    app: AndroidApp,
    directory: PathBuf,
    title: String,
}

impl MobileBackend {
    pub fn new(
        app: AndroidApp,
        directory: PathBuf,
        font: Rc<RefCell<FontRenderer>>,
        display: DisplayPreferences,
        screen_size: [u32; 2],
    ) -> Self {
        Self {
            app,
            directory,
            font,
            display,
            screen_size,
            frame: None,
            font_pending: false,
            title: String::new(),
        }
    }
    pub fn mode(&self) -> u32 {
        cmvs_core::viewport::presentation_mode(
            self.display.fullscreen,
            self.display.aspect_scaling,
            self.display.aspect_mode,
        )
    }
}

impl PlatformBackend for MobileBackend {
    fn display_count(&self) -> Result<u32, Box<dyn Error>> {
        Ok(1)
    }
    fn logical_size(&self) -> [u32; 2] {
        self.screen_size
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        self.frame.as_ref().ok_or("no completed game frame")?.read()
    }
    // Android owns the physical window; scripts operate on a virtual game window.
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.screen_size)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.screen_size)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.display.fullscreen)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
        Ok(0)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.display.render_mode)
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
        Ok(self.display.aspect_mode)
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.display.aspect_scaling)
    }
    fn poll_font_dialog(&mut self, _current: &[u8]) -> Result<FontDialogResult, Box<dyn Error>> {
        let response = self.directory.join("font-result.cfg");
        if !self.font_pending {
            match std::fs::remove_file(&response) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            self.font_pending = true;
            cmvs_native::show_font_picker(&self.app);
            return Ok(FontDialogResult::Pending);
        }
        let file = match File::open(&response) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FontDialogResult::Pending);
            }
            Err(e) => return Err(e.into()),
        };
        let mut text = String::new();
        file.take(4097).read_to_string(&mut text)?;
        std::fs::remove_file(response)?;
        self.font_pending = false;
        if text.trim() == "cancel" {
            return Ok(FontDialogResult::Cancelled);
        }
        if text.len() > 4096 {
            return Err("font selection is too large".into());
        }
        let mut lines = text.lines();
        let path = resolve_under(
            &self.directory,
            lines.next().ok_or("missing font selection")?.as_bytes(),
        )?;
        let index = lines
            .next()
            .ok_or("missing font collection index")?
            .parse()?;
        match self.font.borrow().import_font(&path, index) {
            Ok(face) => Ok(FontDialogResult::Selected(face)),
            Err(error) => {
                log::warn!("Font selection rejected: {error}");
                Ok(FontDialogResult::Cancelled)
            }
        }
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        match request {
            PlatformRequest::Fullscreen(value) => self.display.fullscreen = *value,
            PlatformRequest::AspectMode(mode) if *mode <= 3 => self.display.aspect_mode = *mode,
            PlatformRequest::AspectScaling(value) => self.display.aspect_scaling = *value,
            PlatformRequest::RenderMode(mode) if *mode <= 1 => self.display.render_mode = *mode,
            PlatformRequest::WindowSize(size) if *size == self.screen_size => {}
            PlatformRequest::Title(bytes) => {
                self.title = encoding_rs::SHIFT_JIS
                    .decode_without_bom_handling_and_without_replacement(bytes)
                    .ok_or("invalid Shift-JIS game title")?
                    .into_owned();
                log::info!("Game title: {}", self.title);
            }
            // Native cursors and desktop window icons have no visible mobile target.
            PlatformRequest::Cursor { .. } | PlatformRequest::Icon { .. } => {}
            _ => return Err("unsupported mobile display request".into()),
        }
        Ok(())
    }
}

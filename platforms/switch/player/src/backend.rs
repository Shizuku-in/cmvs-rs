use cmvs_core::{
    platform::{PlatformBackend, PlatformRequest, TextDialogRequest, TextDialogResult},
    raster::RgbaImage,
    settings::DisplayPreferences,
};
use std::error::Error;
pub struct Backend<'a> {
    pub display: DisplayPreferences,
    pub size: [u32; 2],
    /// Last software frame; unused when `gpu` renders.
    pub frame: Option<RgbaImage>,
    pub gpu: Option<&'a mut crate::gpu::GpuRenderer>,
    /// A modal system applet ran since the host last cleared this.
    pub modal_shown: bool,
}
impl PlatformBackend for Backend<'_> {
    fn logical_size(&self) -> [u32; 2] {
        self.size
    }
    fn display_count(&self) -> Result<u32, Box<dyn Error>> {
        Ok(1)
    }
    fn select_fullscreen_monitor(&mut self, mode: u32) -> Result<bool, Box<dyn Error>> {
        Ok(mode == 5)
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.size)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.size)
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
    fn poll_text_dialog(
        &mut self,
        request: &TextDialogRequest,
    ) -> Result<TextDialogResult, Box<dyn Error>> {
        self.modal_shown = true;
        crate::text_input::edit(request, &mut |title, prompt, initial, limit, check| {
            crate::sys::software_keyboard(title, prompt, initial, limit, check)
        })
    }
    fn choose_font(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(None)
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        if let Some(gpu) = &mut self.gpu {
            return gpu.read();
        }
        self.frame
            .clone()
            .ok_or_else(|| "no completed frame".into())
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        match request {
            PlatformRequest::Fullscreen(v) => self.display.fullscreen = *v,
            PlatformRequest::AspectMode(v) if *v <= 3 => self.display.aspect_mode = *v,
            PlatformRequest::AspectScaling(v) => self.display.aspect_scaling = *v,
            PlatformRequest::RenderMode(v) if *v <= 1 => self.display.render_mode = *v,
            PlatformRequest::WindowSize(v) if *v == self.size => {}
            PlatformRequest::Title(_)
            | PlatformRequest::Cursor { .. }
            | PlatformRequest::Icon { .. } => {}
            _ => return Err("unsupported Switch display request".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_request_keeps_preferences_and_no_frame_is_an_error() {
        let mut backend = Backend {
            display: DisplayPreferences::default(),
            size: [1280, 720],
            frame: None,
            gpu: None,
            modal_shown: false,
        };
        let previous = backend.display.render_mode;
        assert!(backend.apply(&PlatformRequest::RenderMode(9)).is_err());
        assert_eq!(backend.display.render_mode, previous);
        assert!(backend.read_device_frame().is_err());
        let frame = RgbaImage::new(1, 1, vec![10, 20, 30, 255]).unwrap();
        backend.frame = Some(frame);
        assert_eq!(
            backend.read_device_frame().unwrap().pixels(),
            &[10, 20, 30, 255]
        );
        assert!(backend.frame.is_some());
    }
}

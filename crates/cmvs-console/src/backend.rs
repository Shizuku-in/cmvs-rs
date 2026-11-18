//! Shared `PlatformBackend` implementation for the console players.
use crate::text_input;
use cmvs_core::{
    host::platform::MessageDialogRequest, host::platform::MessageDialogResult,
    host::platform::PlatformBackend, host::platform::PlatformRequest,
    host::platform::TextDialogRequest, host::platform::TextDialogResult,
    host::settings::DisplayPreferences, render::raster::RgbaImage, render::raster::movie_canvas,
};
use std::error::Error;

/// Presents GPU-rendered canvases that can optionally be read back for engine screenshots.
pub trait ConsoleGpu {
    fn read_presented(&mut self) -> Option<Result<RgbaImage, Box<dyn Error + Send + Sync>>>;
}

/// Callback displaying the platform's modal text input dialog.
pub type TextDialogFn = fn(
    &str,
    &str,
    &str,
    u32,
    &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, &'static str>;

/// The last software-presented frame, with decoded movie pixels retained for readback.
pub enum ConsoleFrame {
    Canvas(RgbaImage),
    Movie { image: RgbaImage, linear: bool },
}

impl ConsoleFrame {
    pub fn image(&self) -> &RgbaImage {
        match self {
            Self::Canvas(image) | Self::Movie { image, .. } => image,
        }
    }

    pub fn readback(&self, size: [u32; 2]) -> Result<RgbaImage, cmvs_core::FormatError> {
        match self {
            Self::Canvas(image) => Ok(image.clone()),
            Self::Movie { image, linear } => movie_canvas(image, size, *linear),
        }
    }
}

/// Shared `PlatformBackend` adapter parameterized by console GPU readback and keyboard dialog.
pub struct ConsoleBackend<'a, G> {
    pub display: DisplayPreferences,
    pub size: [u32; 2],
    /// Last completed software frame when software compositing owns the screen.
    pub frame: Option<ConsoleFrame>,
    pub gpu: Option<&'a mut G>,
    /// A modal system keyboard/dialog ran since the host last cleared this.
    pub modal_shown: bool,
    /// The game text encoding of the comment dialog's strings.
    pub encoding: cmvs_core::TextEncoding,
    pub platform_name: &'static str,
    pub text_dialog: TextDialogFn,
}

impl<G: ConsoleGpu> PlatformBackend for ConsoleBackend<'_, G> {
    fn logical_size(&self) -> [u32; 2] {
        self.size
    }
    fn display_count(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(1)
    }
    fn select_fullscreen_monitor(
        &mut self,
        mode: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(mode == 5)
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Ok(self.size)
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Ok(self.size)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.display.fullscreen)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error + Send + Sync>> {
        Ok(0)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(self.display.render_mode)
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
        Ok(self.display.aspect_mode)
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.display.aspect_scaling)
    }
    fn poll_message_dialog(
        &mut self,
        _: &MessageDialogRequest,
    ) -> Result<MessageDialogResult, Box<dyn Error + Send + Sync>> {
        Ok(MessageDialogResult::Accepted)
    }
    fn poll_text_dialog(
        &mut self,
        request: &TextDialogRequest,
    ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
        self.modal_shown = true;
        let dialog = self.text_dialog;
        text_input::edit(request, self.encoding, &mut |t, p, i, l, c| {
            dialog(t, p, i, l, c)
        })
    }
    fn choose_font(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        if let Some(frame) = self.gpu.as_deref_mut().and_then(ConsoleGpu::read_presented) {
            return frame;
        }
        let frame = self.frame.as_ref().ok_or("no completed frame")?;
        Ok(frame.readback(self.size)?)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        match request {
            PlatformRequest::Fullscreen(v) => self.display.fullscreen = *v,
            PlatformRequest::AspectMode(v) if *v <= 3 => self.display.aspect_mode = *v,
            PlatformRequest::AspectScaling(v) => self.display.aspect_scaling = *v,
            PlatformRequest::RenderMode(v) if *v <= 1 => self.display.render_mode = *v,
            PlatformRequest::WindowSize(v) if *v == self.size => {}
            PlatformRequest::Title(_)
            | PlatformRequest::Cursor { .. }
            | PlatformRequest::Icon { .. } => {}
            _ => return Err(format!("unsupported {} display request", self.platform_name).into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeGpu {
        presented: Option<Result<RgbaImage, &'static str>>,
    }

    impl ConsoleGpu for FakeGpu {
        fn read_presented(&mut self) -> Option<Result<RgbaImage, Box<dyn Error + Send + Sync>>> {
            self.presented
                .take()
                .map(|result| result.map_err(Into::into))
        }
    }

    fn no_dialog(
        _: &str,
        _: &str,
        _: &str,
        _: u32,
        _: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<Option<String>, &'static str> {
        Ok(None)
    }

    #[test]
    fn decoded_movie_readback_uses_the_logical_canvas() {
        let image = RgbaImage::new(1, 1, vec![10, 20, 30, 255]).unwrap();
        let frame = ConsoleFrame::Movie {
            image,
            linear: false,
        };
        let canvas = frame.readback([3, 1]).unwrap();
        assert_eq!([canvas.width(), canvas.height()], [3, 1]);
        assert_eq!(
            canvas.pixels(),
            &[0, 0, 0, 255, 10, 20, 30, 255, 0, 0, 0, 255]
        );
        assert_eq!(frame.image().width(), 1);
    }

    #[test]
    fn rejected_request_keeps_preferences_and_readback_prefers_presented_gpu_frame() {
        let mut backend: ConsoleBackend<'_, FakeGpu> = ConsoleBackend {
            display: DisplayPreferences::default(),
            size: [1280, 720],
            frame: None,
            gpu: None,
            modal_shown: false,
            encoding: cmvs_core::TextEncoding::ShiftJis,
            platform_name: "Test",
            text_dialog: no_dialog,
        };
        let previous = backend.display.render_mode;
        assert!(
            backend
                .apply(&PlatformRequest::RenderMode(9))
                .unwrap_err()
                .to_string()
                .contains("Test")
        );
        assert_eq!(backend.display.render_mode, previous);
        assert!(backend.read_device_frame().is_err());

        let sw = RgbaImage::new(1, 1, vec![10, 20, 30, 255]).unwrap();
        backend.frame = Some(ConsoleFrame::Canvas(sw));
        assert_eq!(
            backend.read_device_frame().unwrap().pixels(),
            &[10, 20, 30, 255]
        );
        assert!(backend.frame.is_some());

        let gpu_frame = RgbaImage::new(1, 1, vec![40, 50, 60, 255]).unwrap();
        let mut fake_gpu = FakeGpu {
            presented: Some(Ok(gpu_frame)),
        };
        backend.gpu = Some(&mut fake_gpu);
        // Presented GPU frame wins over retained software frame.
        assert_eq!(
            backend.read_device_frame().unwrap().pixels(),
            &[40, 50, 60, 255]
        );
        // When the GPU has no presented frame, fall back to the software frame.
        assert_eq!(
            backend.read_device_frame().unwrap().pixels(),
            &[10, 20, 30, 255]
        );
    }
}

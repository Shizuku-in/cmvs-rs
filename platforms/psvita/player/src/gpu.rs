//! GXM execution of the shared scene queue through [`cmvs_console::gpu::Compositor`].
//! The window shows the canvas as it shows software frames; movie frames and host
//! pages keep the software path. Readback happens only when the engine requests a
//! device frame.
use crate::sys::VitaGpu;
use cmvs_console::gpu::Compositor;
use cmvs_core::{raster::RgbaImage, scene_render::RenderFrame};
use std::error::Error;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// Retained texture bytes; video memory (128 MiB) also holds the display buffers, and
/// uploads fall back to main memory once it is full.
const CACHE_LIMIT: usize = 64 * 1024 * 1024;

pub struct GpuRenderer(Compositor<VitaGpu>);

impl GpuRenderer {
    pub fn new(gpu: VitaGpu) -> Result<Self> {
        Ok(Self(Compositor::new(gpu, CACHE_LIMIT)?))
    }

    /// Releases every cached texture, e.g. before another game starts.
    pub fn clear_cache(&mut self) {
        self.0.clear_cache();
    }

    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<()> {
        self.0.render(frame)
    }

    /// Shows the canvas scaled into `view` (screen pixels) with an optional crosshair
    /// at a screen position, which device readback never includes.
    pub fn present(
        &mut self,
        view: [f32; 4],
        linear: bool,
        cursor: Option<[f32; 2]>,
    ) -> Result<()> {
        self.0.canvas().ok_or("no GPU canvas to present")?;
        self.0.device().present(view, linear, cursor)?;
        self.0.mark_presented();
        Ok(())
    }

    /// The window now shows a software frame instead of the canvas.
    pub fn forget_presented(&mut self) {
        self.0.forget_presented();
    }

    /// The last presented logical canvas, if the canvas is what the screen shows.
    pub fn read(&mut self) -> Option<Result<RgbaImage>> {
        let size = self.0.presented()?;
        let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        Some(
            self.0
                .device()
                .read(size, &mut pixels)
                .map_err(Into::into)
                .and_then(|()| Ok(RgbaImage::new(size[0], size[1], pixels)?)),
        )
    }
}

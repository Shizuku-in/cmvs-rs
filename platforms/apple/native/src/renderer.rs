use crate::Result;
use cmvs_core::render::raster::RgbaImage;
use cmvs_host::{GpuTarget, HostSession};
use cmvs_wgpu::{GpuFrame, Presenter, Renderer};

pub enum Frame {
    Cpu(RgbaImage),
    Metal(GpuFrame),
}
impl Frame {
    pub fn read(&self) -> Result<RgbaImage> {
        match self {
            Self::Cpu(image) => Ok(image.clone()),
            Self::Metal(frame) => frame.read(),
        }
    }
}
pub struct Metal {
    presenter: Presenter,
    renderer: Renderer,
}
impl Metal {
    #[cfg(target_vendor = "apple")]
    pub fn new(
        surface: cmvs_wgpu::wgpu::Surface<'static>,
        instance: &cmvs_wgpu::wgpu::Instance,
    ) -> Result<Self> {
        let (renderer, adapter) = Renderer::new(instance, Some(&surface))?;
        Ok(Self {
            presenter: Presenter::new(surface, &adapter, &renderer)?,
            renderer,
        })
    }
    pub fn draw(
        &mut self,
        session: &mut HostSession,
        size: [u32; 2],
        mode: u32,
        linear: bool,
    ) -> Result<Frame> {
        let frame = session.with_gpu_target(linear, |target| match target {
            GpuTarget::Movie {
                image,
                size,
                linear,
            } => self.renderer.render_image(image, size, linear),
            GpuTarget::Scene(frame) => self.renderer.render(&frame),
        })?;
        self.presenter
            .present(&self.renderer, &frame, size, mode, linear, false)?;
        if self.presenter.lost {
            return Err("Metal surface lost".into());
        }
        Ok(Frame::Metal(frame))
    }
}

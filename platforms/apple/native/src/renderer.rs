use crate::Result;
use cmvs_core::render::{
    raster::{RenderScale, RgbaImage, ScaleFilter},
    viewport::Viewport,
};
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
type RebuildPresenter = Box<dyn Fn(&Renderer) -> Result<Presenter>>;

pub struct Metal {
    rebuild: RebuildPresenter,
    presenter: Presenter,
    renderer: Renderer,
}
impl Metal {
    #[cfg(target_vendor = "apple")]
    pub fn new(
        instance: &cmvs_wgpu::wgpu::Instance,
        anti_aliasing: bool,
        render_scale: RenderScale,
        create_surface: impl Fn() -> Result<cmvs_wgpu::wgpu::Surface<'static>> + 'static,
    ) -> Result<Self> {
        let surface = create_surface()?;
        let (mut renderer, adapter) = Renderer::new(instance, Some(&surface))?;
        renderer.set_anti_aliasing(anti_aliasing);
        renderer.set_render_scale(render_scale);
        Ok(Self {
            presenter: Presenter::new(surface, &adapter, &renderer)?,
            renderer,
            rebuild: Box::new(move |renderer| {
                Presenter::new(create_surface()?, &adapter, renderer)
            }),
        })
    }
    pub fn draw(
        &mut self,
        session: &mut HostSession,
        size: [u32; 2],
        mode: u32,
        linear: bool,
        filter: ScaleFilter,
    ) -> Result<Frame> {
        let logical = session.runtime.screen_size();
        let viewport = Viewport::new(logical, size, mode)
            .ok()
            .map(|view| [view.width, view.height]);
        self.renderer.set_viewport_size(viewport);
        let render_size = self.renderer.resolved_render_size(logical);
        session
            .font
            .set_raster_scale(RenderScale::font_scale(logical, render_size));
        let frame = session.with_gpu_target(linear, |target| match target {
            GpuTarget::Movie {
                image,
                size,
                render_mode,
            } => self.renderer.render_image(image, size, render_mode),
            GpuTarget::Scene(frame) => self.renderer.render(&frame),
        })?;
        if self.presenter.lost {
            self.presenter = (self.rebuild)(&self.renderer)?;
        }
        self.presenter
            .present(&self.renderer, &frame, size, mode, filter, false)?;
        Ok(Frame::Metal(frame))
    }
}

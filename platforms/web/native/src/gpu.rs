//! Worker-owned WebGPU surface; only explicit device screenshots read pixels back.
use cmvs_core::{runner::Startup, scene_render::prepare_scene, settings::DisplayPreferences};
use cmvs_font::font::FontRenderer;
use cmvs_wgpu::{GpuFrame, Presenter, Renderer, wgpu};
use std::error::Error;
use web_sys::OffscreenCanvas;

pub struct BrowserGpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    canvas: OffscreenCanvas,
    renderer: Renderer,
    presenter: Presenter,
    pub frame: Option<GpuFrame>,
}
impl BrowserGpu {
    pub async fn new(canvas: OffscreenCanvas, size: [u32; 2]) -> Result<Self, Box<dyn Error>> {
        canvas.set_width(size[0]);
        canvas.set_height(size[1]);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface =
            instance.create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas.clone()))?;
        let (renderer, adapter) = Renderer::new_async(&instance, Some(&surface)).await?;
        let presenter = Presenter::new(surface, &adapter, &renderer)?;
        Ok(Self {
            instance,
            adapter,
            canvas,
            renderer,
            presenter,
            frame: None,
        })
    }
    pub fn draw(
        &mut self,
        startup: &Startup<'_>,
        font: &mut FontRenderer,
        display: DisplayPreferences,
    ) -> Result<bool, Box<dyn Error>> {
        self.renderer.check()?;
        if self.presenter.lost {
            let surface = self
                .instance
                .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(self.canvas.clone()))?;
            self.presenter = Presenter::new(surface, &self.adapter, &self.renderer)?;
        }
        let size = startup.screen_size();
        let frame = if let Some(image) = startup.movie_frame() {
            self.renderer
                .render_image(image, size, display.render_mode == 1)?
        } else {
            let objects = font.prepare_objects(startup.messages())?;
            let mut additional: Vec<_> = objects.iter().collect();
            additional.extend(startup.messages().visible_window_objects());
            let scene = prepare_scene(
                startup.scene(),
                Some(startup.camera()),
                &additional,
                size[0],
                size[1],
            )?;
            self.renderer.render(&scene)?
        };
        let presented = self.presenter.present(
            &self.renderer,
            &frame,
            size,
            display.aspect_mode,
            display.render_mode == 1,
            false,
        )?;
        if presented {
            self.frame = Some(frame);
        }
        Ok(presented)
    }
}

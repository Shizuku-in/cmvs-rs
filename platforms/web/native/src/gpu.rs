//! Worker-owned WebGPU surface; only explicit device screenshots read pixels back.
use cmvs_core::{
    host::settings::DisplayPreferences, render::raster::ScaleFilter, runtime::Runtime,
};
use cmvs_font::font::FontRenderer;
use cmvs_host::{GpuTarget, with_gpu_target};
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
    pub capture: Option<GpuFrame>,
}
impl BrowserGpu {
    pub async fn new(
        canvas: OffscreenCanvas,
        size: [u32; 2],
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
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
            capture: None,
        })
    }
    pub fn texture_limit(&self) -> u32 {
        self.renderer.device.limits().max_texture_dimension_2d
    }
    pub fn draw(
        &mut self,
        startup: &Runtime,
        font: &mut FontRenderer,
        display: DisplayPreferences,
        output: [u32; 2],
        filter: Option<ScaleFilter>,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.renderer.check()?;
        if self.presenter.lost {
            let surface = self
                .instance
                .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(self.canvas.clone()))?;
            self.presenter = Presenter::new(surface, &self.adapter, &self.renderer)?;
        }
        let linear = display.render_mode == 1;
        let frame = with_gpu_target(startup, Some(font), linear, |target| match target {
            GpuTarget::Movie {
                image,
                size,
                render_mode,
            } => self.renderer.render_image(image, size, render_mode),
            GpuTarget::Scene(scene) => self.renderer.render(&scene),
        })?;
        // The browser lays out the canvas and maps input over its full extent.
        let mode = if filter.is_some() {
            3
        } else {
            cmvs_core::render::viewport::presentation_mode(
                display.fullscreen,
                display.aspect_scaling,
                display.aspect_mode,
            )
        };
        if self.canvas.width() != output[0] {
            self.canvas.set_width(output[0]);
        }
        if self.canvas.height() != output[1] {
            self.canvas.set_height(output[1]);
        }
        let presented = self.presenter.present(
            &self.renderer,
            &frame,
            output,
            mode,
            filter.unwrap_or_else(|| linear.into()),
            false,
        )?;
        if presented {
            self.capture = None;
            self.frame = Some(frame);
        } else if startup.capture_due() {
            self.capture = Some(frame);
        }
        Ok(presented)
    }
}

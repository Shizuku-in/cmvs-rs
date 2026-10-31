use super::*;
/// Window presentation is separate from the logical CMVS canvas.
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    pub lost: bool,
    configured: bool,
}
impl Presenter {
    pub fn new(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        renderer: &Renderer,
    ) -> Result<Self> {
        let caps = surface.get_capabilities(adapter);
        let format = *caps.formats.first().ok_or("no GPU surface formats")?;
        let view_format = format.remove_srgb_suffix();
        let mut config = surface
            .get_default_config(adapter, 1, 1)
            .ok_or("no GPU surface configuration")?;
        config.format = format;
        config.view_formats = vec![view_format];
        let (layout, pipeline) = scaling_pipeline(&renderer.device, view_format);
        Ok(Self {
            surface,
            config,
            layout,
            pipeline,
            format: view_format,
            lost: false,
            configured: false,
        })
    }
    /// Returns false when no frame was presented (zero size, timeout, or surface recovery).
    pub fn present(
        &mut self,
        renderer: &Renderer,
        frame: &GpuFrame,
        size: [u32; 2],
        mode: u32,
        linear: bool,
        preview: bool,
    ) -> Result<bool> {
        renderer.check()?;
        if size.contains(&0) {
            return Ok(false);
        }
        if size
            .iter()
            .any(|&v| v > renderer.device.limits().max_texture_dimension_2d)
        {
            return Err("GPU surface exceeds device limits".into());
        }
        if !self.configured || self.config.width != size[0] || self.config.height != size[1] {
            self.config.width = size[0];
            self.config.height = size[1];
            self.surface.configure(&renderer.device, &self.config);
            self.configured = true;
        }
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.configured = false;
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.lost = true;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configured = false;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                renderer.check()?;
                return Err("GPU surface validation failed".into());
            }
        };
        let viewport = cmvs_core::viewport::Viewport::new(frame.size, size, mode)?;
        let data = [
            viewport.left as f32,
            viewport.top as f32,
            viewport.width as f32,
            viewport.height as f32,
            frame.size[0] as f32,
            frame.size[1] as f32,
            u32::from(linear) as f32,
            u32::from(preview) as f32,
        ];
        let bytes: Vec<_> = data.into_iter().flat_map(f32::to_le_bytes).collect();
        let buffer = renderer
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let source = frame.texture.create_view(&Default::default());
        let group = renderer
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source),
                    },
                ],
            });
        let target = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        renderer.queue.submit([encoder.finish()]);
        renderer.queue.present(output);
        renderer.check()?;
        Ok(true)
    }
}

fn scaling_pipeline(
    device: &wgpu::Device,
    view_format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("window scaling"),
        source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: view_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    (layout, pipeline)
}

pub(crate) fn scale_image(
    renderer: &mut Renderer,
    image: &RgbaImage,
    size: [u32; 2],
    linear: bool,
) -> Result<GpuFrame> {
    renderer.check()?;
    let viewport = cmvs_core::viewport::Viewport::new([image.width(), image.height()], size, 1)?;
    if size
        .iter()
        .any(|&v| v > renderer.device.limits().max_texture_dimension_2d)
    {
        return Err("GPU canvas exceeds device limits".into());
    }
    let source = renderer.texture(image)?.create_view(&Default::default());
    let data = [
        viewport.left as f32,
        viewport.top as f32,
        viewport.width as f32,
        viewport.height as f32,
        image.width() as f32,
        image.height() as f32,
        u32::from(linear) as f32,
        0.0,
    ];
    let bytes: Vec<_> = data.into_iter().flat_map(f32::to_le_bytes).collect();
    let buffer = renderer
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let (layout, pipeline) = renderer
        .scaling
        .get_or_insert_with(|| scaling_pipeline(&renderer.device, wgpu::TextureFormat::Rgba8Unorm));
    let group = renderer
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source),
                },
            ],
        });
    let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("movie canvas"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("movie scale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    renderer.queue.submit([encoder.finish()]);
    renderer.check()?;
    Ok(GpuFrame {
        texture,
        device: renderer.device.clone(),
        queue: renderer.queue.clone(),
        size,
    })
}

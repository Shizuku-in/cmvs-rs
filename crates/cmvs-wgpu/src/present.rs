use super::*;

struct ScaleGroup {
    source: Weak<Texture>,
    binding: wgpu::BindGroup,
}

/// One scaling pass per submission. Queue writes precede that submission, just as
/// for scene uniforms; later changes cannot affect earlier submitted frames.
pub(super) struct Scaling {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    data: Option<[u8; 32]>,
    groups: Vec<ScaleGroup>,
}
impl Scaling {
    pub(super) fn prune(&mut self) {
        self.groups.retain(|group| group.source.strong_count() != 0);
    }

    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let (layout, pipeline) = scaling_pipeline(device, format);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scaling parameters"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            buffer,
            data: None,
            groups: Vec::new(),
        }
    }

    fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &Shared<Texture>,
        data: [f32; 8],
    ) -> wgpu::BindGroup {
        let mut bytes = [0u8; 32];
        for (out, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(data) {
            *out = value.to_le_bytes();
        }
        if self.data != Some(bytes) {
            queue.write_buffer(&self.buffer, 0, &bytes);
            self.data = Some(bytes);
        }
        // Weak owners do not pin a recyclable texture. Stale bindings disappear
        // after its pool/cache releases it, even though the GPU view is a handle.
        self.prune();
        if let Some(at) = self.groups.iter().position(|group| {
            group
                .source
                .upgrade()
                .is_some_and(|owner| Shared::ptr_eq(&owner, source))
        }) {
            let group = self.groups.remove(at);
            let binding = group.binding.clone();
            self.groups.push(group);
            return binding;
        }
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scaling texture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
            ],
        });
        if self.groups.len() == 16 {
            self.groups.remove(0);
        }
        self.groups.push(ScaleGroup {
            source: Shared::downgrade(source),
            binding: binding.clone(),
        });
        binding
    }
}

/// Window presentation is separate from the logical CMVS canvas.
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scaling: Scaling,
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
        let scaling = Scaling::new(&renderer.device, view_format);
        Ok(Self {
            surface,
            config,
            scaling,
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
        let group = self
            .scaling
            .bind(&renderer.device, &renderer.queue, &frame.target, data);
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
            pass.set_pipeline(&self.scaling.pipeline);
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
    renderer.bindings.clear();
    let source = renderer.texture(image)?;
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
    let target = renderer.canvases.acquire(&renderer.device, size);
    let scaling = renderer
        .scaling
        .get_or_insert_with(|| Scaling::new(&renderer.device, wgpu::TextureFormat::Rgba8Unorm));
    let group = scaling.bind(&renderer.device, &renderer.queue, &source, data);
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("movie scale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&scaling.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    renderer.queue.submit([encoder.finish()]);
    renderer.check()?;
    Ok(GpuFrame {
        target,
        device: renderer.device.clone(),
        queue: renderer.queue.clone(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn scaling_reuses_uniforms_and_bindings_without_changing_pending_frames() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, _) = Renderer::new(&instance, None).unwrap();
        let a = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
        let b = RgbaImage::new(2, 1, vec![0, 0, 255, 255, 255, 255, 255, 128]).unwrap();
        let mut frames = Vec::new();
        let first = gpu.render_image(&a, [4, 3], false).unwrap();
        let scaling = gpu.scaling.as_ref().unwrap();
        let buffer = scaling.buffer.clone();
        let binding = scaling.groups[0].binding.clone();
        frames.push((first, &a, [4, 3], false));
        for (image, size, linear) in [(&a, [7, 5], true), (&b, [4, 3], false), (&a, [2, 1], true)] {
            frames.push((
                gpu.render_image(image, size, linear).unwrap(),
                image,
                size,
                linear,
            ));
        }
        let scaling = gpu.scaling.as_ref().unwrap();
        assert_eq!(scaling.buffer, buffer);
        assert_eq!(scaling.groups.last().unwrap().binding, binding);
        assert_eq!(scaling.groups.len(), 2);
        // Read only after every submission, with geometry, filters and sources changed.
        for (frame, image, size, linear) in frames {
            let mut expected = vec![0; (size[0] * size[1]) as usize];
            image
                .draw_scaled(&mut expected, size[0], size[1], 1, linear)
                .unwrap();
            let actual = frame.read().unwrap();
            for (word, pixel) in expected.iter().zip(actual.pixels().as_chunks::<4>().0) {
                for (c, shift) in [16, 8, 0].into_iter().enumerate() {
                    assert!(pixel[c].abs_diff((word >> shift) as u8) <= 2);
                }
                assert_eq!(pixel[3], 255);
            }
        }
        for red in 0..32 {
            let image = RgbaImage::new(1, 1, vec![red, 0, 0, 255]).unwrap();
            drop(gpu.render_image(&image, [1, 1], false).unwrap());
        }
        assert_eq!(gpu.scaling.as_ref().unwrap().groups.len(), 16);
        gpu.cache.clear();
        gpu.cache_bytes = 0;
        let after_eviction = gpu.render_image(&a, [2, 1], false).unwrap();
        assert_eq!(gpu.scaling.as_ref().unwrap().groups.len(), 1);
        assert_eq!(gpu.scaling.as_ref().unwrap().buffer, buffer);
        assert_eq!(
            after_eviction.read().unwrap().pixels(),
            &[255, 0, 0, 255, 0, 0, 0, 255]
        );
        gpu.check().unwrap();
    }
}

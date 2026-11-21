use super::*;
use cmvs_core::render::viewport::Viewport;

struct ScaleGroup {
    image: Weak<Texture>,
    original: Weak<Texture>,
    binding: wgpu::BindGroup,
}

/// Scaling parameters are shared by the passes in one submission. Queue writes precede it, just as
/// for scene uniforms; later changes cannot affect earlier submitted frames.
pub(super) struct Scaling {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    intermediate_pipeline: wgpu::RenderPipeline,
    intermediate: Option<Shared<Texture>>,
    buffer: wgpu::Buffer,
    data: Option<[u8; 48]>,
    groups: Vec<ScaleGroup>,
}

struct ScaleOptions {
    view: Viewport,
    filter: ScaleFilter,
    preview: bool,
}

impl Scaling {
    pub(super) fn prune(&mut self) {
        self.groups
            .retain(|group| group.image.strong_count() != 0 && group.original.strong_count() != 0);
    }

    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let (layout, pipeline, intermediate_pipeline) = scaling_pipelines(device, format);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scaling parameters"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            intermediate_pipeline,
            intermediate: None,
            buffer,
            data: None,
            groups: Vec::new(),
        }
    }

    fn bind(
        &mut self,
        device: &wgpu::Device,
        image: &Shared<Texture>,
        original: &Shared<Texture>,
    ) -> wgpu::BindGroup {
        // Weak owners do not pin a recyclable texture. Stale bindings disappear
        // after its pool/cache releases it, even though the GPU view is a handle.
        self.prune();
        let image_owner = Shared::downgrade(image);
        let original_owner = Shared::downgrade(original);
        if let Some(at) = self.groups.iter().position(|group| {
            Weak::ptr_eq(&group.image, &image_owner)
                && Weak::ptr_eq(&group.original, &original_owner)
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
                    resource: wgpu::BindingResource::TextureView(&image.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&original.view),
                },
            ],
        });
        if self.groups.len() == 16 {
            self.groups.remove(0);
        }
        self.groups.push(ScaleGroup {
            image: image_owner,
            original: original_owner,
            binding: binding.clone(),
        });
        binding
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &Shared<Texture>,
        target: &wgpu::TextureView,
        options: ScaleOptions,
    ) {
        let ScaleOptions {
            view,
            filter,
            preview,
        } = options;
        let [width, height] = source.size();
        // Shader filter selectors are independent of the game's numbered modes.
        // Keep these in sync with the constants in present.wgsl.
        let (shader_filter, anti_ringing) = match filter {
            ScaleFilter::Nearest => (0, false),
            ScaleFilter::Bilinear => (1, false),
            ScaleFilter::Keys => (2, false),
            ScaleFilter::Lanczos4 => (3, false),
            ScaleFilter::Mitchell => (4, false),
            ScaleFilter::Lanczos3 { anti_ringing } => (5, anti_ringing),
        };
        let mut data = [
            view.left as i32,
            view.top as i32,
            view.width as i32,
            view.height as i32,
            width as i32,
            height as i32,
            shader_filter,
            i32::from(preview),
            0,
            0,
            0,
            i32::from(anti_ringing),
        ];
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut intermediate = None;
        if !matches!(filter, ScaleFilter::Nearest | ScaleFilter::Bilinear)
            && [width, height] != [view.width, view.height]
        {
            let [xs, ys] = view.visible_ranges();
            let visible_width = (xs.end - xs.start) as u32;
            let visible_height = (ys.end - ys.start) as u32;
            // Choose the smaller intermediate, with the cropped margins omitted.
            let vertical_first = u64::from(visible_width) * u64::from(height)
                > u64::from(width) * u64::from(visible_height);
            let size = if vertical_first {
                [width, visible_height]
            } else {
                [visible_width, height]
            };
            data[8] = xs.start as i32;
            data[9] = ys.start as i32;
            data[10] = i32::from(vertical_first);
            if self
                .intermediate
                .as_ref()
                .is_none_or(|image| image.size() != size)
            {
                // Keep signed intermediate weights until both passes are complete.
                self.intermediate = Some(Texture::with_format(
                    device,
                    size,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    wgpu::TextureFormat::Rgba16Float,
                ));
            }
            let binding = self.bind(device, source, source);
            let image = self.intermediate.as_ref().expect("intermediate texture");
            encode_scale(
                &mut encoder,
                &image.view,
                &self.intermediate_pipeline,
                &binding,
            );
            intermediate = Some(Shared::clone(image));
        }
        let mut bytes = [0u8; 48];
        for (out, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(data) {
            *out = value.to_le_bytes();
        }
        if self.data != Some(bytes) {
            queue.write_buffer(&self.buffer, 0, &bytes);
            self.data = Some(bytes);
        }
        let image = intermediate.as_ref().unwrap_or(source);
        let original = if anti_ringing { source } else { image };
        let binding = self.bind(device, image, original);
        encode_scale(&mut encoder, target, &self.pipeline, &binding);
        queue.submit([encoder.finish()]);
    }
}

fn encode_scale(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("frame scaling"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
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
        aspect_mode: u32,
        filter: impl Into<ScaleFilter>,
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
        let view = Viewport::new(frame.size, size, aspect_mode)?;
        let target = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format),
            ..Default::default()
        });
        self.scaling.draw(
            &renderer.device,
            &renderer.queue,
            &frame.target,
            &target,
            ScaleOptions {
                view,
                filter: filter.into(),
                preview,
            },
        );
        renderer.queue.present(output);
        renderer.check()?;
        Ok(true)
    }
}

fn scaling_pipelines(
    device: &wgpu::Device,
    view_format: wgpu::TextureFormat,
) -> (
    wgpu::BindGroupLayout,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
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
            texture(1),
            texture(2),
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
    let pipeline = |entry, format| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };

    (
        layout,
        pipeline("fs", view_format),
        pipeline("fs_intermediate", wgpu::TextureFormat::Rgba16Float),
    )
}

pub(crate) fn scale_image(
    renderer: &mut Renderer,
    image: &RgbaImage,
    size: [u32; 2],
    filter: ScaleFilter,
) -> Result<GpuFrame> {
    renderer.check()?;
    let view = Viewport::new([image.width(), image.height()], size, 1)?;
    if size
        .iter()
        .any(|&v| v > renderer.device.limits().max_texture_dimension_2d)
    {
        return Err("GPU canvas exceeds device limits".into());
    }
    renderer.bindings.clear();
    let source = renderer.texture(image)?;
    let target = renderer.canvases.acquire(&renderer.device, size);
    let scaling = renderer
        .scaling
        .get_or_insert_with(|| Scaling::new(&renderer.device, wgpu::TextureFormat::Rgba8Unorm));
    scaling.draw(
        &renderer.device,
        &renderer.queue,
        &source,
        &target.view,
        ScaleOptions {
            view,
            filter,
            preview: false,
        },
    );
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
    fn high_quality_scaling_matches_cpu_with_transparency_and_cropping() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, _) = Renderer::new(&instance, None).unwrap();
        let image = RgbaImage::new(
            11,
            7,
            (0..77)
                .flat_map(|n| [n * 37, n * 53, n * 13, n % 7 * 42].map(|v| v as u8))
                .collect(),
        )
        .unwrap();
        let source = gpu.preview_image(&image).unwrap();
        let mut scaling = Scaling::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        let mut frames = Vec::new();
        // Exercise both pass orders, exact sample centers, single-axis resizing,
        // letterbox, crop, and centered identity scaling.
        for (size, mode) in [
            ([22, 5], 3),
            ([5, 17], 3),
            ([33, 21], 3),
            ([11, 21], 3),
            ([13, 14], 2),
            ([12, 19], 1),
            ([14, 10], 0),
        ] {
            let view = Viewport::new(source.size, size, mode).unwrap();
            for filter in [
                ScaleFilter::Keys,
                ScaleFilter::Lanczos4,
                ScaleFilter::Mitchell,
                ScaleFilter::Lanczos3 {
                    anti_ringing: false,
                },
                ScaleFilter::Lanczos3 { anti_ringing: true },
            ] {
                let target = gpu.canvases.acquire(&gpu.device, size);
                scaling.draw(
                    &gpu.device,
                    &gpu.queue,
                    &source.target,
                    &target.view,
                    ScaleOptions {
                        view,
                        filter,
                        preview: false,
                    },
                );
                frames.push((
                    GpuFrame {
                        target,
                        device: gpu.device.clone(),
                        queue: gpu.queue.clone(),
                        size,
                    },
                    &image,
                    mode,
                    filter,
                ));
            }
        }
        // Reuse the intermediate with a different original before reading either result.
        let solid = RgbaImage::new(11, 7, [240, 25, 100, 255].repeat(77)).unwrap();
        let filter = ScaleFilter::Lanczos3 { anti_ringing: true };
        for source in [&image, &solid, &image] {
            let frame = gpu.render_image(source, [33, 21], filter).unwrap();
            frames.push((frame, source, 1, filter));
        }
        for (frame, image, mode, filter) in frames {
            let [width, height] = frame.size;
            let mut expected = vec![0; (width * height) as usize];
            image
                .draw_filtered(&mut expected, width, height, mode, filter)
                .unwrap();
            let actual = frame.read().unwrap();
            for (word, pixel) in expected.iter().zip(actual.pixels().as_chunks::<4>().0) {
                for (c, shift) in [16, 8, 0].into_iter().enumerate() {
                    assert!(
                        pixel[c].abs_diff((word >> shift) as u8) <= 1,
                        "{filter:?}, {width}x{height}, aspect {mode}: {word:06x} != {pixel:?}"
                    );
                }
                assert_eq!(pixel[3], 255);
            }
        }
        gpu.check().unwrap();
    }

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

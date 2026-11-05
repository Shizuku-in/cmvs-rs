//! GPU execution of validated CMVS draw commands, with lazy logical-frame readback.
use cmvs_core::{
    raster::{CacheKey, RgbaImage},
    scene_render::{
        Layer, RenderFrame,
        shader::{ShaderBlend, shader_draw},
    },
};
use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};
pub use wgpu;
use wgpu::util::DeviceExt;
mod bindings;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CACHE_LIMIT: usize = 128 * 1024 * 1024;
/// Movie frames live apart from the content cache so replaced frames never evict
/// static images. Each player shows one frame (three scene slots and the full-screen
/// player), which leaves room for images that stopped players retain.
const FRAME_TEXTURES: usize = 8;

/// An immutable submitted logical frame. Readback is performed only on demand.
pub struct GpuFrame {
    pub texture: wgpu::Texture,
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: [u32; 2],
}
impl GpuFrame {
    pub fn dimensions(&self) -> [u32; 2] {
        self.size
    }
    fn begin_read(&self) -> wgpu::Buffer {
        let [w, h] = self.size;
        let pitch = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame readback"),
            size: u64::from(pitch) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(&self) -> Result<RgbaImage> {
        let buffer = self.begin_read();
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        self.finish_read(&buffer)
    }
    /// Browser mappings complete through the event loop, without blocking the Worker.
    #[cfg(target_arch = "wasm32")]
    pub async fn read_async(&self) -> Result<RgbaImage> {
        let buffer = self.begin_read();
        let state = Arc::new(Mutex::new((None, None::<std::task::Waker>)));
        let callback = state.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let mut state = callback.lock().expect("readback lock");
                state.0 = Some(result);
                let waker = state.1.take();
                drop(state);
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        std::future::poll_fn(|cx| {
            let mut state = state.lock().expect("readback lock");
            if let Some(result) = state.0.take() {
                std::task::Poll::Ready(result)
            } else {
                state.1 = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await?;
        self.finish_read(&buffer)
    }
    fn finish_read(&self, buffer: &wgpu::Buffer) -> Result<RgbaImage> {
        let [w, h] = self.size;
        let pitch = (w * 4).div_ceil(256) * 256;
        let bytes = buffer.slice(..).get_mapped_range()?;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in bytes.chunks_exact(pitch as usize) {
            pixels.extend_from_slice(&row[..(w * 4) as usize]);
        }
        drop(bytes);
        buffer.unmap();
        Ok(RgbaImage::new(w, h, pixels)?)
    }
}
struct Cached {
    view: wgpu::TextureView,
    bytes: usize,
    used: u64,
}
/// Device resources are confined to this backend; the core remains dependency-free.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    bindings: bindings::DrawBindings,
    scaling: Option<(wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
    pipelines: [wgpu::RenderPipeline; 3],
    cache: HashMap<([u8; 32], u32, u32), Cached>,
    cache_bytes: usize,
    frames: HashMap<u64, Cached>,
    clock: u64,
    errors: Arc<Mutex<Option<String>>>,
    pub uploaded_bytes: u64,
}
impl Renderer {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<(Self, wgpu::Adapter)> {
        pollster::block_on(Self::new_async(instance, surface))
    }
    pub async fn new_async(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<(Self, wgpu::Adapter)> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: surface,
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("CMVS GPU"),
                ..Default::default()
            })
            .await?;
        let errors = Arc::new(Mutex::new(None));
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |error| {
            *sink.lock().expect("GPU error lock") = Some(error.to_string());
        }));
        let sink = errors.clone();
        device.set_device_lost_callback(move |reason, message| {
            *sink.lock().expect("GPU error lock") =
                Some(format!("GPU device lost: {reason:?}: {message}"));
        });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(
                    (cmvs_core::scene_render::shader::PARAM_FLOATS * 4) as u64,
                ),
            },
            count: None,
        }];
        for binding in 1..=3 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene bindings"),
            entries: &entries,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CMVS scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|mode| {
            let blend = match mode {
                0 => None,
                1 => Some(wgpu::BlendState::ALPHA_BLENDING),
                _ => Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("CMVS blend"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend,
                        write_mask: wgpu::ColorWrites::RED
                            | wgpu::ColorWrites::GREEN
                            | wgpu::ColorWrites::BLUE,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let bindings = bindings::DrawBindings::new(&device);
        let renderer = Self {
            device,
            queue,
            layout,
            bindings,
            scaling: None,
            pipelines,
            cache: HashMap::new(),
            cache_bytes: 0,
            frames: HashMap::new(),
            clock: 0,
            errors,
            uploaded_bytes: 0,
        };
        renderer.check()?;
        Ok((renderer, adapter))
    }
    pub fn check(&self) -> Result<()> {
        if let Some(message) = self.errors.lock().expect("GPU error lock").take() {
            return Err(message.into());
        }
        Ok(())
    }
    fn texture(&mut self, image: &RgbaImage) -> Result<wgpu::TextureView> {
        let [w, h] = [image.width(), image.height()];
        if w > self.device.limits().max_texture_dimension_2d
            || h > self.device.limits().max_texture_dimension_2d
        {
            return Err("GPU image exceeds device texture limits".into());
        }
        let key = match image.cache_key() {
            CacheKey::Frame(id) => return Ok(self.frame_texture(id, image)),
            CacheKey::Content(digest, w, h) => (digest, w, h),
        };
        self.clock += 1;
        if let Some(cached) = self.cache.get_mut(&key) {
            cached.used = self.clock;
            return Ok(cached.view.clone());
        }
        let bytes = image.pixels().len();
        while self.cache_bytes + bytes > CACHE_LIMIT && !self.cache.is_empty() {
            let oldest = *self
                .cache
                .iter()
                .min_by_key(|(_, c)| c.used)
                .expect("nonempty cache")
                .0;
            self.cache_bytes -= self.cache.remove(&oldest).expect("cache entry").bytes;
        }
        let texture = self.upload(image);
        if bytes <= CACHE_LIMIT {
            self.cache.insert(
                key,
                Cached {
                    view: texture.clone(),
                    bytes,
                    used: self.clock,
                },
            );
            self.cache_bytes += bytes;
        }
        Ok(texture)
    }
    fn frame_texture(&mut self, id: u64, image: &RgbaImage) -> wgpu::TextureView {
        self.clock += 1;
        if let Some(cached) = self.frames.get_mut(&id) {
            cached.used = self.clock;
            return cached.view.clone();
        }
        if self.frames.len() >= FRAME_TEXTURES {
            let oldest = *self
                .frames
                .iter()
                .min_by_key(|(_, c)| c.used)
                .expect("nonempty frames")
                .0;
            self.frames.remove(&oldest);
        }
        let texture = self.upload(image);
        self.frames.insert(
            id,
            Cached {
                view: texture.clone(),
                bytes: image.pixels().len(),
                used: self.clock,
            },
        );
        texture
    }
    fn upload(&mut self, image: &RgbaImage) -> wgpu::TextureView {
        let [w, h] = [image.width(), image.height()];
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CMVS image"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            image.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            texture.size(),
        );
        self.uploaded_bytes += image.pixels().len() as u64;
        texture.create_view(&Default::default())
    }
    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<GpuFrame> {
        self.compose(frame.dimensions(), frame.clear_color(), frame.commands())
    }
    /// Preserves the source alpha for the desktop checkerboard preview path.
    pub fn preview_image(&mut self, image: &RgbaImage) -> Result<GpuFrame> {
        self.bindings.clear();
        let texture = self.texture(image)?.texture().clone();
        Ok(GpuFrame {
            texture,
            device: self.device.clone(),
            queue: self.queue.clone(),
            size: [image.width(), image.height()],
        })
    }
    /// Scales a decoded movie onto an opaque logical canvas entirely on the GPU.
    pub fn render_image(
        &mut self,
        image: &RgbaImage,
        size: [u32; 2],
        linear: bool,
    ) -> Result<GpuFrame> {
        present::scale_image(self, image, size, linear)
    }
    fn compose(
        &mut self,
        size: [u32; 2],
        clear: [u8; 4],
        commands: &[Layer<'_>],
    ) -> Result<GpuFrame> {
        self.check()?;
        let [w, h] = size;
        if size.contains(&0)
            || size
                .iter()
                .any(|&v| v > self.device.limits().max_texture_dimension_2d)
        {
            return Err("invalid GPU canvas size".into());
        }
        self.bindings.prepare(&self.device, commands.len())?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("logical canvas"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
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
        let white = RgbaImage::new(1, 1, vec![255; 4])?;
        let dummy = self.texture(&white)?;
        let mut draws = Vec::with_capacity(commands.len());
        for (index, command) in commands.iter().enumerate() {
            let draw = shader_draw(command, size);
            let mode = match draw.blend {
                ShaderBlend::Replace => 0,
                ShaderBlend::Alpha => 1,
                ShaderBlend::Additive => 2,
            };
            let mut views = [dummy.clone(), dummy.clone(), dummy.clone()];
            for (view, image) in views.iter_mut().zip(draw.textures) {
                if let Some(image) = image {
                    *view = self.texture(image)?;
                }
            }
            let offset = self.bindings.write(index, draw.used_params());
            let group = self.bindings.group(&self.device, &self.layout, views);
            draws.push((mode, group, offset));
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let view = texture.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("CMVS scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear[0]) / 255.0,
                            g: f64::from(clear[1]) / 255.0,
                            b: f64::from(clear[2]) / 255.0,
                            a: f64::from(clear[3]) / 255.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut bound_mode = None;
            for (mode, group, offset) in &draws {
                if bound_mode != Some(*mode) {
                    pass.set_pipeline(&self.pipelines[*mode]);
                    bound_mode = Some(*mode);
                }
                pass.set_bind_group(0, group, &[*offset]);
                pass.draw(0..4, 0..1);
            }
        }
        self.bindings.upload(&self.queue);
        self.queue.submit([encoder.finish()]);
        self.bindings.retain(
            self.cache
                .values()
                .chain(self.frames.values())
                .map(|c| &c.view),
        );
        self.check()?;
        Ok(GpuFrame {
            texture,
            device: self.device.clone(),
            queue: self.queue.clone(),
            size,
        })
    }
}

mod present;
pub use present::Presenter;

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::{
        scene::{DrawState, ObjectImage, SceneHost, SceneObject},
        scene_render::Blend,
        scene_render::prepare_scene,
    };
    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn gpu_matches_cpu_synthetic_scenes_and_reuses_texture_uploads() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, adapter) = Renderer::new(&instance, None).unwrap();
        println!("adapter: {:?}", adapter.get_info());
        let scene = SceneHost::default();
        let pixels: Vec<u8> = (0..24)
            .flat_map(|i| {
                [
                    (i * 31) as u8,
                    (i * 57) as u8,
                    (i * 13) as u8,
                    if i % 3 == 0 { 0 } else { 128 + i as u8 },
                ]
            })
            .collect();
        for (scale, angle, mode, rect) in [
            (1.0f32, 0.0f32, 1, [0u32, 0, 6, 4]),
            (1.0, 0.0, 2, [0, 0, 6, 4]),
            (1.5, 0.0, 1, [0, 0, 6, 4]),
            (0.75, 0.0, 2, [0, 0, 6, 4]),
            (1.0, 60.0, 1, [0, 0, 6, 4]),
            (1.0, 0.0, 1, [u32::MAX, u32::MAX, 8, 6]),
            (1.5, 0.0, 1, [u32::MAX, u32::MAX, 8, 6]),
            (1.0, 60.0, 1, [u32::MAX, u32::MAX, 8, 6]),
        ] {
            let mut draw = DrawState::default();
            draw.words[1] = mode;
            draw.words[2..6].copy_from_slice(&rect);
            draw.words[8..10].copy_from_slice(&[2, 1]);
            draw.words[18..20].fill(scale.to_bits());
            draw.words[20] = angle.to_bits();
            let object = SceneObject {
                image: Some(ObjectImage {
                    name: Vec::new(),
                    filter: None,
                    alpha: true,
                    surface: false,
                    pixels: RgbaImage::new(6, 4, pixels.clone()).unwrap(),
                }),
                draw: Some(draw),
                ..SceneObject::default()
            };
            let frame = prepare_scene(&scene, None, &[&object], 13, 7).unwrap();
            let result = gpu.render(&frame).unwrap();
            let first = result.read().unwrap();
            assert_eq!(first, result.read().unwrap());
            let uploaded = gpu.uploaded_bytes;
            let again = gpu.render(&frame).unwrap().read().unwrap();
            assert_eq!(first, again);
            assert_eq!(uploaded, gpu.uploaded_bytes);
            let expected = frame.render_cpu().unwrap();
            for (i, (a, b)) in first.pixels().iter().zip(expected.pixels()).enumerate() {
                assert!(
                    a.abs_diff(*b) <= 2,
                    "scale={scale} angle={angle} mode={mode} byte={i}: {a} != {b}"
                );
            }
        }

        // World billboards retain queue 5's WRAP sampler at both texture edges.
        let wrapped = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap(),
        };
        let image = gpu
            .compose(
                [4, 2],
                [0, 0, 0, 255],
                &[Layer::Billboard {
                    image: &wrapped,
                    quad: [0.0, 0.0, 4.0, 2.0],
                    alpha: 255,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        for row in image.pixels().as_chunks::<16>().0 {
            assert_eq!(
                row,
                &[
                    128, 0, 128, 255, 255, 0, 0, 255, 128, 0, 128, 255, 0, 0, 255, 255
                ]
            );
        }

        // Coverage lookup, screen-space second texture, and texture blackening.
        let image = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(
                3,
                1,
                vec![255, 60, 90, 255, 80, 180, 20, 128, 200, 10, 100, 0],
            )
            .unwrap(),
        };
        let mask_image = ObjectImage {
            pixels: RgbaImage::new(3, 1, vec![0, 0, 0, 255, 0, 128, 0, 255, 0, 255, 0, 255])
                .unwrap(),
            ..image.clone()
        };
        let transition = cmvs_core::scene::MaskTransition {
            put: true,
            slot: 0,
            ramp: 100,
            width: 0,
            elapsed: 50,
            mask: mask_image.clone(),
        };
        let frame = gpu
            .compose(
                [3, 1],
                [20, 30, 40, 255],
                &[
                    Layer::Solid {
                        rect: [0, 0, 3, 1],
                        color: [20, 30, 40],
                        alpha: 255,
                        blend: Blend::Replace,
                    },
                    // The full coverage table must work at a nonzero dynamic offset.
                    Layer::Image {
                        image: &image,
                        rect: [0, 0, 3, 1],
                        corner: [0, 0],
                        alpha: 180,
                        mask: Some(&transition),
                        screen_mask: Some(&mask_image),
                        blend: Blend::Alpha,
                    },
                ],
            )
            .unwrap()
            .read()
            .unwrap();
        for x in 0..3usize {
            let green = mask_image.pixels.pixels()[x * 4 + 1];
            let alpha = (u32::from(image.pixels.pixels()[x * 4 + 3]) * 180 + 127) / 255;
            let alpha = (alpha as f32 * transition.coverage(green)).round() as u32;
            let screen_green = [0u32, 64, 192][x];
            let alpha = (alpha * screen_green + 127) / 255;
            for (c, under) in [20u32, 30, 40].iter().enumerate() {
                let expected = ((u32::from(image.pixels.pixels()[x * 4 + c]) * alpha
                    + under * (255 - alpha)
                    + 127)
                    / 255) as u8;
                assert!(frame.pixels()[x * 4 + c].abs_diff(expected) <= 2);
            }
        }
        for blend in [
            Blend::Replace,
            Blend::Alpha,
            Blend::Additive,
            Blend::BlackReplace,
            Blend::BlackAlpha,
            Blend::BlackAdditive,
        ] {
            let frame = gpu
                .compose(
                    [3, 1],
                    [20, 30, 40, 255],
                    &[Layer::Image {
                        image: &image,
                        rect: [0, 0, 3, 1],
                        corner: [0, 0],
                        alpha: 255,
                        mask: None,
                        screen_mask: None,
                        blend,
                    }],
                )
                .unwrap()
                .read()
                .unwrap();
            for x in 0..3usize {
                for (c, under) in [20u32, 30, 40].iter().enumerate() {
                    let src = if matches!(
                        blend,
                        Blend::BlackReplace | Blend::BlackAlpha | Blend::BlackAdditive
                    ) {
                        0
                    } else {
                        u32::from(image.pixels.pixels()[x * 4 + c])
                    };
                    let alpha = u32::from(image.pixels.pixels()[x * 4 + 3]);
                    let expected = match blend {
                        Blend::Replace | Blend::BlackReplace => src,
                        Blend::Alpha | Blend::BlackAlpha => {
                            (src * alpha + under * (255 - alpha) + 127) / 255
                        }
                        _ => (under + (src * alpha + 127) / 255).min(255),
                    } as u8;
                    assert!(
                        frame.pixels()[x * 4 + c].abs_diff(expected) <= 2,
                        "blend byte mismatch"
                    );
                }
            }
        }
        let fade = cmvs_core::fade::Fade {
            direction: 0,
            color: 0xff804020,
            duration: 100,
            order: 0,
            elapsed: 50,
            finished: false,
            opacity: 128,
        };
        let result = gpu
            .compose(
                [3, 1],
                [10, 20, 30, 255],
                &[
                    Layer::Solid {
                        rect: [0, 0, 3, 1],
                        color: [100, 120, 140],
                        alpha: 255,
                        blend: Blend::Replace,
                    },
                    Layer::Fade(&fade),
                ],
            )
            .unwrap()
            .read()
            .unwrap();
        for (c, (src, dst)) in [128u32, 64, 32].iter().zip([100u32, 120, 140]).enumerate() {
            let expected = ((src * 128 + dst * 127 + 127) / 255) as u8;
            assert!(result.pixels()[c].abs_diff(expected) <= 2);
        }
        let changed = RgbaImage::new(3, 1, [3, 4, 5, 255].repeat(3)).unwrap();
        let result = gpu
            .render_image(&changed, [3, 1], false)
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(result, changed);

        assert_eq!(
            gpu.preview_image(&image.pixels).unwrap().read().unwrap(),
            image.pixels
        );
        // Include 1:1, both letterbox directions, and actual scaling. Transparent
        // source pixels must still be premultiplied over the opaque black canvas.
        for [w, h] in [[3, 1], [3, 5], [7, 1], [13, 7]] {
            let mut point = None;
            for linear in [false, true] {
                let actual = gpu
                    .render_image(&image.pixels, [w, h], linear)
                    .unwrap()
                    .read()
                    .unwrap();
                let mut target = vec![0; (w * h) as usize];
                image
                    .pixels
                    .draw_scaled(&mut target, w, h, 1, linear)
                    .unwrap();
                for (pixel, expected) in actual.pixels().as_chunks::<4>().0.iter().zip(target) {
                    for (a, shift) in [16, 8, 0].into_iter().enumerate() {
                        assert!(
                            pixel[a].abs_diff((expected >> shift) as u8) <= 1,
                            "movie scaling mismatch"
                        );
                    }
                    assert_eq!(pixel[3], 255);
                }
                if [w, h] != [13, 7] {
                    if let Some(point) = &point {
                        assert_eq!(&actual, point, "1:1 point and linear must agree");
                    } else {
                        point = Some(actual);
                    }
                }
            }
        }
        gpu.check().unwrap();
    }
}

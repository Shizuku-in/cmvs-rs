//! GPU execution of validated CMVS draw commands, with lazy logical-frame readback.
use cmvs_core::{
    render::raster::{CacheKey, RenderScale, RgbaImage, ScaleFilter},
    render::scene_render::Layer,
    render::scene_render::RenderFrame,
    render::scene_render::shader::ShaderBlend,
    render::scene_render::shader::flags as shader_flags,
    render::scene_render::shader::shader_draw,
};
use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};
pub use wgpu;
mod bindings;
mod textures;
use textures::{Shared, Texture, TexturePool, Weak};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const CACHE_LIMIT: usize = 128 * 1024 * 1024;
/// Movie frames live apart from the content cache so replaced frames never evict
/// static images. Each player shows one frame (three scene slots and the full-screen
/// player), which leaves room for images that stopped players retain.
const FRAME_TEXTURES: usize = 8;
/// Internal wgpu-only shader flag (above `cmvs_core::render::scene_render::shader::flags`):
/// filter RGB as opaque without alpha premultiplication for `ShaderBlend::Replace`.
const FLAG_OPAQUE_FILTER: u32 = 128;

/// An immutable submitted frame. Readback is performed only on demand and always
/// returns the logical canvas dimensions (downsampling high-resolution renders).
pub struct GpuFrame {
    // Raw handles stay private so an untracked clone cannot outlive its pool lease.
    target: Shared<Texture>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: [u32; 2],
    logical_size: [u32; 2],
}
impl GpuFrame {
    /// Logical canvas dimensions expected by engine captures and presentation viewports.
    pub fn dimensions(&self) -> [u32; 2] {
        self.logical_size
    }
    /// Actual GPU render-target dimensions (larger than [`Self::dimensions`] when high-res scaling is active).
    pub fn render_dimensions(&self) -> [u32; 2] {
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
            self.target.texture.as_image_copy(),
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
    fn map_read(&self) -> Result<wgpu::Buffer> {
        let buffer = self.begin_read();
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        Ok(buffer)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(&self) -> Result<RgbaImage> {
        let buffer = self.map_read()?;
        self.finish_read_logical(&buffer)
    }
    /// Reads back the raw GPU render target without downsampling to logical dimensions.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_native(&self) -> Result<RgbaImage> {
        let buffer = self.map_read()?;
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
        self.finish_read_logical(&buffer)
    }
    fn finish_read_logical(&self, buffer: &wgpu::Buffer) -> Result<RgbaImage> {
        if self.size == self.logical_size {
            return self.finish_read(buffer);
        }
        let [w, _] = self.size;
        let pitch = (w * 4).div_ceil(256) * 256;
        let bytes = buffer.slice(..).get_mapped_range()?;
        let image = downsample_area_rows(&bytes, pitch as usize, self.size, self.logical_size);
        drop(bytes);
        buffer.unmap();
        Ok(image?)
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

/// Separable box-area downsampler streaming one destination row at a time from a strided
/// RGBA byte buffer (such as a mapped GPU readback buffer) to keep scratch memory O(dst_w).
fn downsample_area_rows(
    src: &[u8],
    src_stride: usize,
    [src_w, src_h]: [u32; 2],
    [dst_w, dst_h]: [u32; 2],
) -> std::result::Result<RgbaImage, cmvs_core::FormatError> {
    let [sw, sh] = [src_w as usize, src_h as usize];
    let [dw, dh] = [dst_w as usize, dst_h as usize];
    let scale_x = sw as f32 / dw as f32;
    let scale_y = sh as f32 / dh as f32;
    let mut cached_y: Option<usize> = None;
    let mut horiz_row = vec![[0f32; 4]; dw];
    let mut dst_row = vec![[0f32; 4]; dw];
    let mut out = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let y0 = dy as f32 * scale_y;
        let y1 = ((dy + 1) as f32 * scale_y).min(sh as f32);
        let iy0 = y0.floor() as usize;
        let iy1 = (y1.ceil() as usize).min(sh);
        let inv_y = 1.0 / (y1 - y0);
        dst_row.fill([0f32; 4]);
        for iy in iy0..iy1 {
            let top = (iy as f32).max(y0);
            let bottom = ((iy + 1) as f32).min(y1);
            let wy = (bottom - top) * inv_y;
            if wy <= 0.0 {
                continue;
            }
            if cached_y != Some(iy) {
                let (row, _) = src[iy * src_stride..][..sw * 4].as_chunks::<4>();
                for (dx, slot) in horiz_row.iter_mut().enumerate() {
                    let x0 = dx as f32 * scale_x;
                    let x1 = ((dx + 1) as f32 * scale_x).min(sw as f32);
                    let ix0 = x0.floor() as usize;
                    let ix1 = (x1.ceil() as usize).min(sw);
                    let inv_x = 1.0 / (x1 - x0);
                    let mut acc = [0f32; 4];
                    for (ix, px) in row.iter().enumerate().take(ix1).skip(ix0) {
                        let left = (ix as f32).max(x0);
                        let right = ((ix + 1) as f32).min(x1);
                        let wx = (right - left) * inv_x;
                        let a = f32::from(px[3]);
                        let af = a / 255.0;
                        acc[0] += f32::from(px[0]) * af * wx;
                        acc[1] += f32::from(px[1]) * af * wx;
                        acc[2] += f32::from(px[2]) * af * wx;
                        acc[3] += a * wx;
                    }
                    *slot = acc;
                }
                cached_y = Some(iy);
            }
            for (dst, src_px) in dst_row.iter_mut().zip(&horiz_row) {
                for c in 0..4 {
                    dst[c] += src_px[c] * wy;
                }
            }
        }
        let out_row = &mut out[dy * dw * 4..][..dw * 4];
        for (px_out, acc) in out_row.as_chunks_mut::<4>().0.iter_mut().zip(&dst_row) {
            let a = acc[3].clamp(0.0, 255.0);
            let a_u8 = a.round() as u8;
            if a_u8 != 0 {
                let unpremul = 255.0 / a;
                px_out[0] = (acc[0] * unpremul).round().clamp(0.0, 255.0) as u8;
                px_out[1] = (acc[1] * unpremul).round().clamp(0.0, 255.0) as u8;
                px_out[2] = (acc[2] * unpremul).round().clamp(0.0, 255.0) as u8;
            }
            px_out[3] = a_u8;
        }
    }
    RgbaImage::new(dst_w, dst_h, out)
}
struct Cached {
    target: Shared<Texture>,
    bytes: usize,
    used: u64,
}
/// Device resources are confined to this backend; the core remains dependency-free.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    bindings: bindings::DrawBindings,
    scaling: Option<present::Scaling>,
    pipelines: [wgpu::RenderPipeline; 3],
    cache: HashMap<(u128, u32, u32), Cached>,
    cache_bytes: usize,
    frames: HashMap<u64, Cached>,
    canvases: TexturePool,
    movie_textures: TexturePool,
    white: Shared<Texture>,
    clock: u64,
    frame_start: u64,
    errors: Arc<Mutex<Option<String>>>,
    anti_aliasing: bool,
    render_scale: RenderScale,
    viewport_size: Option<[u32; 2]>,
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
        let mut required_limits = wgpu::Limits::default();
        // The scene shader passes only UVs between stages. Some Metal adapters
        // expose 15 locations, one fewer than the WebGPU default.
        required_limits.max_inter_stage_shader_variables = required_limits
            .max_inter_stage_shader_variables
            .min(adapter.limits().max_inter_stage_shader_variables);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("CMVS GPU"),
                required_limits,
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
                    (cmvs_core::render::scene_render::shader::PARAM_FLOATS * 4) as u64,
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
        let white = Texture::new(
            &device,
            [1, 1],
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            white.texture.as_image_copy(),
            &[255u8; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            white.texture.size(),
        );
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
            canvases: TexturePool::new(
                4,
                128 << 20,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            ),
            movie_textures: TexturePool::new(
                FRAME_TEXTURES,
                128 << 20,
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
            ),
            white,
            clock: 0,
            frame_start: 0,
            errors,
            anti_aliasing: false,
            render_scale: RenderScale::Original,
            viewport_size: None,
            uploaded_bytes: 0,
        };
        renderer.check()?;
        Ok((renderer, adapter))
    }
    /// Enables or disables premultiplied supersampled anti-aliasing for scaled, projected,
    /// rotated, and billboarded sprites. Disabled by default to match original CMVS/CPU parity.
    pub fn set_anti_aliasing(&mut self, enabled: bool) {
        self.anti_aliasing = enabled;
    }
    pub fn anti_aliasing(&self) -> bool {
        self.anti_aliasing
    }
    /// Configures internal GPU scene resolution scaling. Defaults to [`RenderScale::Original`] (1x).
    pub fn set_render_scale(&mut self, scale: RenderScale) {
        self.render_scale = scale;
    }
    pub fn render_scale(&self) -> RenderScale {
        self.render_scale
    }
    /// Updates the physical presentation viewport hint used by [`RenderScale::Auto`].
    pub fn set_viewport_size(&mut self, viewport: Option<[u32; 2]>) {
        self.viewport_size = viewport;
    }
    /// Returns the internal render-target dimensions that [`Self::render`] will use for a
    /// CMVS scene of `logical` dimensions.
    pub fn resolved_render_size(&self, logical: [u32; 2]) -> [u32; 2] {
        self.render_scale.resolve(
            logical,
            self.viewport_size,
            self.device.limits().max_texture_dimension_2d,
        )
    }
    pub fn check(&self) -> Result<()> {
        if let Some(message) = self.errors.lock().expect("GPU error lock").take() {
            return Err(message.into());
        }
        Ok(())
    }
    fn texture(&mut self, image: &RgbaImage) -> Result<Shared<Texture>> {
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
            return Ok(Shared::clone(&cached.target));
        }
        let bytes = image.pixels().len();
        while self.cache_bytes + bytes > CACHE_LIMIT {
            let Some(oldest) = self
                .cache
                .iter()
                .filter(|(_, c)| c.used <= self.frame_start)
                .min_by_key(|(_, c)| c.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.cache_bytes -= self.cache.remove(&oldest).expect("cache entry").bytes;
        }
        let texture = self.upload(image);
        if bytes <= CACHE_LIMIT && self.cache_bytes + bytes <= CACHE_LIMIT {
            self.cache.insert(
                key,
                Cached {
                    target: Shared::clone(&texture),
                    bytes,
                    used: self.clock,
                },
            );
            self.cache_bytes += bytes;
        }
        Ok(texture)
    }
    fn touch(&mut self, image: &RgbaImage) {
        let cached = match image.cache_key() {
            CacheKey::Frame(id) => self.frames.get_mut(&id),
            CacheKey::Content(digest, w, h) => self.cache.get_mut(&(digest, w, h)),
        };
        if let Some(cached) = cached {
            self.clock += 1;
            cached.used = self.clock;
        }
    }
    fn frame_texture(&mut self, id: u64, image: &RgbaImage) -> Shared<Texture> {
        self.clock += 1;
        if let Some(cached) = self.frames.get_mut(&id) {
            cached.used = self.clock;
            return Shared::clone(&cached.target);
        }
        while self.frames.len() >= FRAME_TEXTURES {
            let Some(oldest) = self
                .frames
                .iter()
                .filter(|(_, c)| c.used <= self.frame_start)
                .min_by_key(|(_, c)| c.used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.frames.remove(&oldest);
        }
        let texture = self
            .movie_textures
            .acquire(&self.device, [image.width(), image.height()]);
        self.write_image(&texture, image);
        self.frames.insert(
            id,
            Cached {
                target: Shared::clone(&texture),
                bytes: image.pixels().len(),
                used: self.clock,
            },
        );
        texture
    }
    fn upload(&mut self, image: &RgbaImage) -> Shared<Texture> {
        let texture = Texture::new(
            &self.device,
            [image.width(), image.height()],
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        );
        self.write_image(&texture, image);
        texture
    }
    fn write_image(&mut self, target: &Texture, image: &RgbaImage) {
        let [w, h] = [image.width(), image.height()];
        self.queue.write_texture(
            target.texture.as_image_copy(),
            image.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            target.texture.size(),
        );
        self.uploaded_bytes += image.pixels().len() as u64;
    }
    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<GpuFrame> {
        if let Some(image) = frame.precomposited() {
            return self.render_precomposited(image);
        }
        if frame.cvns_source_exceeds(self.device.limits().max_texture_dimension_2d) {
            return self.render_precomposited(&frame.render_cpu()?);
        }
        self.compose(frame.dimensions(), frame.clear_color(), frame.commands())
    }
    /// Uploads a CPU-composited scene (e.g. CVNS) into a transient pooled texture so
    /// animated frames never hash or evict the static content cache.
    fn render_precomposited(&mut self, image: &RgbaImage) -> Result<GpuFrame> {
        self.check()?;
        let [w, h] = [image.width(), image.height()];
        if w == 0
            || h == 0
            || w > self.device.limits().max_texture_dimension_2d
            || h > self.device.limits().max_texture_dimension_2d
        {
            return Err("invalid GPU canvas size".into());
        }
        self.bindings.clear();
        let target = self.movie_textures.acquire(&self.device, [w, h]);
        self.write_image(&target, image);
        if let Some(scaling) = &mut self.scaling {
            scaling.prune();
        }
        Ok(GpuFrame {
            target,
            device: self.device.clone(),
            queue: self.queue.clone(),
            size: [w, h],
            logical_size: [w, h],
        })
    }
    /// Preserves the source alpha for the desktop checkerboard preview path.
    pub fn preview_image(&mut self, image: &RgbaImage) -> Result<GpuFrame> {
        self.frame_start = self.clock;
        self.bindings.clear();
        let target = self.texture(image)?;
        if let Some(scaling) = &mut self.scaling {
            scaling.prune();
        }
        Ok(GpuFrame {
            target,
            device: self.device.clone(),
            queue: self.queue.clone(),
            size: [image.width(), image.height()],
            logical_size: [image.width(), image.height()],
        })
    }
    /// Scales an image to fit an opaque canvas entirely on the GPU.
    pub fn render_image(
        &mut self,
        image: &RgbaImage,
        size: [u32; 2],
        filter: impl Into<ScaleFilter>,
    ) -> Result<GpuFrame> {
        self.frame_start = self.clock;
        present::scale_image(self, image, size, filter.into())
    }
    fn compose(
        &mut self,
        size: [u32; 2],
        clear: [u8; 4],
        commands: &[Layer<'_>],
    ) -> Result<GpuFrame> {
        self.check()?;
        if size.contains(&0)
            || size
                .iter()
                .any(|&v| v > self.device.limits().max_texture_dimension_2d)
        {
            return Err("invalid GPU canvas size".into());
        }
        self.frame_start = self.clock;
        self.bindings.prepare(&self.device, commands.len())?;
        let has_cvns = commands.iter().any(|c| matches!(c, Layer::Cvns(_)));
        let render_size = if has_cvns {
            size
        } else {
            self.resolved_render_size(size)
        };
        let high_res = render_size != size;
        let target = self.canvases.acquire(&self.device, render_size);
        let dummy = Shared::clone(&self.white);
        let mut draws = Vec::with_capacity(commands.len());
        let mut resolved = Vec::with_capacity(commands.len());
        for command in commands {
            let mut draw = shader_draw(command, size);
            let mut flags = draw.flags();
            if flags & shader_flags::CVNS == 0 {
                draw.params[20] = render_size[0] as f32;
                draw.params[21] = render_size[1] as f32;
                if draw.blend == ShaderBlend::Replace {
                    flags |= FLAG_OPAQUE_FILTER;
                    draw.params[19] = flags as f32;
                }
                if draw.params[3] > 0.0 {
                    if self.anti_aliasing || high_res {
                        draw.params[3] = 2.0;
                    }
                } else if high_res && flags & shader_flags::SOLID == 0 {
                    // Upscale 1:1 CMVS Layer::Image with rect-clamped anti-ringing bicubic
                    // to avoid nearest-neighbor blocks and sprite-sheet edge bleed.
                    draw.params[3] = 3.0;
                }
            }
            for image in draw.textures.into_iter().flatten() {
                self.touch(image);
            }
            resolved.push(draw);
        }
        // Pin every input until submission: an eviction or transient uncached upload
        // must remain alive until the command encoder is submitted.
        let mut inputs = HashMap::new();
        for (index, draw) in resolved.into_iter().enumerate() {
            let mode = match draw.blend {
                ShaderBlend::Replace => 0,
                ShaderBlend::Alpha => 1,
                ShaderBlend::Additive => 2,
            };
            let mut views = [dummy.view.clone(), dummy.view.clone(), dummy.view.clone()];
            for (view, image) in views.iter_mut().zip(draw.textures) {
                if let Some(image) = image {
                    let texture = match inputs.entry(image.cache_key()) {
                        std::collections::hash_map::Entry::Occupied(entry) => {
                            self.touch(image);
                            entry.into_mut()
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(self.texture(image)?)
                        }
                    };
                    *view = texture.view.clone();
                }
            }
            let offset = self.bindings.write(index, draw.used_params());
            let group = self.bindings.group(&self.device, &self.layout, views);
            draws.push((mode, group, offset));
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("CMVS scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
        drop(inputs);
        while self.frames.len() > FRAME_TEXTURES {
            let oldest = self
                .frames
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(id, _)| *id)
                .expect("frame entry");
            self.frames.remove(&oldest);
        }
        self.bindings.retain(
            self.cache
                .values()
                .chain(self.frames.values())
                .map(|c| &c.target.view)
                .chain(std::iter::once(&self.white.view)),
        );
        if let Some(scaling) = &mut self.scaling {
            scaling.prune();
        }
        self.check()?;
        Ok(GpuFrame {
            target,
            device: self.device.clone(),
            queue: self.queue.clone(),
            size: render_size,
            logical_size: size,
        })
    }
}

mod present;
pub use present::Presenter;

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::{
        render::scene::DrawState, render::scene::ObjectImage, render::scene::SceneHost,
        render::scene::SceneObject, render::scene_render::Blend,
        render::scene_render::prepare_scene,
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
            draw.set_blend_mode(mode);
            draw.set_source_rect(rect);
            draw.set_position_raw([2, 1]);
            draw.set_scale([scale; 2]);
            draw.set_rotation_hundredths((angle * 100.0) as i32);
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
        let transition = cmvs_core::render::scene::MaskTransition {
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
                        screen_mask: None,
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
        let fade = cmvs_core::render::fade::Fade {
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
                        screen_mask: None,
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

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn anti_aliasing_removes_dark_fringes_and_reduces_minification_shimmer() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, _) = Renderer::new(&instance, None).unwrap();
        assert!(!gpu.anti_aliasing());

        // 1. Premultiplied alpha eliminates dark halos on bright sprites bordered by [0,0,0,0].
        let edge_sprite = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(2, 1, vec![255, 255, 255, 255, 0, 0, 0, 0]).unwrap(),
        };
        let render_edge = |gpu: &mut Renderer| {
            gpu.compose(
                [4, 1],
                [200, 200, 200, 255],
                &[Layer::Projected {
                    image: &edge_sprite,
                    rect: [0, 0, 2, 1],
                    // x=1 samples uv.x=0.75 and x=2 samples uv.x=1.25 across the [255..255]/[0..0] border.
                    quad: [-0.5, -0.5, 3.5, 0.5],
                    alpha: 255,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap()
        };
        let off = render_edge(&mut gpu);
        // Straight-alpha bilinear mixes the bright border with [0,0,0,0] and dips below background 200.
        let off_min = off
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .min()
            .unwrap();
        assert!(
            off_min < 190,
            "expected straight-alpha dark fringe below 190, got {off_min}"
        );

        gpu.set_anti_aliasing(true);
        assert!(gpu.anti_aliasing());
        let on = render_edge(&mut gpu);
        let on_pixels: Vec<u8> = on
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .collect();
        assert!(
            on_pixels.iter().all(|&v| v >= 200),
            "premultiplied AA must not dip below background 200: {on_pixels:?}"
        );
        // x=1 has 75% white over 200 -> ~241, x=2 has 25% white over 200 -> ~214 (step=1.0 -> linear premul).
        assert!((239..=243).contains(&on_pixels[1]), "got {on_pixels:?}");
        assert!((212..=216).contains(&on_pixels[2]), "got {on_pixels:?}");

        // 1b. Uniform 2D magnification (step = 0.5 < 0.95) uses premultiplied Catmull-Rom AR
        // for sharper edges without overshoot or dark halos.
        let edge_2d = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(
                2,
                2,
                vec![
                    255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0,
                ],
            )
            .unwrap(),
        };
        let bicubic_2d = gpu
            .compose(
                [4, 4],
                [200, 200, 200, 255],
                &[Layer::Projected {
                    image: &edge_2d,
                    rect: [0, 0, 2, 2],
                    quad: [-0.5, -0.5, 3.5, 3.5],
                    alpha: 255,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        let row1: Vec<u8> = (0..4).map(|x| bicubic_2d.pixels()[(4 + x) * 4]).collect();
        assert!(
            row1.iter().all(|&v| v >= 200),
            "Catmull-Rom AR must not dip below background 200: {row1:?}"
        );
        // Catmull-Rom AR sharpens 75%/25% coverage to ~79.7% (244) and ~20.3% (211).
        assert!((242..=246).contains(&row1[1]), "got {row1:?}");
        assert!((209..=213).contains(&row1[2]), "got {row1:?}");

        // 2. RGSS minification reduces sub-pixel vertical float shimmer on thin bright strands.
        let mut strand_pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16usize {
            let alpha = match y {
                6 | 7 => 255u8,
                5 | 8 => 128u8,
                _ => 0u8,
            };
            if alpha > 0 {
                for x in 0..16usize {
                    let i = (y * 16 + x) * 4;
                    strand_pixels[i..i + 4].copy_from_slice(&[255, 255, 255, alpha]);
                }
            }
        }
        let strand = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(16, 16, strand_pixels).unwrap(),
        };
        let shimmer_range = |gpu: &mut Renderer| -> (f32, u8) {
            let mut sums = Vec::new();
            let mut min_channel = 255u8;
            for step in 0..8 {
                let dy = step as f32 * 0.125;
                let frame = gpu
                    .compose(
                        [12, 12],
                        [150, 150, 150, 255],
                        &[Layer::Projected {
                            image: &strand,
                            rect: [0, 0, 16, 16],
                            // 0.5x scale (step = 2.0 texels/px -> RGSS 4-tap path)
                            quad: [1.5, 1.5 + dy, 9.5, 9.5 + dy],
                            alpha: 255,
                            blend: Blend::Alpha,
                        }],
                    )
                    .unwrap()
                    .read()
                    .unwrap();
                let mut total = 0f32;
                for p in frame.pixels().as_chunks::<4>().0 {
                    total += f32::from(p[0]);
                    min_channel = min_channel.min(p[0]);
                }
                sums.push(total);
            }
            let min_s = sums.iter().copied().fold(f32::INFINITY, f32::min);
            let max_s = sums.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            (max_s - min_s, min_channel)
        };
        gpu.set_anti_aliasing(false);
        let (off_range, off_min_c) = shimmer_range(&mut gpu);
        gpu.set_anti_aliasing(true);
        let (on_range, on_min_c) = shimmer_range(&mut gpu);
        assert!(
            off_min_c < 150 && on_min_c >= 150,
            "dark fringe check: off_min={off_min_c} on_min={on_min_c}"
        );
        assert!(
            on_range < off_range * 0.35,
            "expected AA to sharply reduce vertical shimmer: off={off_range} on={on_range}"
        );

        // 3. Strong minification (0.25x, step = 4.0 > 2.5 -> 4x4 16-tap path) averages checkerboard cleanly.
        let mut checker_pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16usize {
            for x in 0..16usize {
                let i = (y * 16 + x) * 4;
                let color = if (x + y) % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                checker_pixels[i..i + 4].copy_from_slice(&color);
            }
        }
        let checker = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: false,
            surface: false,
            pixels: RgbaImage::new(16, 16, checker_pixels).unwrap(),
        };
        let minified = gpu
            .compose(
                [4, 4],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &checker,
                    rect: [0, 0, 16, 16],
                    quad: [-0.5, -0.5, 3.5, 3.5],
                    alpha: 255,
                    blend: Blend::Replace,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        for p in minified.pixels().as_chunks::<4>().0 {
            assert!(
                (122..=134).contains(&p[0]) && p[1] == 0 && (122..=134).contains(&p[2]),
                "expected averaged purple around [128, 0, 128], got {p:?}"
            );
        }

        // 4. 1:1 Layer::Image stays bit-identical with AA on or off, and Additive/BlackAlpha/Rotated work cleanly.
        let unscaled_on = gpu
            .compose(
                [2, 1],
                [10, 20, 30, 255],
                &[Layer::Image {
                    image: &edge_sprite,
                    rect: [0, 0, 2, 1],
                    corner: [0, 0],
                    alpha: 255,
                    mask: None,
                    screen_mask: None,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        gpu.set_anti_aliasing(false);
        let unscaled_off = gpu
            .compose(
                [2, 1],
                [10, 20, 30, 255],
                &[Layer::Image {
                    image: &edge_sprite,
                    rect: [0, 0, 2, 1],
                    corner: [0, 0],
                    alpha: 255,
                    mask: None,
                    screen_mask: None,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(unscaled_on, unscaled_off);

        gpu.set_anti_aliasing(true);
        let additive = gpu
            .compose(
                [4, 1],
                [100, 100, 100, 255],
                &[Layer::Projected {
                    image: &edge_sprite,
                    rect: [0, 0, 2, 1],
                    quad: [-0.5, -0.5, 3.5, 0.5],
                    alpha: 255,
                    blend: Blend::Additive,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        // x=2 has 25% premultiplied coverage of white (255) added onto 100 -> ~164
        // (straight-alpha bilinear would lose brightness to ~116).
        let add_x2 = additive.pixels()[2 * 4];
        assert!(
            (160..=168).contains(&add_x2),
            "expected premultiplied additive ~164 at x=2, got {add_x2}"
        );

        // 5. Blend::Replace ignores source alpha even when texels carry alpha=0.
        let opaque_zero_alpha = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: false,
            surface: false,
            pixels: RgbaImage::new(
                2,
                2,
                vec![
                    255, 0, 0, 0, 0, 0, 255, 255, //
                    255, 0, 0, 0, 0, 0, 255, 255,
                ],
            )
            .unwrap(),
        };
        let replace_min = gpu
            .compose(
                [1, 1],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &opaque_zero_alpha,
                    rect: [0, 0, 2, 2],
                    quad: [-0.5, -0.5, 0.5, 0.5],
                    alpha: 255,
                    blend: Blend::Replace,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        let px = &replace_min.pixels()[..4];
        assert!(
            (122..=134).contains(&px[0]) && px[1] == 0 && (122..=134).contains(&px[2]),
            "Blend::Replace must weight alpha=0 red equally with alpha=255 blue, got {px:?}"
        );
        let replace_mag = gpu
            .compose(
                [4, 4],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &opaque_zero_alpha,
                    rect: [0, 0, 2, 2],
                    quad: [-0.5, -0.5, 3.5, 3.5],
                    alpha: 255,
                    blend: Blend::Replace,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        let left_px = &replace_mag.pixels()[..4];
        assert!(
            left_px[0] > 240 && left_px[1] == 0 && left_px[2] < 15,
            "Blend::Replace must keep alpha=0 red bright under bicubic AA, got {left_px:?}"
        );

        // 6. Cropped atlas cells clamp AA samples to their source rect (RGSS and bicubic),
        // while world billboards retain wrapped sampling.
        let atlas_pixels: Vec<u8> = (0..2)
            .flat_map(|_| {
                [
                    [255, 255, 255, 255],
                    [255, 255, 255, 255],
                    [255, 0, 0, 255],
                    [255, 0, 0, 255],
                ]
                .into_iter()
                .flatten()
            })
            .collect();
        let atlas = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(4, 2, atlas_pixels).unwrap(),
        };
        let cropped_min = gpu
            .compose(
                [1, 1],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &atlas,
                    rect: [0, 0, 2, 2],
                    quad: [-0.5, -0.5, 0.5, 0.5],
                    alpha: 255,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(
            &cropped_min.pixels()[..4],
            &[255, 255, 255, 255],
            "RGSS minification of cropped atlas cell must not bleed adjacent red cell"
        );
        let cropped_mag = gpu
            .compose(
                [4, 4],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &atlas,
                    rect: [0, 0, 2, 2],
                    quad: [-0.5, -0.5, 3.5, 3.5],
                    alpha: 255,
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read()
            .unwrap();
        for p in cropped_mag.pixels().as_chunks::<4>().0 {
            assert_eq!(
                p,
                &[255, 255, 255, 255],
                "bicubic magnification of cropped atlas cell must not bleed adjacent red cell"
            );
        }
        let wrapped = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap(),
        };
        let billboard = gpu
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
        let wrap_edge = &billboard.pixels()[..4];
        assert!(
            (110..=145).contains(&wrap_edge[0])
                && wrap_edge[1] == 0
                && (110..=145).contains(&wrap_edge[2]),
            "Billboard AA must preserve wrapped edge blending, got {wrap_edge:?}"
        );
        gpu.check().unwrap();
    }

    #[test]
    fn streaming_box_area_downsampler_handles_strided_rows_and_fractional_scales() {
        // 6x6 source in a 256-byte-strided buffer (matching GPU readback row pitch):
        // left 3 columns red, right 3 columns blue; top 3 rows opaque, bottom 3 rows transparent.
        let stride = 256usize;
        let mut buf = vec![0xAAu8; 6 * stride];
        for y in 0..6usize {
            for x in 0..6usize {
                let px = if y < 3 {
                    if x < 3 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }
                } else {
                    [0, 255, 0, 0]
                };
                buf[y * stride + x * 4..][..4].copy_from_slice(&px);
            }
        }
        // 1.5x fractional downsample: 6x6 -> 4x4 (shares boundary row y=1 between dy=0 and dy=1).
        let frac = downsample_area_rows(&buf, stride, [6, 6], [4, 4]).unwrap();
        assert_eq!([frac.width(), frac.height()], [4, 4]);
        let at = |x: usize, y: usize| &frac.pixels()[(y * 4 + x) * 4..][..4];
        assert_eq!(at(0, 0), &[255, 0, 0, 255]);
        assert_eq!(at(1, 0), &[255, 0, 0, 255]);
        assert_eq!(at(2, 0), &[0, 0, 255, 255]);
        assert_eq!(at(3, 0), &[0, 0, 255, 255]);
        // Bottom two rows cover y in [3, 6) which is completely transparent.
        assert_eq!(at(0, 2), &[0, 0, 0, 0]);
        assert_eq!(at(3, 3), &[0, 0, 0, 0]);

        // 2x exact downsample: 6x6 -> 3x3 (middle column mixes red+blue, middle row mixes opaque+transparent).
        let half = downsample_area_rows(&buf, stride, [6, 6], [3, 3]).unwrap();
        assert_eq!([half.width(), half.height()], [3, 3]);
        let at2 = |x: usize, y: usize| &half.pixels()[(y * 3 + x) * 4..][..4];
        assert_eq!(at2(0, 0), &[255, 0, 0, 255]);
        assert_eq!(at2(1, 0), &[128, 0, 128, 255]);
        assert_eq!(at2(2, 0), &[0, 0, 255, 255]);
        // Middle row has 50% opaque red (top) and 50% transparent green (bottom):
        // premultiplied downsampling must keep pure red at ~50% alpha, with zero green bleed.
        let mid_left = at2(0, 1);
        assert_eq!(mid_left[0], 255);
        assert_eq!(mid_left[1], 0);
        assert!((127..=128).contains(&mid_left[3]), "got {mid_left:?}");
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn high_res_internal_rendering_preserves_source_detail_and_logical_readback() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, _) = Renderer::new(&instance, None).unwrap();
        assert_eq!(gpu.render_scale(), RenderScale::Original);

        // 1. High-res 16x4 alternating red/green stripes drawn at 0.5x into an 8x2 logical canvas:
        // 2x internal resolution (16x4) preserves every stripe, while read() returns 8x2 logical.
        let stripe_pixels: Vec<u8> = (0..4)
            .flat_map(|_| {
                (0..16).flat_map(|x| {
                    if x % 2 == 0 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 255, 0, 255]
                    }
                })
            })
            .collect();
        let stripes = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: false,
            surface: false,
            pixels: RgbaImage::new(16, 4, stripe_pixels).unwrap(),
        };
        gpu.set_render_scale(RenderScale::X2);
        let frame = gpu
            .compose(
                [8, 2],
                [0, 0, 0, 255],
                &[Layer::Projected {
                    image: &stripes,
                    rect: [0, 0, 16, 4],
                    quad: [-0.5, -0.5, 7.5, 1.5],
                    alpha: 255,
                    blend: Blend::Replace,
                }],
            )
            .unwrap();
        assert_eq!(frame.dimensions(), [8, 2]);
        assert_eq!(frame.render_dimensions(), [16, 4]);
        let native = frame.read_native().unwrap();
        assert_eq!([native.width(), native.height()], [16, 4]);
        for y in 0..4usize {
            for x in 0..16usize {
                let p = &native.pixels()[(y * 16 + x) * 4..][..4];
                if x % 2 == 0 {
                    assert!(
                        p[0] > 240 && p[1] < 15,
                        "expected crisp red stripe at ({x},{y}), got {p:?}"
                    );
                } else {
                    assert!(
                        p[1] > 240 && p[0] < 15,
                        "expected crisp green stripe at ({x},{y}), got {p:?}"
                    );
                }
            }
        }
        let logical = frame.read().unwrap();
        assert_eq!([logical.width(), logical.height()], [8, 2]);
        for p in logical.pixels().as_chunks::<4>().0 {
            assert!(
                (120..=135).contains(&p[0]) && (120..=135).contains(&p[1]),
                "expected box-downsampled yellow ~[128,128,0], got {p:?}"
            );
        }

        // 2. Cropped Layer::Image upscaling clamps to its source rect with zero sprite-sheet bleed.
        let atlas_pixels: Vec<u8> = (0..2)
            .flat_map(|_| {
                [
                    [255, 255, 255, 255],
                    [255, 255, 255, 255],
                    [255, 0, 0, 255],
                    [255, 0, 0, 255],
                ]
                .into_iter()
                .flatten()
            })
            .collect();
        let atlas = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: false,
            surface: false,
            pixels: RgbaImage::new(4, 2, atlas_pixels).unwrap(),
        };
        let cropped = gpu
            .compose(
                [2, 2],
                [0, 0, 0, 255],
                &[Layer::Image {
                    image: &atlas,
                    rect: [0, 0, 2, 2],
                    corner: [0, 0],
                    alpha: 255,
                    mask: None,
                    screen_mask: None,
                    blend: Blend::Replace,
                }],
            )
            .unwrap()
            .read_native()
            .unwrap();
        assert_eq!([cropped.width(), cropped.height()], [4, 4]);
        for p in cropped.pixels().as_chunks::<4>().0 {
            assert_eq!(p, &[255, 255, 255, 255], "cropped UI must not bleed red");
        }

        // 3. Screen-space second texture mask scales to the full high-res render target.
        let white = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(8, 4, [255, 255, 255, 255].repeat(32)).unwrap(),
        };
        let mask_pixels: Vec<u8> = (0..4)
            .flat_map(|_| {
                (0..8).flat_map(|x| {
                    let g = if x < 4 { 255 } else { 0 };
                    [0, g, 0, 255]
                })
            })
            .collect();
        let screen_mask = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(8, 4, mask_pixels).unwrap(),
        };
        let masked = gpu
            .compose(
                [8, 4],
                [0, 0, 0, 255],
                &[Layer::Image {
                    image: &white,
                    rect: [0, 0, 8, 4],
                    corner: [0, 0],
                    alpha: 255,
                    mask: None,
                    screen_mask: Some(&screen_mask),
                    blend: Blend::Alpha,
                }],
            )
            .unwrap()
            .read_native()
            .unwrap();
        assert_eq!([masked.width(), masked.height()], [16, 8]);
        for y in 0..8usize {
            for x in 0..6usize {
                assert!(
                    masked.pixels()[(y * 16 + x) * 4] > 240,
                    "left screen_mask region must stay white at ({x},{y})"
                );
            }
            for x in 10..16usize {
                assert!(
                    masked.pixels()[(y * 16 + x) * 4] < 15,
                    "right screen_mask region must stay black at ({x},{y})"
                );
            }
        }

        // 4. RenderScale::Auto follows viewport_size cleanly and clamps to [1x .. 3x].
        gpu.set_render_scale(RenderScale::Auto);
        gpu.set_viewport_size(Some([16, 8]));
        assert_eq!(gpu.resolved_render_size([8, 4]), [16, 8]);
        gpu.set_viewport_size(Some([4, 2]));
        assert_eq!(gpu.resolved_render_size([8, 4]), [8, 4]);
        gpu.set_viewport_size(Some([40, 20]));
        assert_eq!(gpu.resolved_render_size([8, 4]), [24, 12]);
        gpu.check().unwrap();
    }
}

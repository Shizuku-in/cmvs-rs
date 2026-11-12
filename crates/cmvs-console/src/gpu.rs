//! Execution of the shared scene queue (`cmvs_core::scene_render::shader`) on a console
//! GPU, with the content-keyed texture cache policy of cmvs-wgpu. Glyph rasterization
//! and movie decoding stay on the CPU; the device composites. Presenting and readback
//! are device specific and live with each player.
use cmvs_core::{
    raster::{CacheKey, RgbaImage},
    scene_render::{
        RenderFrame,
        shader::{ShaderBlend, shader_draw},
    },
};
use cmvs_native::{GpuDevice, GpuError, GpuTexture};
use std::{collections::HashMap, error::Error, io::ErrorKind, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// Movie frames live apart from the content cache so replaced frames never evict
/// static images. Each player shows one frame (three scene slots and the full-screen
/// player), which leaves room for images that stopped players retain.
const FRAME_TEXTURES: usize = 8;

struct Cached {
    texture: GpuTexture,
    bytes: usize,
    used: u64,
}

pub struct Compositor<D> {
    device: D,
    /// Retained texture bytes; not a bound on total GPU memory.
    cache_limit: usize,
    cache: HashMap<(u128, u32, u32), Cached>,
    cache_bytes: usize,
    frames: HashMap<u64, Cached>,
    clock: u64,
    /// Clock when the current frame began; textures used since are never evicted.
    frame_start: u64,
    white: GpuTexture,
    /// Canvas size once a pass has begun, and once it has been presented.
    canvas: Option<[u32; 2]>,
    presented: Option<[u32; 2]>,
}

impl<D: GpuDevice> Compositor<D> {
    pub fn new(mut device: D, cache_limit: usize) -> Result<Self> {
        let white = device.create_texture(1, 1, &[255; 4])?;
        Ok(Self {
            device,
            cache_limit,
            cache: HashMap::new(),
            cache_bytes: 0,
            frames: HashMap::new(),
            clock: 0,
            frame_start: 0,
            white,
            canvas: None,
            presented: None,
        })
    }

    pub fn device(&mut self) -> &mut D {
        &mut self.device
    }

    /// Releases every cached texture, e.g. before another game starts.
    pub fn clear_cache(&mut self) {
        let cached = self.cache.drain().map(|(_, c)| c);
        for cached in cached.chain(self.frames.drain().map(|(_, c)| c)) {
            self.device.release_texture(cached.texture);
        }
        self.cache_bytes = 0;
    }

    pub fn begin_frame(&mut self) -> Result<()> {
        self.device.begin_frame()?;
        self.frame_start = self.clock;
        Ok(())
    }

    pub fn begin_canvas(&mut self, size: [u32; 2], clear: [u8; 4]) -> Result<()> {
        self.canvas = None;
        self.presented = None;
        self.device.begin_canvas(size, clear)?;
        self.canvas = Some(size);
        Ok(())
    }

    /// The size of the canvas being drawn, once a pass has begun.
    pub fn canvas(&self) -> Option<[u32; 2]> {
        self.canvas
    }

    /// Records that the device showed the current canvas.
    pub fn mark_presented(&mut self) {
        self.presented = self.canvas;
    }

    /// The canvas size the device last showed, the source of readback.
    pub fn presented(&self) -> Option<[u32; 2]> {
        self.presented
    }

    /// Something other than the canvas is on screen now.
    pub fn forget_presented(&mut self) {
        self.presented = None;
    }

    /// Releases the least recently used movie frame not needed by the current frame.
    fn evict_frame(&mut self) -> bool {
        let Some(id) = self
            .frames
            .iter()
            .filter(|(_, c)| c.used <= self.frame_start)
            .min_by_key(|(_, c)| c.used)
            .map(|(id, _)| *id)
        else {
            return false;
        };
        let cached = self.frames.remove(&id).expect("frame entry");
        self.device.release_texture(cached.texture);
        true
    }

    /// Releases the least recently used texture not needed by the current frame.
    fn evict_one(&mut self) -> bool {
        let Some(key) = self
            .cache
            .iter()
            .filter(|(_, c)| c.used <= self.frame_start)
            .min_by_key(|(_, c)| c.used)
            .map(|(key, _)| *key)
        else {
            return false;
        };
        let cached = self.cache.remove(&key).expect("cache entry");
        self.cache_bytes -= cached.bytes;
        self.device.release_texture(cached.texture);
        true
    }

    /// Uploads, evicting while the device reports exhausted texture memory. The caller
    /// owns the texture.
    pub fn upload(&mut self, image: &RgbaImage) -> Result<GpuTexture> {
        loop {
            match self
                .device
                .create_texture(image.width(), image.height(), image.pixels())
            {
                Err(GpuError::OutOfMemory) if self.evict_frame() || self.evict_one() => {}
                result => return Ok(result?),
            }
        }
    }

    /// Marks the image's texture, if cached, as used by the current frame.
    fn touch(&mut self, image: &RgbaImage) {
        let cached = match image.cache_key() {
            CacheKey::Frame(id) => self.frames.get_mut(&id),
            CacheKey::Content(digest, width, height) => {
                self.cache.get_mut(&(digest, width, height))
            }
        };
        if let Some(cached) = cached {
            self.clock += 1;
            cached.used = self.clock;
        }
    }

    /// Images above the cache limit are uploaded for this frame only (`transient`).
    fn texture(
        &mut self,
        image: &RgbaImage,
        transient: &mut Vec<GpuTexture>,
    ) -> Result<GpuTexture> {
        let key = match image.cache_key() {
            CacheKey::Frame(id) => return self.frame_texture(id, image),
            CacheKey::Content(digest, width, height) => (digest, width, height),
        };
        self.clock += 1;
        if let Some(cached) = self.cache.get_mut(&key) {
            cached.used = self.clock;
            return Ok(cached.texture);
        }
        let bytes = image.pixels().len();
        while self.cache_bytes + bytes > self.cache_limit && self.evict_one() {}
        let texture = self.upload(image)?;
        if bytes <= self.cache_limit {
            self.cache.insert(
                key,
                Cached {
                    texture,
                    bytes,
                    used: self.clock,
                },
            );
            self.cache_bytes += bytes;
        } else {
            transient.push(texture);
        }
        Ok(texture)
    }

    /// A decoded movie frame's texture, kept in the frame cache so a frame shown again
    /// (the display outpaces the movie) is not uploaded again.
    pub fn frame_texture(&mut self, id: u64, image: &RgbaImage) -> Result<GpuTexture> {
        self.clock += 1;
        if let Some(cached) = self.frames.get_mut(&id) {
            cached.used = self.clock;
            return Ok(cached.texture);
        }
        if self.frames.len() >= FRAME_TEXTURES {
            self.evict_frame();
        }
        let texture = self.upload(image)?;
        let bytes = image.pixels().len();
        let used = self.clock;
        self.frames.insert(
            id,
            Cached {
                texture,
                bytes,
                used,
            },
        );
        Ok(texture)
    }

    /// Composites a validated frame onto a fresh canvas; the device presents it next.
    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<()> {
        self.begin_frame()?;
        let size = frame.dimensions();
        let mut transient = Vec::new();
        let result = self.compose(frame, size, &mut transient);
        for texture in transient {
            self.device.release_texture(texture);
        }
        result
    }

    fn compose(
        &mut self,
        frame: &RenderFrame<'_>,
        size: [u32; 2],
        transient: &mut Vec<GpuTexture>,
    ) -> Result<()> {
        // Every upload precedes the pass, so the pass needs a single barrier.
        let queue: Vec<_> = frame
            .commands()
            .iter()
            .map(|command| shader_draw(command, size))
            .collect();
        // Cached textures of the whole frame count as used before any upload: making
        // room for an early image must not evict one that a later draw needs.
        for image in queue.iter().flat_map(|draw| draw.textures).flatten() {
            self.touch(image);
        }
        let mut draws = Vec::with_capacity(queue.len());
        for draw in queue {
            let mut textures = [self.white; 3];
            for (texture, image) in textures.iter_mut().zip(draw.textures) {
                if let Some(image) = image {
                    *texture = self.texture(image, transient)?;
                }
            }
            draws.push((draw, textures));
        }
        self.begin_canvas(size, frame.clear_color())?;
        for (draw, textures) in &draws {
            let blend = match draw.blend {
                ShaderBlend::Replace => 0,
                ShaderBlend::Alpha => 1,
                ShaderBlend::Additive => 2,
            };
            self.device.draw(blend, draw.used_params(), *textures)?;
        }
        Ok(())
    }
}

/// `renderer.txt` in `base` holds `gpu` (the default when absent) or `cpu`.
pub fn use_gpu(base: &Path) -> Result<bool> {
    match std::fs::read_to_string(base.join("renderer.txt")) {
        Ok(text) => parse_renderer(Some(&text)),
        Err(error) if error.kind() == ErrorKind::NotFound => parse_renderer(None),
        Err(error) => Err(error.into()),
    }
}

fn parse_renderer(setting: Option<&str>) -> Result<bool> {
    match setting.map(str::trim) {
        None | Some("gpu") => Ok(true),
        Some("cpu") => Ok(false),
        Some(other) => Err(format!("renderer.txt must be gpu or cpu, not {other:?}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::scene::{DrawState, ObjectImage, SceneHost, SceneObject};

    /// Records calls; `capacity` live textures fit before uploads report exhaustion.
    #[derive(Default)]
    struct FakeDevice {
        capacity: usize,
        next: u32,
        live: Vec<u32>,
        calls: Vec<String>,
    }
    impl GpuDevice for FakeDevice {
        fn begin_frame(&mut self) -> std::result::Result<(), GpuError> {
            self.calls.push("frame".into());
            Ok(())
        }
        fn create_texture(
            &mut self,
            width: u32,
            height: u32,
            _: &[u8],
        ) -> std::result::Result<GpuTexture, GpuError> {
            if self.live.len() >= self.capacity {
                return Err(GpuError::OutOfMemory);
            }
            self.next += 1;
            self.live.push(self.next);
            self.calls
                .push(format!("upload {} {width}x{height}", self.next));
            Ok(GpuTexture::from_raw(self.next))
        }
        fn release_texture(&mut self, texture: GpuTexture) {
            self.live.retain(|&id| id != texture.raw());
            self.calls.push(format!("release {}", texture.raw()));
        }
        fn begin_canvas(
            &mut self,
            size: [u32; 2],
            clear: [u8; 4],
        ) -> std::result::Result<(), GpuError> {
            self.calls.push(format!("canvas {size:?} {clear:?}"));
            Ok(())
        }
        fn draw(
            &mut self,
            blend: u32,
            params: &[f32],
            textures: [GpuTexture; 3],
        ) -> std::result::Result<(), GpuError> {
            let ids = textures.map(GpuTexture::raw);
            self.calls
                .push(format!("draw {blend} {} {ids:?}", params.len()));
            Ok(())
        }
    }

    /// Opaque 2x2 alpha-blended images side by side, one per color.
    fn objects(colors: &[u8]) -> Vec<SceneObject> {
        colors
            .iter()
            .enumerate()
            .map(|(i, &color)| {
                let mut draw = DrawState::default();
                draw.words[1] = 1;
                draw.words[2..6].copy_from_slice(&[0, 0, 2, 2]);
                draw.words[8..10].copy_from_slice(&[i as u32 * 2, 0]);
                draw.words[18..20].fill(1.0f32.to_bits());
                SceneObject {
                    image: Some(ObjectImage {
                        name: Vec::new(),
                        filter: None,
                        alpha: true,
                        surface: false,
                        pixels: RgbaImage::new(2, 2, vec![color; 16]).unwrap(),
                    }),
                    draw: Some(draw),
                    ..SceneObject::default()
                }
            })
            .collect()
    }

    fn render(gpu: &mut Compositor<FakeDevice>, colors: &[u8]) -> Result<()> {
        let scene = SceneHost::default();
        let objects = objects(colors);
        let extra: Vec<_> = objects.iter().collect();
        let frame = cmvs_core::scene_render::prepare_scene(&scene, None, &extra, 8, 4)?;
        gpu.render(&frame)
    }

    #[test]
    fn frames_upload_once_then_draw_from_the_cache() {
        let device = FakeDevice {
            capacity: 16,
            ..FakeDevice::default()
        };
        let mut gpu = Compositor::new(device, 1 << 20).unwrap();
        for _ in 0..2 {
            render(&mut gpu, &[10]).unwrap();
        }
        assert_eq!(gpu.canvas(), Some([8, 4]));
        assert_eq!(gpu.presented(), None);
        gpu.mark_presented();
        assert_eq!(gpu.presented(), Some([8, 4]));
        let calls = &gpu.device().calls;
        let uploads = calls.iter().filter(|c| c.starts_with("upload")).count();
        // The white placeholder and the object image, uploaded once across two frames.
        assert_eq!(uploads, 2, "{calls:?}");
        assert_eq!(
            calls.iter().filter(|c| *c == "draw 1 24 [2, 1, 1]").count(),
            2
        );
    }

    #[test]
    fn exhausted_memory_evicts_textures_from_earlier_frames_only() {
        // The placeholder plus one image fit.
        let device = FakeDevice {
            capacity: 2,
            ..FakeDevice::default()
        };
        let mut gpu = Compositor::new(device, 1 << 20).unwrap();
        render(&mut gpu, &[10]).unwrap();
        render(&mut gpu, &[20]).unwrap();
        assert!(gpu.device().calls.contains(&"release 2".to_owned()));
        // Two images in one frame cannot both fit: nothing of this frame is evicted.
        let error = render(&mut gpu, &[30, 40]).unwrap_err();
        assert_eq!(error.to_string(), "GPU texture memory exhausted");
    }

    #[test]
    fn a_new_image_does_not_evict_textures_the_same_frame_still_draws() {
        let device = FakeDevice {
            capacity: 16,
            ..FakeDevice::default()
        };
        // Two 2x2 images fill the cache.
        let mut gpu = Compositor::new(device, 32).unwrap();
        render(&mut gpu, &[10, 20]).unwrap();
        // The new image draws first; both cached ones follow in the same frame.
        for _ in 0..2 {
            render(&mut gpu, &[30, 10, 20]).unwrap();
        }
        let calls = &gpu.device().calls;
        let uploads = calls.iter().filter(|c| c.starts_with("upload")).count();
        // The placeholder and each image once.
        assert_eq!(uploads, 4, "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("release")), "{calls:?}");
        // Once the frame no longer needs them, they make room again.
        render(&mut gpu, &[40]).unwrap();
        assert!(gpu.device().calls.iter().any(|c| c.starts_with("release")));
    }

    #[test]
    fn renderer_setting_defaults_to_gpu_and_rejects_unknown_names() {
        assert!(parse_renderer(None).unwrap());
        assert!(parse_renderer(Some("gpu\n")).unwrap());
        assert!(!parse_renderer(Some(" cpu\r\n")).unwrap());
        assert!(parse_renderer(Some("vulkan")).is_err());
    }
}

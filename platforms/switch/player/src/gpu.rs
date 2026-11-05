//! deko3d execution of the shared scene queue (`cmvs_core::scene_render::shader`), with
//! the same content-keyed texture cache policy as cmvs-wgpu. Glyph rasterization and
//! movie decoding stay on the CPU; compositing, movie scaling and presentation run on
//! the GPU. Readback happens only when the engine requests a device frame.
use crate::{SCREEN, sys::SwitchGpu};
use cmvs_core::{
    raster::{CacheKey, RgbaImage},
    scene_render::{
        Blend, Layer, RenderFrame,
        shader::{BASE_FLOATS, ShaderBlend, shader_draw},
    },
    viewport::Viewport,
};
use cmvs_native::{GpuError, GpuTexture};
use std::{collections::HashMap, error::Error};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// Retained texture bytes, as in cmvs-wgpu; not a bound on total GPU memory.
const CACHE_LIMIT: usize = 128 * 1024 * 1024;
/// Movie frames live apart from the content cache so replaced frames never evict
/// static images. Each player shows one frame (three scene slots and the full-screen
/// player), which leaves room for images that stopped players retain.
const FRAME_TEXTURES: usize = 8;

struct Cached {
    texture: GpuTexture,
    bytes: usize,
    used: u64,
}

pub struct GpuRenderer {
    gpu: SwitchGpu,
    cache: HashMap<([u8; 32], u32, u32), Cached>,
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

impl GpuRenderer {
    pub fn new(mut gpu: SwitchGpu) -> Result<Self> {
        let white = gpu.create_texture(1, 1, &[255; 4])?;
        Ok(Self {
            gpu,
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

    /// Releases every cached texture, e.g. before another game starts.
    pub fn clear_cache(&mut self) {
        let cached = self.cache.drain().map(|(_, c)| c);
        for cached in cached.chain(self.frames.drain().map(|(_, c)| c)) {
            self.gpu.release_texture(cached.texture);
        }
        self.cache_bytes = 0;
    }

    fn begin_frame(&mut self) -> Result<()> {
        self.gpu.begin_frame()?;
        self.frame_start = self.clock;
        Ok(())
    }

    fn begin_canvas(&mut self, size: [u32; 2], clear: [u8; 4]) -> Result<()> {
        self.canvas = None;
        self.presented = None;
        self.gpu.begin_canvas(size, clear)?;
        self.canvas = Some(size);
        Ok(())
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
        self.gpu.release_texture(cached.texture);
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
        self.gpu.release_texture(cached.texture);
        true
    }

    /// Uploads, evicting while the device reports exhausted texture memory.
    fn upload(&mut self, image: &RgbaImage) -> Result<GpuTexture> {
        loop {
            match self
                .gpu
                .create_texture(image.width(), image.height(), image.pixels())
            {
                Err(GpuError::OutOfMemory) if self.evict_frame() || self.evict_one() => {}
                result => return Ok(result?),
            }
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
        while self.cache_bytes + bytes > CACHE_LIMIT && self.evict_one() {}
        let texture = self.upload(image)?;
        if bytes <= CACHE_LIMIT {
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

    fn frame_texture(&mut self, id: u64, image: &RgbaImage) -> Result<GpuTexture> {
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

    pub fn render(&mut self, frame: &RenderFrame<'_>) -> Result<()> {
        self.begin_frame()?;
        let size = frame.dimensions();
        let mut transient = Vec::new();
        let result = self.compose(frame, size, &mut transient);
        for texture in transient {
            self.gpu.release_texture(texture);
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
        let mut draws = Vec::with_capacity(frame.commands().len());
        for command in frame.commands() {
            let draw = shader_draw(command, size);
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
            self.gpu.draw(blend, draw.used_params(), *textures)?;
        }
        Ok(())
    }

    /// Fits an image (a decoded movie frame, a host screen) onto an opaque canvas of
    /// `size`. A movie frame stays in the frame cache, so a frame shown again (the
    /// display outpaces the movie) is not uploaded again. Host screens bypass the
    /// cache and its digest.
    pub fn render_image(&mut self, image: &RgbaImage, size: [u32; 2], linear: bool) -> Result<()> {
        self.begin_frame()?;
        let source = [image.width(), image.height()];
        let view = Viewport::new(source, size, 1)?;
        let (texture, transient) = match image.frame_id() {
            Some(id) => (self.frame_texture(id, image)?, false),
            None => (self.upload(image)?, true),
        };
        let result = self
            .begin_canvas(size, [0, 0, 0, 255])
            .and_then(|()| Ok(self.gpu.scale(texture, view_params(view, source, linear))?));
        if transient {
            self.gpu.release_texture(texture);
        }
        result
    }

    /// Shows the canvas with the engine's display `mode`; `cursor` is a screen
    /// position for the host crosshair, which device readback never includes.
    pub fn present(&mut self, mode: u32, linear: bool, cursor: Option<[f64; 2]>) -> Result<()> {
        let size = self.canvas.ok_or("no GPU canvas to present")?;
        let view = Viewport::new(size, SCREEN, mode)?;
        let overlays = cursor.map(cursor_overlays).unwrap_or_default();
        self.gpu
            .present(view_params(view, size, linear), &overlays)?;
        self.presented = Some(size);
        Ok(())
    }

    /// The last presented logical canvas.
    pub fn read(&mut self) -> Result<RgbaImage> {
        let size = self.presented.ok_or("no completed frame")?;
        let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        self.gpu.read(size, &mut pixels)?;
        Ok(RgbaImage::new(size[0], size[1], pixels)?)
    }
}

fn view_params(view: Viewport, source: [u32; 2], linear: bool) -> [f32; 8] {
    [
        view.left as f32,
        view.top as f32,
        view.width as f32,
        view.height as f32,
        source[0] as f32,
        source[1] as f32,
        f32::from(u8::from(linear)),
        0.0,
    ]
}

/// The software host's 11-pixel white crosshair as two solid scene quads.
fn cursor_overlays(at: [f64; 2]) -> Vec<[f32; BASE_FLOATS]> {
    let [x, y] = at.map(|v| v as i64);
    [[x - 5, y, 11, 1], [x, y - 5, 1, 11]]
        .map(|rect| {
            let draw = shader_draw(
                &Layer::Solid {
                    rect,
                    color: [255; 3],
                    alpha: 255,
                    blend: Blend::Replace,
                },
                SCREEN,
            );
            let mut params = [0.0; BASE_FLOATS];
            params.copy_from_slice(&draw.params[..BASE_FLOATS]);
            params
        })
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_matches_the_software_crosshair() {
        let [horizontal, vertical] = cursor_overlays([100.7, 50.2])[..] else {
            panic!("two quads");
        };
        // Strip corners TL, TR, BL, BR in screen pixels.
        assert_eq!(
            &horizontal[8..16],
            &[95.0, 50.0, 106.0, 50.0, 95.0, 51.0, 106.0, 51.0]
        );
        assert_eq!(
            &vertical[8..16],
            &[100.0, 45.0, 101.0, 45.0, 100.0, 56.0, 101.0, 56.0]
        );
        assert_eq!(&horizontal[..3], &[1280.0, 720.0, 255.0]);
    }
}

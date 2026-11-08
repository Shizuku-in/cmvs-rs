//! Bounded texture pools. CPU leases protect snapshots and draw inputs; all GPU
//! writes and reads use one queue, so reuse needs no per-frame device idle wait.
use super::*;

// Browser GPU handles are confined to their Worker; native frames can cross threads.
#[cfg(target_arch = "wasm32")]
pub(super) use std::rc::{Rc as Shared, Weak};
#[cfg(not(target_arch = "wasm32"))]
pub(super) use std::sync::{Arc as Shared, Weak};

pub(super) struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}
impl Texture {
    pub fn new(device: &wgpu::Device, size: [u32; 2], usage: wgpu::TextureUsages) -> Shared<Self> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CMVS texture"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Shared::new(Self { texture, view })
    }
    fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }
    fn bytes(&self) -> u64 {
        u64::from(self.texture.width()) * u64::from(self.texture.height()) * 4
    }
}

pub(super) struct TexturePool {
    // Oldest use first. Pool ownership alone means no CPU reader/draw still uses it.
    textures: Vec<Shared<Texture>>,
    bytes: u64,
    max_bytes: u64,
    max_entries: usize,
    usage: wgpu::TextureUsages,
    pub created: u64,
}
impl TexturePool {
    pub fn new(max_entries: usize, max_bytes: u64, usage: wgpu::TextureUsages) -> Self {
        Self {
            textures: Vec::new(),
            bytes: 0,
            max_bytes,
            max_entries,
            usage,
            created: 0,
        }
    }
    pub fn acquire(&mut self, device: &wgpu::Device, size: [u32; 2]) -> Shared<Texture> {
        if let Some(at) = self
            .textures
            .iter()
            .position(|texture| texture.size() == size && Shared::strong_count(texture) == 1)
        {
            let texture = self.textures.remove(at);
            self.textures.push(Shared::clone(&texture));
            return texture;
        }
        let texture = Texture::new(device, size, self.usage);
        self.created += 1;
        let bytes = texture.bytes();
        if bytes > self.max_bytes {
            return texture;
        }
        while self.textures.len() >= self.max_entries || self.bytes + bytes > self.max_bytes {
            let Some(at) = self
                .textures
                .iter()
                .position(|t| Shared::strong_count(t) == 1)
            else {
                // All retained textures are leased. This allocation stays transient.
                return texture;
            };
            self.bytes -= self.textures.remove(at).bytes();
        }
        self.bytes += bytes;
        self.textures.push(Shared::clone(&texture));
        texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Renderer {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        Renderer::new(&instance, None).unwrap().0
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn canvas_leases_preserve_old_frames_and_reuse_released_targets() {
        let mut gpu = renderer();
        let first = gpu.compose([3, 2], [10, 20, 30, 255], &[]).unwrap();
        let second = gpu.compose([3, 2], [40, 50, 60, 255], &[]).unwrap();
        assert!(!Shared::ptr_eq(&first.target, &second.target));
        let released = Shared::downgrade(&second.target);
        drop(second);
        let third = gpu.compose([3, 2], [70, 80, 90, 255], &[]).unwrap();
        assert!(Weak::ptr_eq(&released, &Shared::downgrade(&third.target)));
        for red in 0..24 {
            drop(gpu.compose([3, 2], [red, 0, 0, 255], &[]).unwrap());
        }
        assert_eq!(gpu.canvases.created, 3);
        assert_eq!(first.read().unwrap().pixels(), [10, 20, 30, 255].repeat(6));
        assert_eq!(third.read().unwrap().pixels(), [70, 80, 90, 255].repeat(6));
        // Different sizes and leased targets beyond the pool's capacity remain valid.
        let mut retained = Vec::new();
        for side in 1..=8 {
            retained.push(
                gpu.compose([side, 2], [side as u8, 1, 2, 255], &[])
                    .unwrap(),
            );
        }
        assert!(gpu.canvases.textures.len() <= 4);
        for (i, frame) in retained.into_iter().enumerate() {
            assert_eq!(
                frame.read().unwrap().pixels(),
                [i as u8 + 1, 1, 2, 255].repeat((i + 1) * 2)
            );
        }
        gpu.check().unwrap();
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn texture_pool_bounds_idle_storage_without_reusing_live_leases() {
        let gpu = renderer();
        let mut pool = TexturePool::new(2, 32, wgpu::TextureUsages::TEXTURE_BINDING);
        let first = pool.acquire(&gpu.device, [2, 2]);
        let second = pool.acquire(&gpu.device, [2, 2]);
        let transient = pool.acquire(&gpu.device, [2, 2]);
        assert!(!Shared::ptr_eq(&first, &transient));
        assert!(!Shared::ptr_eq(&second, &transient));
        assert_eq!(pool.textures.len(), 2);
        assert_eq!(pool.bytes, 32);
        drop(first);
        let small = pool.acquire(&gpu.device, [1, 1]);
        assert_eq!(pool.textures.len(), 2);
        assert_eq!(pool.bytes, 20);
        // Oversized requests bypass the pool without evicting its useful entries.
        let oversized = pool.acquire(&gpu.device, [4, 4]);
        assert_eq!(oversized.bytes(), 64);
        assert_eq!(pool.bytes, 20);
        assert!(pool.textures.iter().any(|t| Shared::ptr_eq(t, &small)));
        assert!(pool.textures.iter().any(|t| Shared::ptr_eq(t, &second)));
        gpu.check().unwrap();
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn movie_uploads_reuse_textures_and_keep_previews_and_same_scene_inputs() {
        let mut gpu = renderer();
        for red in 0..64 {
            let image = RgbaImage::new(1, 1, vec![red, 20, 30, 255])
                .unwrap()
                .into_frame();
            let frame = gpu.render_image(&image, [1, 1], false).unwrap();
            let uploaded = gpu.uploaded_bytes;
            let duplicate = gpu.render_image(&image, [1, 1], false).unwrap();
            assert_eq!(gpu.uploaded_bytes, uploaded);
            assert_eq!(frame.read().unwrap(), duplicate.read().unwrap());
        }
        assert_eq!(gpu.movie_textures.created, FRAME_TEXTURES as u64);

        let images: Vec<_> = (0..FRAME_TEXTURES + 5)
            .map(|red| cmvs_core::scene::ObjectImage {
                name: Vec::new(),
                filter: None,
                alpha: true,
                surface: true,
                pixels: RgbaImage::new(1, 1, vec![red as u8, 80, 90, 255])
                    .unwrap()
                    .into_frame(),
            })
            .collect();
        let preview = gpu.preview_image(&images[0].pixels).unwrap();
        let layers: Vec<_> = images
            .iter()
            .enumerate()
            .map(|(i, image)| Layer::Image {
                image,
                rect: [0, 0, 1, 1],
                corner: [i as i64, 0],
                alpha: 255,
                mask: None,
                screen_mask: None,
                blend: cmvs_core::scene_render::Blend::Replace,
            })
            .collect();
        let scene = gpu
            .compose([images.len() as u32, 1], [0, 0, 0, 255], &layers)
            .unwrap();
        // Force further evictions before reading any of the retained outputs.
        for red in 100..140 {
            let image = RgbaImage::new(1, 1, vec![red, 1, 2, 255])
                .unwrap()
                .into_frame();
            drop(gpu.render_image(&image, [1, 1], false).unwrap());
        }
        assert_eq!(preview.read().unwrap().pixels(), &[0, 80, 90, 255]);
        let expected: Vec<_> = (0..images.len())
            .flat_map(|i| [i as u8, 80, 90, 255])
            .collect();
        assert_eq!(scene.read().unwrap().pixels(), expected);
        gpu.check().unwrap();
    }
}

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
        Self::with_format(device, size, usage, wgpu::TextureFormat::Rgba8Unorm)
    }

    pub fn with_format(
        device: &wgpu::Device,
        size: [u32; 2],
        usage: wgpu::TextureUsages,
        format: wgpu::TextureFormat,
    ) -> Shared<Self> {
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
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Shared::new(Self { texture, view })
    }
    pub fn size(&self) -> [u32; 2] {
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
            .map(|red| cmvs_core::render::scene::ObjectImage {
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
                blend: cmvs_core::render::scene_render::Blend::Replace,
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

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn cvns_layers_match_software_and_reuse_textures() {
        use cmvs_core::render::{
            scene::{
                DrawState, ObjectImage, SceneHost, SceneObject,
                cvns::{CvnsImageBuffer, CvnsSprite, CvnsSpriteKind},
            },
            scene_render::prepare_scene,
        };

        let mut scene = SceneHost::default();
        let cvns = scene.cvns_mut();
        cvns.sprites[0] = Some(CvnsSprite {
            kind: CvnsSpriteKind::SolidRect { color: 0x0020_3040 },
            src_rect: [0, 0, 12, 8],
            z: 127,
            ..CvnsSprite::default()
        });
        let pixels = RgbaImage::new(
            4,
            3,
            (0..12)
                .flat_map(|i| [30 * (i % 8), 20 * i, 13 * i, 0])
                .collect(),
        )
        .unwrap();
        cvns.images[0] = Some(CvnsImageBuffer {
            layers: vec![None],
            composited: Some(pixels.clone()),
            mask: None,
        });
        cvns.sprites[5] = Some(CvnsSprite {
            image_slot: 0,
            src_rect: [0, 0, 4, 3],
            x: -1,
            y: 1,
            ..CvnsSprite::default()
        });
        cvns.sprites[4] = Some(CvnsSprite {
            image_slot: 0,
            src_rect: [1, 0, 4, 3],
            x: 6,
            y: 3,
            origin_x: 1,
            origin_y: 1,
            scale_x: 2000,
            scale_y: 2000,
            rotation: 30,
            alpha: 150,
            blend_mode: 1,
            color_strength: 123,
            color: 0x003A_6CB7,
            ..CvnsSprite::default()
        });
        let mut draw = DrawState::default();
        draw.set_source_size(2, 2);
        draw.set_position([2, 1]);
        draw.set_layer(0);
        draw.set_opacity(256);
        let overlay = SceneObject {
            draw: Some(draw),
            image: Some(ObjectImage {
                pixels: RgbaImage::new(2, 2, [200, 80, 120, 255].repeat(4)).unwrap(),
                alpha: true,
                surface: false,
                name: Vec::new(),
                filter: None,
            }),
            shown: true,
            ..SceneObject::default()
        };
        let frame = prepare_scene(&scene, None, &[&overlay], 12, 8).unwrap();
        let expected = frame.render_cpu().unwrap();
        assert_eq!(frame.commands().len(), 4);
        assert!(frame.precomposited().is_none());
        let mut gpu = renderer();
        let actual = gpu.render(&frame).unwrap().read().unwrap();
        assert_eq!(actual, expected);
        assert_eq!(gpu.cache.len(), 2);
        let bytes = gpu.uploaded_bytes;
        drop(gpu.render(&frame).unwrap());
        assert_eq!(gpu.uploaded_bytes, bytes);
        gpu.check().unwrap();
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn cvns_oversized_source_uploads_only_the_canvas() {
        use cmvs_core::render::{
            scene::{
                SceneHost,
                cvns::{CvnsImageBuffer, CvnsSprite},
            },
            scene_render::prepare_scene,
        };

        let mut gpu = renderer();
        let width = gpu.device.limits().max_texture_dimension_2d + 1;
        let mut scene = SceneHost::default();
        let cvns = scene.cvns_mut();
        let pixel = [10, 20, 30, 255];
        cvns.images[0] = Some(CvnsImageBuffer {
            layers: vec![None],
            composited: Some(RgbaImage::new(width, 1, pixel.repeat(width as usize)).unwrap()),
            mask: None,
        });
        cvns.sprites[0] = Some(CvnsSprite {
            image_slot: 0,
            src_rect: [width as i32 - 1, 0, width as i32, 1],
            ..CvnsSprite::default()
        });
        let frame = prepare_scene(&scene, None, &[], 1, 1).unwrap();
        assert_eq!(gpu.render(&frame).unwrap().read().unwrap().pixels(), pixel);
        assert_eq!(gpu.uploaded_bytes, 4);
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn precomposited_frames_skip_content_cache_and_frame_start_protects_same_frame_uploads() {
        fn layer_at(image: &cmvs_core::render::scene::ObjectImage, x: usize) -> Layer<'_> {
            Layer::Image {
                image,
                rect: [0, 0, 1, 1],
                corner: [x as i64, 0],
                alpha: 255,
                mask: None,
                screen_mask: None,
                blend: cmvs_core::render::scene_render::Blend::Replace,
            }
        }

        let mut gpu = renderer();
        let render_composite = |gpu: &mut Renderer, red: u8| {
            let expected = RgbaImage::new(2, 1, vec![red, 20, 30, 255, 40, red, 60, 255]).unwrap();
            let frame = RenderFrame::from_precomposited(expected.clone());
            (gpu.render(&frame).unwrap(), expected)
        };
        let (retained_composite, first_expected) = render_composite(&mut gpu, 11);
        for red in 12..40 {
            drop(render_composite(&mut gpu, red).0);
        }
        let (final_composite, last_expected) = render_composite(&mut gpu, 99);
        assert!(gpu.cache.is_empty());
        assert_eq!(gpu.cache_bytes, 0);
        assert!(gpu.frames.is_empty());
        assert_eq!(gpu.canvases.created, 0);
        assert_eq!(gpu.movie_textures.created, 2);
        assert_eq!(retained_composite.read().unwrap(), first_expected);
        assert_eq!(final_composite.read().unwrap(), last_expected);
        drop((retained_composite, final_composite));

        let frame_obj = |red: u8| cmvs_core::render::scene::ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: true,
            pixels: RgbaImage::new(1, 1, vec![red, 50, 60, 255])
                .unwrap()
                .into_frame(),
        };
        let stale_movie = frame_obj(200);
        drop(
            gpu.compose([1, 1], [0, 0, 0, 255], &[layer_at(&stale_movie, 0)])
                .unwrap(),
        );
        let stale_movie_id = stale_movie.pixels.frame_id().unwrap();
        assert!(gpu.frames.contains_key(&stale_movie_id));

        let same_frame_movies: Vec<_> = (0..=FRAME_TEXTURES)
            .map(|i| frame_obj(i as u8 + 1))
            .collect();
        let mut movie_layers: Vec<_> = same_frame_movies
            .iter()
            .enumerate()
            .map(|(i, img)| layer_at(img, i))
            .collect();
        movie_layers.push(layer_at(&same_frame_movies[0], same_frame_movies.len()));
        let before_movies = gpu.uploaded_bytes;
        let movie_scene = gpu
            .compose(
                [movie_layers.len() as u32, 1],
                [0, 0, 0, 255],
                &movie_layers,
            )
            .unwrap();
        assert!(!gpu.frames.contains_key(&stale_movie_id));
        assert_eq!(gpu.frames.len(), FRAME_TEXTURES);
        assert!(
            !gpu.frames
                .contains_key(&same_frame_movies[1].pixels.frame_id().unwrap())
        );
        for img in std::iter::once(&same_frame_movies[0]).chain(&same_frame_movies[2..]) {
            assert!(gpu.frames.contains_key(&img.pixels.frame_id().unwrap()));
        }
        assert_eq!(
            gpu.uploaded_bytes - before_movies,
            (same_frame_movies.len() * 4) as u64
        );
        let mut expected_movies: Vec<u8> = (0..=FRAME_TEXTURES)
            .flat_map(|i| [i as u8 + 1, 50, 60, 255])
            .collect();
        expected_movies.extend_from_slice(&[1, 50, 60, 255]);
        assert_eq!(movie_scene.read().unwrap().pixels(), expected_movies);

        let static_obj = |red: u8| cmvs_core::render::scene::ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(1, 1, vec![red, 70, 80, 255]).unwrap(),
        };
        let pinned = static_obj(10);
        drop(
            gpu.compose([1, 1], [0, 0, 0, 255], &[layer_at(&pinned, 0)])
                .unwrap(),
        );
        assert_eq!(gpu.cache.len(), 1);
        gpu.cache.values_mut().next().unwrap().bytes = CACHE_LIMIT - 4;
        gpu.cache_bytes = CACHE_LIMIT - 4;

        let new_a = static_obj(20);
        let new_b = static_obj(30);
        let before_static = gpu.uploaded_bytes;
        let static_scene = gpu
            .compose(
                [4, 1],
                [0, 0, 0, 255],
                &[
                    layer_at(&pinned, 0),
                    layer_at(&new_a, 1),
                    layer_at(&new_b, 2),
                    layer_at(&new_b, 3),
                ],
            )
            .unwrap();
        assert_eq!(gpu.cache.len(), 2);
        assert_eq!(gpu.cache_bytes, CACHE_LIMIT);
        assert_eq!(gpu.uploaded_bytes - before_static, 8);
        assert_eq!(
            static_scene.read().unwrap().pixels(),
            &[
                10, 70, 80, 255, 20, 70, 80, 255, 30, 70, 80, 255, 30, 70, 80, 255
            ]
        );

        let new_c = static_obj(40);
        let next_scene = gpu
            .compose([1, 1], [0, 0, 0, 255], &[layer_at(&new_c, 0)])
            .unwrap();
        assert_eq!(gpu.cache.len(), 2);
        assert_eq!(gpu.cache_bytes, 8);
        assert_eq!(next_scene.read().unwrap().pixels(), &[40, 70, 80, 255]);
        gpu.check().unwrap();
    }

    #[test]
    #[ignore = "requires a native GPU adapter; run explicitly"]
    fn excess_same_scene_frame_textures_trim_after_submit_and_stay_bounded() {
        fn color_for(i: u8) -> [u8; 4] {
            [
                i.wrapping_mul(13).wrapping_add(5),
                i.wrapping_mul(29).wrapping_add(17),
                240u8.wrapping_sub(i.wrapping_mul(7)),
                255,
            ]
        }
        fn frame_obj(i: u8) -> cmvs_core::render::scene::ObjectImage {
            cmvs_core::render::scene::ObjectImage {
                name: Vec::new(),
                filter: None,
                alpha: true,
                surface: true,
                pixels: RgbaImage::new(1, 1, color_for(i).to_vec())
                    .unwrap()
                    .into_frame(),
            }
        }
        fn layer_at(image: &cmvs_core::render::scene::ObjectImage, x: usize) -> Layer<'_> {
            Layer::Image {
                image,
                rect: [0, 0, 1, 1],
                corner: [x as i64, 0],
                alpha: 255,
                mask: None,
                screen_mask: None,
                blend: cmvs_core::render::scene_render::Blend::Replace,
            }
        }

        let mut gpu = renderer();
        let first_16: Vec<_> = (0..16).map(frame_obj).collect();
        let first_layers: Vec<_> = first_16
            .iter()
            .enumerate()
            .map(|(x, img)| layer_at(img, x))
            .collect();
        let first_scene = gpu.compose([16, 1], [0, 0, 0, 255], &first_layers).unwrap();
        assert_eq!(gpu.frames.len(), FRAME_TEXTURES);
        assert_eq!(gpu.movie_textures.textures.len(), FRAME_TEXTURES);
        assert!(
            gpu.movie_textures
                .textures
                .iter()
                .all(|t| Shared::strong_count(t) == 1)
        );
        let transient_weaks: Vec<_> = gpu
            .frames
            .values()
            .map(|c| {
                assert!(
                    !gpu.movie_textures
                        .textures
                        .iter()
                        .any(|t| Shared::ptr_eq(t, &c.target))
                );
                assert_eq!(Shared::strong_count(&c.target), 1);
                Shared::downgrade(&c.target)
            })
            .collect();
        assert_eq!(transient_weaks.len(), FRAME_TEXTURES);
        assert!(transient_weaks.iter().all(|w| w.upgrade().is_some()));
        let expected_first: Vec<u8> = (0..16).flat_map(color_for).collect();
        assert_eq!(first_scene.read().unwrap().pixels(), expected_first);

        // Compose another 16-layer scene whose oldest 8 layers are those transient
        // allocations and whose newest 8 layers reuse the pooled movie textures, so
        // post-submit trimming evicts the transient allocations immediately.
        let extra_8: Vec<_> = (16..24).map(frame_obj).collect();
        let second_16: Vec<_> = first_16[FRAME_TEXTURES..]
            .iter()
            .chain(extra_8.iter())
            .collect();
        let second_layers: Vec<_> = second_16
            .iter()
            .enumerate()
            .map(|(x, img)| layer_at(img, x))
            .collect();
        let created_before_second = gpu.movie_textures.created;
        let second_scene = gpu
            .compose([16, 1], [0, 0, 0, 255], &second_layers)
            .unwrap();
        assert_eq!(gpu.frames.len(), FRAME_TEXTURES);
        assert_eq!(gpu.movie_textures.created, created_before_second);
        assert!(transient_weaks.iter().all(|w| w.upgrade().is_none()));
        let expected_second: Vec<u8> = (8..24).flat_map(color_for).collect();
        assert_eq!(second_scene.read().unwrap().pixels(), expected_second);

        // 32 subsequent 1-texture frames via compose and 32 via render_image stay bounded.
        for i in 24..56 {
            let obj = frame_obj(i);
            let scene = gpu
                .compose([1, 1], [0, 0, 0, 255], &[layer_at(&obj, 0)])
                .unwrap();
            assert!(gpu.frames.len() <= FRAME_TEXTURES);
            assert_eq!(scene.read().unwrap().pixels(), color_for(i));
        }
        for i in 56..88 {
            let img = frame_obj(i).pixels;
            let frame = gpu.render_image(&img, [1, 1], false).unwrap();
            assert!(gpu.frames.len() <= FRAME_TEXTURES);
            assert_eq!(frame.read().unwrap().pixels(), color_for(i));
        }

        // Verify frame_texture's while-loop also shrinks oversized stale frame maps.
        let mut stale_weaks = Vec::new();
        for i in 88..92 {
            let id = frame_obj(i).pixels.frame_id().unwrap();
            let tex = Texture::new(
                &gpu.device,
                [1, 1],
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            stale_weaks.push(Shared::downgrade(&tex));
            gpu.frames.insert(
                id,
                Cached {
                    target: tex,
                    bytes: 4,
                    used: 0,
                },
            );
        }
        assert_eq!(gpu.frames.len(), FRAME_TEXTURES + 4);
        let tail = frame_obj(92).pixels;
        let tail_frame = gpu.render_image(&tail, [1, 1], false).unwrap();
        assert_eq!(gpu.frames.len(), FRAME_TEXTURES);
        assert!(stale_weaks.iter().all(|w| w.upgrade().is_none()));
        assert_eq!(tail_frame.read().unwrap().pixels(), color_for(92));
        gpu.check().unwrap();
    }
}

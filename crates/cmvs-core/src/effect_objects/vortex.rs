//! PURIRING type 0x57: 0x41fad0/0x41f7c0 geometry, 0x41f410 update, 0x41f330 texture switch.
use super::legacy::{LegacyEffect, LegacyMesh, MeshScene, quarter};
use crate::{
    audio::AudioBackend, camera::CameraHost, dispatch::Call, image::ImageSource, random::CrtRandom,
    scene::SceneHost,
};
use std::error::Error;

#[derive(Debug, PartialEq, Eq)]
pub struct Vortex {
    pub scene: MeshScene,
    ring_clock: u32,
    particle_clock: u32,
    heights: [u32; 6],
}

fn upright(left: f32, top: f32, right: f32, bottom: f32) -> [[f32; 3]; 4] {
    [
        [left, top, 0.0],
        [right, top, 0.0],
        [left, bottom, 0.0],
        [right, bottom, 0.0],
    ]
}

impl Vortex {
    pub fn new(
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut images = Vec::new();
        // The last two are prepared for parameter 10, as with Magic's alternate texture.
        for name in [
            b"__ev-xx004a.pb3".as_slice(),
            b"__ev-xx004g.pb3",
            b"__ev-xx004f.pb3",
            b"__ev-xx004b1.pb3",
            b"it011d1.pb3",
            b"it011d2.pb3",
            b"it011d3.pb3",
            b"it011d4.pb3",
            b"it011c.pb3",
            b"__ev-xx004e.pb3",
            b"__ev-xx004b2.pb3",
        ] {
            images.push(super::curtain::load(source, name)?);
        }
        Ok(Self::with_images(images, random))
    }

    fn with_images(images: Vec<crate::scene::ObjectImage>, random: &mut CrtRandom) -> Self {
        let mut meshes = vec![
            LegacyMesh::new(0, upright(-15.0, 9.0, 15.0, -9.0), [0.0, 0.0, 32.0]),
            LegacyMesh::new(1, upright(-15.0, 9.0, 15.0, -9.0), [0.0, 0.0, 31.9]).blended(false),
            LegacyMesh::new(2, upright(-0.5, 0.5, 0.5, -0.5), [-0.17, 0.17, 1.2]).blended(true),
            LegacyMesh::new(3, upright(-2.3, 3.8, 1.7, -0.2), [2.6, -2.5, 10.0]).blended(true),
        ];
        meshes[1].alpha = 0;
        for image in 4..8 {
            let mut mesh = LegacyMesh::new(
                image,
                [
                    [-12.0, 0.0, 12.0],
                    [12.0, 0.0, 12.0],
                    [-12.0, 0.0, -12.0],
                    [12.0, 0.0, -12.0],
                ],
                [2.6, -2.5, 10.1],
            )
            .blended(true);
            mesh.alpha = 0;
            meshes.push(mesh);
        }
        let mut heights = [0; 6];
        let mut height = -0.6f32;
        for h in &mut heights {
            *h = height.to_bits();
            // Creation uses whole-degree steps of 11, unlike the frame's 11.25 degrees.
            for angle in (0..352u32).step_by(11) {
                let tile = random.next_value() % 8;
                let u = (tile & 3) as f32 * 0.25;
                let v = (tile >> 2) as f32 * 0.25;
                let (x, z) = quarter(angle as f32, 1.5);
                let mut mesh = LegacyMesh::new(
                    8,
                    upright(-0.1, 0.1, 0.1, -0.1),
                    [x - 0.17, height, z + 1.2],
                )
                .blended(true);
                mesh.rotation[1] = ((450 - angle) % 360) as f32;
                mesh.uv = [u, v, u + 0.25, v + 0.25];
                meshes.push(mesh);
            }
            height = (f64::from(height) + f64::from(0.3f32)) as f32;
        }
        Self {
            scene: MeshScene { images, meshes },
            ring_clock: 0,
            particle_clock: 0,
            heights,
        }
    }

    pub fn request(&mut self, value: u32) {
        if value == 10 {
            self.scene.meshes[2].image = 9;
            self.scene.meshes[3].image = 10;
            for index in [1, 4, 5, 6, 7] {
                self.scene.meshes[index].alpha = 255;
            }
        }
    }

    pub fn advance(&mut self, delta: u32) {
        let delta = delta.min(200);
        self.ring_clock = self.ring_clock.wrapping_add(delta);
        let angle = (self.ring_clock.wrapping_mul(3600) / 30000 % 3600) as f32 / 10.0;
        for (i, mesh) in self.scene.meshes[4..8].iter_mut().enumerate() {
            mesh.rotation[1] = if i & 1 == 0 { angle } else { 360.0 - angle };
        }
        self.particle_clock = self.particle_clock.wrapping_add(delta);
        let phase = self.particle_clock.wrapping_mul(36000) / 4000 % 36000;
        let step = (2000 * delta / 3000) as f32 / 1000.0;
        for (ring, height) in self.heights.iter_mut().enumerate() {
            let mut y = f32::from_bits(*height) - step;
            if y <= -0.9 {
                y = (f64::from(y) + f64::from(1.8f32)) as f32;
            }
            *height = y.to_bits();
            for particle in 0..32 {
                let mut angle = (particle as u32 * 1125 + phase) % 36000;
                if ring & 1 == 0 {
                    angle = (36000 - angle) % 36000;
                }
                let (x, z) = quarter(angle as f32 / 100.0, 1.5);
                let mesh = &mut self.scene.meshes[8 + ring * 32 + particle];
                mesh.position = [x - 0.17, y, z + 1.2];
                mesh.rotation[1] = ((45000 - angle) % 36000 / 100) as f32;
            }
        }
    }
}

impl SceneHost {
    /// 0x2297 / 0x43c9e0: one slot argument, no system result, return 0x4004.
    pub(crate) fn create_vortex(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
        camera: &mut CameraHost,
        random: &mut CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        let mut next_random = random.clone();
        let vortex = Vortex::new(source, &mut next_random)?;
        self.release_legacy(slot as u8, backend)?;
        call.complete(0x4004)?;
        self.clear_effect_slot(slot as u8);
        camera.set_view_position([0.0; 3]);
        camera.set_view_angle(0, 0.0);
        camera.set_view_angle(1, 0.0);
        *random = next_random;
        self.legacy_effects
            .insert(slot as u8, LegacyEffect::Vortex(Box::new(vortex)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{raster::RgbaImage, scene::ObjectImage};

    fn effect() -> Vortex {
        let image = ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(64, 64, vec![255; 64 * 64 * 4]).unwrap(),
        };
        Vortex::with_images(vec![image; 11], &mut CrtRandom::new(0))
    }

    #[test]
    fn geometry_uses_six_rings_and_one_crt_draw_per_atlas_particle() {
        let mut rng = CrtRandom::new(17);
        let mut expected = rng.clone();
        let tiles: Vec<_> = (0..192).map(|_| expected.next_value() % 8).collect();
        let effect = Vortex::with_images(effect().scene.images, &mut rng);
        assert_eq!(rng, expected);
        assert_eq!(effect.scene.meshes.len(), 200);
        assert_eq!(effect.scene.quads().len(), 195);
        assert_eq!(effect.scene.meshes[0].position, [0.0, 0.0, 32.0]);
        assert_eq!(effect.scene.meshes[8].position, [1.33, -0.6, 1.2]);
        assert_eq!(effect.scene.meshes[9].rotation, [0.0, 79.0, 0.0]);
        for (mesh, tile) in effect.scene.meshes[8..].iter().zip(tiles) {
            let u = (tile & 3) as f32 * 0.25;
            let v = (tile >> 2) as f32 * 0.25;
            assert_eq!(mesh.uv, [u, v, u + 0.25, v + 0.25]);
            assert!(mesh.blended && mesh.late);
        }
    }

    #[test]
    fn frame_caps_delta_wraps_heights_and_rotates_neighboring_rings_oppositely() {
        let mut effect = effect();
        effect.advance(5000);
        assert_eq!((effect.ring_clock, effect.particle_clock), (200, 200));
        assert_eq!(effect.scene.meshes[4].rotation, [0.0, 2.4, 0.0]);
        assert_eq!(effect.scene.meshes[5].rotation, [0.0, 357.6, 0.0]);
        let first = &effect.scene.meshes[8];
        let next_ring = &effect.scene.meshes[40];
        assert_eq!(first.rotation, [0.0, 108.0, 0.0]);
        assert_eq!(next_ring.rotation, [0.0, 72.0, 0.0]);
        assert!(first.position[2] < 1.2 && next_ring.position[2] > 1.2);
        assert_eq!(first.position[1], -0.6f32 - 0.133);
        effect.heights[0] = (-0.9f32).to_bits();
        effect.advance(0);
        assert_eq!(f32::from_bits(effect.heights[0]), 0.9);
        effect.ring_clock = u32::MAX - 10;
        effect.particle_clock = u32::MAX - 10;
        effect.advance(16);
        assert_eq!((effect.ring_clock, effect.particle_clock), (5, 5));
    }

    #[test]
    fn texture_request_is_immediate_idempotent_and_snapshot_saves_only_the_type() {
        let mut effect = effect();
        effect.request(9);
        assert_eq!(effect.scene.meshes[2].image, 2);
        effect.request(10);
        effect.request(10);
        assert_eq!(effect.scene.meshes[2].image, 9);
        assert_eq!(effect.scene.meshes[3].image, 10);
        assert_eq!(effect.scene.quads().len(), 200);
        let mut scene = SceneHost::default();
        scene
            .legacy_effects
            .insert(1, LegacyEffect::Vortex(Box::new(effect)));
        assert_eq!(scene.query_legacy_effect(1), Some(0));
        assert_eq!(
            scene.saved_effect_pools_with_legacy(true).unwrap(),
            vec![(1, vec![1, 0, 0x57, 0])]
        );
        assert_eq!(scene.legacy_effects.len(), 1);
        scene.clear_effect_slot(1);
        assert!(scene.legacy_effects.is_empty());
    }
}

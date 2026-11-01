//! Identity-bound type-2 falling billboards; see games/aoitori/vm-semantics.md.
use crate::{
    camera::CameraHost,
    dispatch::Call,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{ObjectImage, SceneHost},
};
use std::{collections::BTreeMap, error::Error};

const MAX_PARTICLES: u32 = 16_384;

#[derive(Debug, PartialEq, Eq)]
pub struct FallingParticle {
    /// Original five-word record: pool index, X/Y/Z and speed divisor.
    pub words: [u32; 5],
    pub opacity: u32,
}

impl FallingParticle {
    pub fn position(&self) -> [f32; 3] {
        [self.words[1], self.words[2], self.words[3]].map(f32::from_bits)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FallingEffect {
    /// Width, height, nominal fall duration and fully opaque distance, as f32 bits.
    pub parameters: [u32; 4],
    pub follow: bool,
    pub image: Option<ObjectImage>,
    pub particles: Vec<FallingParticle>,
}

impl FallingEffect {
    pub(crate) fn new(
        parameters: [u32; 4],
        count: u32,
        follow: bool,
        image: Option<ObjectImage>,
        random: &mut CrtRandom,
    ) -> Self {
        let [width, height, duration, _] = parameters.map(f32::from_bits);
        let unit = |random: &mut CrtRandom| random.next_value() as f32 / 32767.0;
        let particles = (0..count)
            .map(|id| {
                let x = unit(random) * width - width * 0.5;
                let z = unit(random) * width - width * 0.5;
                let y = unit(random) * height;
                let spread = duration / 5.0;
                let speed = (unit(random) * spread - spread * 0.5) + duration;
                FallingParticle {
                    words: [id, x.to_bits(), y.to_bits(), z.to_bits(), speed.to_bits()],
                    opacity: 255,
                }
            })
            .collect();
        Self {
            parameters,
            follow,
            image,
            particles,
        }
    }

    pub fn advance(&mut self, delta: u32, view: [f32; 3]) {
        let [width, height, _, distance] = self.parameters.map(f32::from_bits);
        let delta = delta.min(500) as f32;
        for particle in &mut self.particles {
            let [mut x, mut y, mut z] = particle.position();
            y -= delta * width / f32::from_bits(particle.words[4]);
            if y <= 0.0 {
                y += height;
            }
            if self.follow {
                for (value, camera) in [(&mut x, view[0]), (&mut z, view[2])] {
                    let difference = *value - camera;
                    if difference.abs() > width * 0.5 {
                        *value = if difference < 0.0 {
                            width + *value
                        } else {
                            *value - width
                        };
                    }
                }
            }
            let d = ((y - view[1]) * (y - view[1])
                + (x - view[0]) * (x - view[0])
                + (z - view[2]) * (z - view[2]))
                .sqrt();
            particle.opacity = if d > distance {
                let excess = d - distance;
                if excess <= 1.0 {
                    ((1.0 - excess) * 255.0) as u32
                } else {
                    0
                }
            } else if y < 0.5 {
                ((y * 510.0) as i32).clamp(0, 255) as u32
            } else {
                255
            };
            particle.words[1..4].copy_from_slice(&[x.to_bits(), y.to_bits(), z.to_bits()]);
        }
    }
}

impl SceneHost {
    pub fn falling_effects(&self) -> &BTreeMap<u8, FallingEffect> {
        &self.falling_effects
    }

    pub fn advance_falling_effects(&mut self, delta: u32, cameras: &CameraHost) {
        let view = cameras.view(0).expect("view exists").words[..3]
            .try_into()
            .map(|words: [u32; 3]| words.map(f32::from_bits))
            .expect("three axes");
        for effect in self.falling_effects.values_mut() {
            effect.advance(delta, view);
        }
    }

    pub(crate) fn create_falling_effect(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let name_id = call.argument(8, "effect image")?;
        let parameters = [
            call.argument(12, "fall width")?,
            call.argument(16, "fall height")?,
            call.argument(20, "fall duration")?,
            call.argument(24, "opaque distance")?,
        ];
        let count = call.argument(28, "falling particle count")?;
        let follow = call.argument(32, "follow view")? != 0;
        let name = call.vm.resolve_string(name_id)?;
        if slot >= 4 {
            call.complete(0x4020)?;
            return Ok(());
        }
        let values = parameters.map(f32::from_bits);
        if count > MAX_PARTICLES
            || values.iter().any(|v| !v.is_finite())
            || values[..3].iter().any(|&v| v <= 0.0)
        {
            return Err(call.error("falling effect needs bounded count and finite positive dimensions and duration").into());
        }
        let image = image::load(source, &name)?.map(|decoded| ObjectImage {
            filter: None,
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            surface: false,
        });
        let mut candidate_random = random.clone();
        let effect = FallingEffect::new(parameters, count, follow, image, &mut candidate_random);
        call.complete(0x4020)?;
        self.particle_effects.remove(&(slot as u8));
        self.falling_effects.insert(slot as u8, effect);
        *random = candidate_random;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        raster::RgbaImage,
        scene_render::{self, Layer},
    };

    fn parameters() -> [u32; 4] {
        [10.0f32, 8.0, 12000.0, 6.0].map(f32::to_bits)
    }

    #[test]
    fn initialization_matches_float32_reference_and_advancement_retains_rng_and_single_wraps() {
        let mut random = CrtRandom::new(0);
        let mut effect = FallingEffect::new(parameters(), 2, true, None, &mut random);
        assert_eq!(
            effect.particles[0].words,
            [0, 3231686911, 1084616012, 3223927763, 1177258493]
        );
        assert_eq!(
            effect.particles[1].words,
            [1, 3222473638, 1073919237, 3216190054, 1179497161]
        );
        let mut expected = CrtRandom::new(0);
        for _ in 0..8 {
            expected.next_value();
        }
        assert_eq!(random.next_value(), expected.next_value());
        effect.particles[0].words[1..5].copy_from_slice(&[
            (-20.0f32).to_bits(),
            0.25f32.to_bits(),
            20.0f32.to_bits(),
            100.0f32.to_bits(),
        ]);
        effect.advance(900, [0.0, 1.0, 0.0]);
        // Only one height addition and one horizontal wrap, even if still outside.
        assert_eq!(effect.particles[0].position(), [-10.0, -41.75, 10.0]);
        assert_eq!(effect.particles[0].opacity, 0);
    }

    #[test]
    fn distance_fade_and_ground_fade_follow_original_priority() {
        let mut effect = FallingEffect::new(parameters(), 1, false, None, &mut CrtRandom::new(0));
        for (position, view, expected) in [
            ([0.0f32, 1.0, 0.0], [0.0, 1.0, 0.0], 255),
            ([0.0, 0.25, 0.0], [0.0, 0.25, 0.0], 127),
            ([0.0, 0.25, 0.0], [6.5, 0.25, 0.0], 127),
            ([0.0, 0.25, 0.0], [7.5, 0.25, 0.0], 0),
        ] {
            effect.particles[0].words[1..4].copy_from_slice(&position.map(f32::to_bits));
            effect.advance(0, view);
            assert_eq!(effect.particles[0].opacity, expected);
        }
    }

    #[test]
    fn world_billboards_use_device_view_queue_five_and_render_without_scene_slots() {
        let mut scene = SceneHost::default();
        let image = ObjectImage {
            filter: None,
            name: b"fall.pb3".to_vec(),
            alpha: true,
            surface: false,
            pixels: RgbaImage::new(1, 1, vec![255, 0, 0, 255]).unwrap(),
        };
        let mut effect =
            FallingEffect::new(parameters(), 1, false, Some(image), &mut CrtRandom::new(0));
        effect.particles[0].words[1..4].copy_from_slice(&[0, 1.0f32.to_bits(), 1.0f32.to_bits()]);
        scene.falling_effects.insert(0, effect);
        let cameras = CameraHost::default();
        assert!(scene_render::prepare_scene(&scene, None, &[], 40, 40).is_err());
        let frame = scene_render::prepare_scene(&scene, Some(&cameras), &[], 40, 40).unwrap();
        assert!(matches!(frame.commands(), [Layer::Billboard { .. }]));
        let image = frame.render_cpu().unwrap();
        assert_eq!(
            &image.pixels()[(20 * 40 + 20) * 4..(20 * 40 + 20) * 4 + 4],
            &[255, 0, 0, 255]
        );
        assert!(scene.objects().is_empty());
    }
}

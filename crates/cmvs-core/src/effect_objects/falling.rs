//! Identity-bound type-2 falling billboards; see games/aoitori/vm-semantics.md and, for the
//! x87 arithmetic, games/memoria/README.md.
use crate::{
    audio::AudioBackend,
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
    pub(crate) template: Option<crate::snapshot::SavedEffect>,
    /// Width, height, nominal fall duration and fully opaque distance, as f32 bits.
    pub parameters: [u32; 4],
    pub follow: bool,
    pub image: Option<ObjectImage>,
    pub particles: Vec<FallingParticle>,
    /// MEMORIA 0x4325e0 / 0x4327a0: f64 intermediates with explicit f32 stores.
    pub(crate) x87_arithmetic: bool,
}

impl FallingEffect {
    #[cfg(test)]
    pub(crate) fn new(
        parameters: [u32; 4],
        count: u32,
        follow: bool,
        image: Option<ObjectImage>,
        random: &mut CrtRandom,
    ) -> Self {
        Self::new_with_arithmetic(parameters, count, follow, image, random, false)
    }

    pub(crate) fn new_with_arithmetic(
        parameters: [u32; 4],
        count: u32,
        follow: bool,
        image: Option<ObjectImage>,
        random: &mut CrtRandom,
        x87_arithmetic: bool,
    ) -> Self {
        let [width, height, duration, _] = parameters.map(f32::from_bits);
        let unit = |random: &mut CrtRandom| random.next_value() as f32 / 32767.0;
        // Each x87 store rounds one f64 expression of rand / 32767.0 (0x43266d..0x432714).
        let wide = |random: &mut CrtRandom| f64::from(random.next_value()) / 32767.0;
        let particles = (0..count)
            .map(|id| {
                let (x, y, z, speed) = if x87_arithmetic {
                    let half = f64::from(width * 0.5);
                    let x = (wide(random) * f64::from(width) - half) as f32;
                    let z = (wide(random) * f64::from(width) - half) as f32;
                    let y = (wide(random) * f64::from(height)) as f32;
                    let spread = f64::from((f64::from(duration) / 5.0) as f32);
                    let offset = (wide(random) * spread - spread * 0.5) as f32;
                    (x, y, z, offset + duration)
                } else {
                    let x = unit(random) * width - width * 0.5;
                    let z = unit(random) * width - width * 0.5;
                    let y = unit(random) * height;
                    let spread = duration / 5.0;
                    let speed = (unit(random) * spread - spread * 0.5) + duration;
                    (x, y, z, speed)
                };
                FallingParticle {
                    words: [id, x.to_bits(), y.to_bits(), z.to_bits(), speed.to_bits()],
                    opacity: 255,
                }
            })
            .collect();
        Self {
            template: None,
            parameters,
            follow,
            image,
            particles,
            x87_arithmetic,
        }
    }

    pub fn advance(&mut self, delta: u32, view: [f32; 3]) {
        let [width, height, _, distance] = self.parameters.map(f32::from_bits);
        let delta = delta.min(500) as f32;
        for particle in &mut self.particles {
            let [mut x, mut y, mut z] = particle.position();
            let speed = f32::from_bits(particle.words[4]);
            y -= if self.x87_arithmetic {
                // 0x432841: the product stays in f64; only the quotient is stored.
                (f64::from(width) * f64::from(delta) / f64::from(speed)) as f32
            } else {
                delta * width / speed
            };
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
            let d = if self.x87_arithmetic {
                // 0x434080: f64 squares summed as (X + Y) + Z, then sqrt, stored as f32.
                let [dx, dy, dz] = [(x, view[0]), (y, view[1]), (z, view[2])]
                    .map(|(a, b)| f64::from(a) - f64::from(b));
                (dx * dx + dy * dy + dz * dz).sqrt() as f32
            } else {
                ((y - view[1]) * (y - view[1])
                    + (x - view[0]) * (x - view[0])
                    + (z - view[2]) * (z - view[2]))
                    .sqrt()
            };
            let scale = |value: f32, factor: f32| {
                if self.x87_arithmetic {
                    (f64::from(value) * f64::from(factor)) as i32
                } else {
                    (value * factor) as i32
                }
            };
            particle.opacity = if d > distance {
                let excess = d - distance;
                if excess <= 1.0 {
                    scale(1.0 - excess, 255.0) as u32
                } else {
                    0
                }
            } else if y < 0.5 {
                scale(y, 510.0).clamp(0, 255) as u32
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
        x87_arithmetic: bool,
        backend: Option<&mut dyn AudioBackend>,
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
        let follow_word = call.argument(32, "follow view")?;
        let follow = follow_word != 0;
        // MEMORIA 0x4539a0 passes `follow != 0`, so its object and template keep 0 or 1.
        let follow_word = if x87_arithmetic {
            u32::from(follow)
        } else {
            follow_word
        };
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
        let template = crate::snapshot::SavedEffect::name_field(&name)
            .ok()
            .map(|name| crate::snapshot::SavedEffect::Falling {
                words: [
                    parameters[0],
                    parameters[1],
                    parameters[2],
                    parameters[3],
                    count,
                    follow_word,
                ],
                name,
            });
        let image = image::load(source, &name)?.map(|decoded| ObjectImage {
            filter: None,
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            surface: false,
        });
        let mut candidate_random = random.clone();
        let mut effect = FallingEffect::new_with_arithmetic(
            parameters,
            count,
            follow,
            image,
            &mut candidate_random,
            x87_arithmetic,
        );
        effect.template = template;
        self.replace_effect_slot(slot as u8, backend)?;
        call.complete(0x4020)?;
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
    fn memoria_x87_creation_normalizes_follow_and_matches_independent_reference() {
        use crate::{
            dispatch::{self, Hosts, ImageHosts},
            effect_objects::legacy_flakes::dispatch_tests::{command_script, pending},
            machine::Machine,
            profile::{Semantics, VariantId},
            snapshot::SavedEffect,
        };
        struct Images;
        impl ImageSource for Images {
            fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
                Ok(Some(b"NONE".to_vec()))
            }
        }
        let f = f32::to_bits;
        // snha05 arguments: width 10, height 8, duration 10000, opaque distance 6.
        let words = [f(10.0), f(8.0), f(10000.0), f(6.0), 2];
        let script = command_script(0x22a2, &[&[0, 0][..], &words, &[7]].concat());
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut scene = SceneHost::default();
        let mut random = CrtRandom::new(0);
        dispatch::run(
            Semantics::Variant(VariantId::FallingBillboardsX87),
            &mut vm,
            &mut Hosts {
                random: Some(&mut random),
                images: Some(ImageHosts {
                    scene: &mut scene,
                    source: &mut Images,
                    change_images: false,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(vm.stack().len_bytes(), 0);
        let mut effect = scene.falling_effects.remove(&0).unwrap();
        assert!(effect.follow && effect.x87_arithmetic);
        assert_eq!(
            effect.template,
            Some(SavedEffect::Falling {
                words: [words[0], words[1], words[2], words[3], 2, 1],
                name: SavedEffect::name_field(b"flake.pb3").unwrap(),
            })
        );
        // Independent Python struct/f64 reference with the native f32 store points.
        assert_eq!(
            effect.particles[0].words,
            [0, 3231686911, 1084616012, 3223927762, 1175384829]
        );
        assert_eq!(
            effect.particles[1].words,
            [1, 3222473638, 1073919237, 3216190054, 1177250386]
        );
        assert_eq!(random.next_value(), 10450);
        effect.advance(900, [1.0, 2.0, -3.0]);
        assert_eq!(
            effect.particles[0].words[1..4],
            [1084251905, 1083469870, 3223927762]
        );
        assert_eq!(effect.particles[0].opacity, 255);
        // Distance fade: (1 - excess) is stored as f32, then scaled in f64 (69, not 70).
        effect.follow = false;
        let mut single = FallingEffect::new(parameters(), 1, false, None, &mut CrtRandom::new(0));
        for (target, expected) in [(&mut effect, 69), (&mut single, 70)] {
            target.particles[0].words[1..4].copy_from_slice(&[1087095161, 1074321411, 1051665910]);
            target.advance(0, [0.0; 3]);
            assert_eq!(target.particles[0].opacity, expected);
        }
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

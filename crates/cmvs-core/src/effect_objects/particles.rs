//! Aoitori type-5 particle pool; executable-bound evidence is in games/aoitori/vm-semantics.md.
use crate::{
    dispatch::Call,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{Animation, AnimationFrame, DrawState, ObjectImage, SceneHost, SceneObject},
};
use std::{collections::BTreeMap, error::Error};

/// Research allocation bound, independent of the original pool's linked-list capacity.
const MAX_PARTICLES: u32 = 16_384;

#[derive(Debug, PartialEq, Eq)]
pub struct Particle {
    /// Original 48-byte record, retained as words to preserve floating-point bits.
    pub words: [u32; 12],
    pub object: SceneObject,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParticleEffect {
    pub mode: u32,
    pub parameters: [u32; 4],
    /// Pool image is shared by its independent objects, rather than copied per particle.
    pub pool: SceneObject,
    pub particles: Vec<Particle>,
}

fn unit(random: &mut CrtRandom) -> f32 {
    random.next_value() as f32 / 32767.0
}

// CVTTSS2SI returns the indefinite integer for NaN or values outside the i32 range.
fn hundredth(value: f32) -> f32 {
    let value = value * 100.0;
    let integer = if (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    };
    integer as f32 / 100.0
}

fn range(mode: u32) -> f32 {
    match (mode >> 24) & 15 {
        1 => 15.0,
        2 => 10.0,
        3 => 5.0,
        4 => 2.5,
        5 => 1.0,
        _ => 20.0,
    }
}

impl ParticleEffect {
    fn new(
        mode: u32,
        parameters: [u32; 4],
        count: u32,
        pool: SceneObject,
        random: &mut CrtRandom,
    ) -> Self {
        let [base_x, base_z, angle, speed] = parameters.map(f32::from_bits);
        let mut particles = Vec::with_capacity(count as usize);
        for id in 0..count {
            let mut words = [0; 12];
            words[0] = 1;
            words[1] = id;
            words[3] = (unit(random) * 120.0 + base_x - 60.0).to_bits();
            words[4] = 100.0f32.to_bits();
            words[5] = (unit(random) * 15.0 + base_z).to_bits();
            let x = unit(random) * 120.0 + base_x - 60.0;
            let z = unit(random) * range(mode) + base_z;
            let y = unit(random) * 100.0;
            let direction = random.next_value() as i32 % 100;
            let mut direction_float = direction as f32 / 10.0 - 5.0 + angle + 270.0;
            if direction_float >= 360.0 {
                direction_float -= 360.0;
            }
            words[6] = direction_float.to_bits();
            words[7] = hundredth(x).to_bits();
            words[8] = hundredth(y).to_bits();
            words[9] = hundredth(z).to_bits();
            words[10] = direction_float.to_bits();
            let spread = speed / 5.0;
            words[11] = (unit(random) * spread - spread * 0.5 + speed).to_bits();
            let column = (random.next_value() % 3) as i16 * 40;
            let mut draw = DrawState::default();
            draw.words[0] = 2;
            draw.words[15] = match mode >> 28 {
                4 => 60.0,
                5 => 50.0,
                6 => 40.0,
                7 => 30.0,
                8 => 20.0,
                _ => base_z + 5.0,
            }
            .to_bits();
            draw.words[11..14].copy_from_slice(&words[7..10]);
            let mut animation = Animation::default();
            for (i, frame) in animation.frames[..10].iter_mut().enumerate() {
                *frame = AnimationFrame {
                    next: ((i + 1) % 10) as i32,
                    words: [
                        column + if i < 6 { 0 } else { 20 },
                        (if i < 6 { i } else { i - 6 }) as i16 * 20,
                        20,
                        20,
                        10,
                        10,
                    ],
                    duration: 100,
                };
            }
            animation.start(0, &mut draw.words);
            draw.animation = Some(Box::new(animation));
            particles.push(Particle {
                words,
                object: SceneObject {
                    draw: Some(draw),
                    flags: 0,
                    ..Default::default()
                },
            });
        }
        Self {
            mode,
            parameters,
            pool,
            particles,
        }
    }

    fn advance(&mut self, delta: u32, random: &mut CrtRandom) {
        let [base_x, base_z, _, _] = self.parameters.map(f32::from_bits);
        let time = delta.min(500) as f32 * 10.0;
        for particle in &mut self.particles {
            let words = &mut particle.words;
            // The original rounds radians to f32 before calling double-precision CRT trig.
            let radians = f64::from(f32::from_bits(words[10]) / 180.0 * std::f32::consts::PI);
            let distance = time / f32::from_bits(words[11]);
            words[7] = (f32::from_bits(words[7])
                + hundredth((radians.cos() * f64::from(distance)) as f32))
            .to_bits();
            words[8] = (f32::from_bits(words[8])
                + hundredth((radians.sin() * f64::from(distance)) as f32))
            .to_bits();
            if f32::from_bits(words[8]) < 0.0 {
                words[7] = hundredth(unit(random) * 120.0 + base_x - 60.0).to_bits();
                words[9] = hundredth(unit(random) * range(self.mode) + base_z).to_bits();
                words[8] = 100.0f32.to_bits();
            }
            particle.object.draw.as_mut().expect("particle draw").words[11..14]
                .copy_from_slice(&words[7..10]);
            particle.object.advance_effects(delta);
        }
    }
}

impl SceneHost {
    pub fn particle_effects(&self) -> &BTreeMap<u8, ParticleEffect> {
        &self.particle_effects
    }

    /// Called once per running frame, sharing the runtime CRT generator.
    pub fn advance_particles(&mut self, delta: u32, random: &mut CrtRandom) {
        for effect in self.particle_effects.values_mut() {
            effect.advance(delta, random);
        }
    }

    pub(crate) fn create_particles(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let mode = call.argument(12, "particle mode")?;
        let parameters = [
            call.argument(16, "particle base x")?,
            call.argument(20, "particle base z")?,
            call.argument(24, "particle angle")?,
            call.argument(28, "particle speed")?,
        ];
        let count = call.argument(32, "particle count")?;
        let name_id = call.argument(8, "particle image")?;
        let name = call.vm.resolve_string(name_id)?;
        if slot >= 4 {
            call.complete(0x4020)?;
            return Ok(());
        }
        if count > MAX_PARTICLES {
            return Err(call
                .error("particle count exceeds research allocation bound")
                .into());
        }
        if parameters
            .iter()
            .any(|&word| !f32::from_bits(word).is_finite())
            || f32::from_bits(parameters[3]) <= 0.0
        {
            return Err(call
                .error("particle parameters need finite values and positive speed")
                .into());
        }
        let image = image::load(source, &name)?.map(|decoded| ObjectImage {
            filter: None,
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            surface: false,
        });
        let pool = SceneObject {
            image,
            ..Default::default()
        };
        let mut candidate_random = random.clone();
        let effect = ParticleEffect::new(mode, parameters, count, pool, &mut candidate_random);
        call.complete(0x4020)?;
        self.falling_effects.remove(&(slot as u8));
        self.particle_effects.insert(slot as u8, effect);
        *random = candidate_random;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(mode: u32, random: &mut CrtRandom) -> ParticleEffect {
        ParticleEffect::new(
            mode,
            [
                40.0f32.to_bits(),
                20.0f32.to_bits(),
                (-20.0f32).to_bits(),
                2000.0f32.to_bits(),
            ],
            1,
            SceneObject::default(),
            random,
        )
    }

    #[test]
    fn seed_zero_initialization_matches_float32_reference_and_sprite_cycle() {
        let mut random = CrtRandom::new(0);
        let mut effect = effect(0, &mut random);
        // Independent MSVC LCG + float32 reference, including the first two unused positions.
        assert_eq!(
            effect.particles[0].words,
            [
                1, 0, 0, 3248415486, 1120403456, 1102857416, 1132376883, 1114051707, 1104685302,
                1101780746, 1132376883, 1156432814
            ]
        );
        let draw = effect.particles[0].object.draw.as_ref().unwrap();
        assert_eq!(&draw.words[2..8], &[80, 0, 20, 20, 10, 10]);
        assert_eq!(draw.words[15], 25.0f32.to_bits());
        for (time, x, y) in [
            (500, 80, 40),
            (200, 80, 80),
            (200, 100, 0),
            (200, 100, 40),
            (200, 80, 0),
        ] {
            effect.advance(time, &mut random);
            let draw = effect.particles[0].object.draw.as_ref().unwrap();
            assert_eq!(&draw.words[2..4], &[x, y]);
        }
    }

    #[test]
    fn movement_clamps_at_500ms_and_respawn_draws_exactly_two_random_values() {
        let mut random = CrtRandom::new(0);
        let mut a = effect(0, &mut random);
        let mut other_random = random.clone();
        let mut b = effect(0, &mut CrtRandom::new(0));
        a.advance(500, &mut random);
        b.advance(900, &mut other_random);
        assert_eq!(a, b);
        let p = &mut a.particles[0];
        p.words[7] = 5.0f32.to_bits();
        p.words[8] = 0.01f32.to_bits();
        p.words[10] = 270.0f32.to_bits();
        p.words[11] = 100.0f32.to_bits();
        let mut expected = random.clone();
        let x = hundredth(unit(&mut expected) * 120.0 + 40.0 - 60.0);
        let z = hundredth(unit(&mut expected) * 20.0 + 20.0);
        a.advance(16, &mut random);
        assert_eq!(random, expected);
        assert_eq!(
            &a.particles[0].words[7..10],
            &[x.to_bits(), 100.0f32.to_bits(), z.to_bits()]
        );
        assert_eq!(
            a.particles[0].object.draw.as_ref().unwrap().words[11..14],
            a.particles[0].words[7..10]
        );
    }

    #[test]
    fn independent_pool_particles_render_with_the_shared_texture_and_camera() {
        let mut effect = effect(0, &mut CrtRandom::new(0));
        effect.pool.image = Some(ObjectImage {
            filter: None,
            name: b"synthetic".to_vec(),
            pixels: crate::raster::RgbaImage::new(120, 120, [255, 0, 0, 255].repeat(120 * 120))
                .unwrap(),
            alpha: true,
            surface: false,
        });
        let draw = effect.particles[0].object.draw.as_mut().unwrap();
        draw.words[11..14].copy_from_slice(&[
            0.0f32.to_bits(),
            0.0f32.to_bits(),
            20.0f32.to_bits(),
        ]);
        let mut scene = SceneHost::default();
        scene.particle_effects.insert(0, effect);
        assert!(scene.objects().is_empty());
        assert!(crate::scene_render::prepare_scene(&scene, None, &[], 1024, 612).is_err());
        let mut camera = crate::camera::CameraHost::default();
        let mut record = camera.saved(0);
        record[14..18].copy_from_slice(&3u32.to_le_bytes());
        camera.restore(0, &record).unwrap();
        let frame =
            crate::scene_render::prepare_scene(&scene, Some(&camera), &[], 1024, 612).unwrap();
        assert_eq!(frame.commands().len(), 1);
        assert!(
            frame
                .render_cpu()
                .unwrap()
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p == &[255, 0, 0, 255])
        );
    }

    #[test]
    fn mode_fields_select_independent_depth_range_and_sprite_projection_size() {
        for (mode, width, depth) in [
            (0, 25.0f32, 20.0f32),
            (0x41000000, 60.0, 15.0),
            (0x52000000, 50.0, 10.0),
            (0x63000000, 40.0, 5.0),
            (0x74000000, 30.0, 2.5),
            (0x85000000, 20.0, 1.0),
        ] {
            let effect = effect(mode, &mut CrtRandom::new(0));
            assert_eq!(range(mode), depth);
            assert_eq!(
                effect.particles[0].object.draw.as_ref().unwrap().words[15],
                width.to_bits()
            );
        }
        assert_eq!(hundredth(-1.239), -1.23);
        assert_eq!(hundredth(f32::NAN), i32::MIN as f32 / 100.0);
    }
}

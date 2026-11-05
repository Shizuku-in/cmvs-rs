//! ASUNANA x86 type 4: timed, dynamically allocated rain streaks.
use crate::{
    dispatch::Call,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
    snapshot::SavedEffect,
};
use std::error::Error;

#[derive(Debug)]
pub(crate) struct RainParticle {
    pub object: SceneObject,
    last_tick: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(mode: u32, interval: u32) -> RainEffect {
        RainEffect::new(
            [mode, 0, 0, 0, 250.0f32.to_bits(), interval],
            SceneObject {
                image: Some(ObjectImage {
                    name: b"rain.pb3".to_vec(),
                    pixels: crate::raster::RgbaImage::new(6, 112, vec![255; 6 * 112 * 4]).unwrap(),
                    alpha: false,
                    filter: None,
                    surface: false,
                }),
                ..Default::default()
            },
            SavedEffect::name_field(b"rain.pb3").unwrap(),
        )
    }

    #[test]
    fn emission_boundary_rng_movement_and_creation_only_snapshot() {
        let mut effect = effect(0, 10);
        let mut random = CrtRandom::new(0);
        effect.advance(10, &mut random);
        assert!(effect.particles.is_empty());
        assert_eq!(random, CrtRandom::new(0));
        effect.advance(1, &mut random);
        let draw = effect.particles[0].object.draw.as_ref().unwrap();
        // Independent CRT/f32 reference for the first X/Z and a 1 ms step.
        assert_eq!(
            &draw.words[11..14],
            &[0xc247893f, 149.0f32.to_bits(), 0x41bc7539]
        );
        let mut expected = CrtRandom::new(0);
        expected.next_value();
        expected.next_value();
        assert_eq!(random, expected);
        let bytes = effect.template.encode().unwrap();
        assert_eq!(bytes.len(), 60);
        assert_eq!(SavedEffect::decode(&bytes).unwrap(), effect.template);
        // Advancing does not persist live streaks or consume random values at save time.
        assert_eq!(bytes, effect.template.encode().unwrap());
        let mut scene = SceneHost::default();
        scene.rain_effects.insert(0, effect);
        let profile = crate::profile::Registry::builtin()
            .profile("cmvs-20080903")
            .unwrap();
        let cameras = crate::camera::CameraHost::for_profile(profile);
        let frame =
            crate::scene_render::prepare_scene(&scene, Some(&cameras), &[], 800, 600).unwrap();
        assert!(!frame.commands().is_empty());
        struct UnrecognizedImage;
        impl ImageSource for UnrecognizedImage {
            fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
                Ok(Some(b"NONE".to_vec()))
            }
        }
        let records = scene.saved_effect_pools().unwrap();
        let borrowed: Vec<_> = records
            .iter()
            .map(|(slot, bytes)| (*slot, bytes.as_slice()))
            .collect();
        let before = random.clone();
        let restored = SceneHost::load_effect_pools(
            &borrowed,
            &mut UnrecognizedImage,
            &mut random,
            &mut 0,
            false,
        )
        .unwrap();
        assert_eq!(restored.saved_effect_pools().unwrap(), records);
        assert!(restored.rain_effects[&0].particles.is_empty());
        assert_eq!(random, before);
        scene
            .advance_effect_pools(250, &mut random, &cameras)
            .unwrap();
        assert!(
            scene.rain_effects[&0]
                .particles
                .iter()
                .all(|p| f32::from_bits(p.object.draw.as_ref().unwrap().words[12]) > -50.0)
        );
    }

    #[test]
    fn doubled_emission_cap_and_missing_images_keep_rng_bounded() {
        let mut rain = effect(0x1000, 10);
        let mut random = CrtRandom::new(0);
        rain.advance(21, &mut random);
        assert_eq!(rain.particles.len(), 4);
        let before = random.clone();
        rain.pool.image = None;
        rain.advance(2000, &mut random);
        assert_eq!(random, before);
        assert!(rain.particles.is_empty());
        assert_eq!(rain.remainder, 10);
        rain.template = SavedEffect::Rain {
            words: [0, 0, 0, 0, 1.0f32.to_bits(), 0],
            name: [0; 32],
        };
        assert!(rain.template.encode().is_err());
    }
}

#[derive(Debug)]
pub(crate) struct RainEffect {
    pub pool: SceneObject,
    pub particles: Vec<RainParticle>,
    pub template: SavedEffect,
    words: [u32; 6],
    clock: u32,
    remainder: u32,
}

impl RainEffect {
    pub(crate) fn new(words: [u32; 6], pool: SceneObject, name: [u8; 32]) -> Self {
        Self {
            pool,
            particles: Vec::new(),
            template: SavedEffect::Rain { words, name },
            words,
            clock: 0,
            remainder: 0,
        }
    }

    fn spawn(&mut self, tick: u32, random: &mut CrtRandom) {
        if self.particles.len() >= 2048 || self.pool.image.is_none() {
            return;
        }
        let mode = self.words[0];
        let x_offset = match mode & 0xf0 {
            0 => -50.0,
            0x10 => 0.0,
            0x20 => -100.0,
            0x30 => 50.0,
            0x40 => -150.0,
            0x50 => -15.0,
            0x60 => -85.0,
            0x70 => -25.0,
            0x80 => -75.0,
            0xa0 => -78.0,
            _ => unreachable!("validated rain mode"),
        };
        let x = (f64::from(random.next_value()) / 32767.0 * 100.0 + x_offset) as f32;
        let depth_mode = (mode >> 8) & 0xf;
        let z = (f64::from(random.next_value()) / 32767.0
            * if depth_mode == 0 { 100.0 } else { 50.0 }
            + if depth_mode == 0 {
                0.0
            } else {
                f64::from(depth_mode - 1) * 10.0
            }) as f32;
        let angle = f32::from_bits(self.words[3]);
        let mut draw = DrawState::default();
        draw.words[0] = 2;
        draw.words[2..8].copy_from_slice(&[0, 0, 6, 112, 3, 112]);
        draw.words[11..14].copy_from_slice(&[x.to_bits(), 150.0f32.to_bits(), z.to_bits()]);
        draw.words[15] = (20.5 + (mode & 3) as f32 * 10.0).to_bits();
        draw.words[16] = 20.0f32.to_bits();
        draw.words[20] =
            (crate::evaluate::truncate(angle).wrapping_add(360) % 360).unsigned_abs() * 100;
        self.particles.push(RainParticle {
            object: SceneObject {
                draw: Some(draw),
                flags: 0,
                ..Default::default()
            },
            last_tick: tick,
        });
    }

    pub(crate) fn advance(&mut self, delta: u32, random: &mut CrtRandom) {
        self.clock = self.clock.wrapping_add(delta);
        self.remainder = self.remainder.wrapping_add(delta).min(1000);
        let interval = self.words[5];
        while self.remainder > interval {
            let tick = self
                .clock
                .wrapping_add(interval)
                .wrapping_sub(self.remainder);
            self.spawn(tick, random);
            if self.words[0] & 0x1000 != 0 {
                self.spawn(tick, random);
            }
            self.remainder -= interval;
        }
        let angle = f32::from_bits(self.words[3]);
        let speed = f64::from(f32::from_bits(self.words[4]));
        for particle in &mut self.particles {
            let draw = particle.object.draw.as_mut().expect("rain draw");
            let distance =
                (f64::from(self.clock.wrapping_sub(particle.last_tick)) * speed / 250.0) as f32;
            let (dx, dy) = if angle == 0.0 {
                (0.0, distance)
            } else {
                let radians = f64::from(angle + 90.0) / 180.0 * f64::from(std::f32::consts::PI);
                (
                    (radians.cos() * f64::from(distance)) as f32,
                    (radians.sin() * f64::from(distance)) as f32,
                )
            };
            draw.words[11] = (f32::from_bits(draw.words[11]) + dx).to_bits();
            draw.words[12] = (f32::from_bits(draw.words[12]) - dy).to_bits();
            particle.last_tick = self.clock;
        }
        self.particles
            .retain(|p| f32::from_bits(p.object.draw.as_ref().unwrap().words[12]) > -50.0);
    }
}

impl SceneHost {
    pub(crate) fn create_rain(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let name_id = call.argument(8, "rain image")?;
        let name = call.vm.resolve_string(name_id)?;
        let words = [12, 16, 20, 24, 28, 32]
            .map(|offset| call.argument(offset, "rain parameter"))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let words: [u32; 6] = words.try_into().unwrap();
        if slot >= 4 {
            call.complete(0x4020)?;
            return Ok(());
        }
        let name_field = SavedEffect::name_field(&name)?;
        let template = SavedEffect::Rain {
            words,
            name: name_field,
        };
        template.encode()?;
        let image = image::load(source, &name)?.map(|decoded| ObjectImage {
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            filter: None,
            surface: false,
        });
        let effect = RainEffect::new(
            words,
            SceneObject {
                image,
                ..Default::default()
            },
            name_field,
        );
        call.complete(0x4020)?;
        self.particle_effects.remove(&(slot as u8));
        self.falling_effects.remove(&(slot as u8));
        self.collision_effects.remove(&(slot as u8));
        self.fragment_effects.remove(&(slot as u8));
        self.rain_effects.insert(slot as u8, effect);
        Ok(())
    }
}

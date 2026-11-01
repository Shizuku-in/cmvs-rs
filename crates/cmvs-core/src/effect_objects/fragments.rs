//! Type-8 image fragments; executable-bound evidence is in games/aoitori/vm-semantics.md.
use crate::{
    bytecode::DecodeError,
    dispatch::Call,
    random::CrtRandom,
    scene::{DrawState, SceneHost, SceneObject},
};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Native 36-byte record: active, index, delay, XY, direction, speed, drift, drift step.
    pub words: [u32; 9],
    pub object: SceneObject,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FragmentEffect {
    pub parameters: [u32; 2],
    pub elapsed: u32,
    pub complete: bool,
    pub pool: SceneObject,
    pub fragments: Vec<Fragment>,
}

impl FragmentEffect {
    fn new(size: [u32; 2], parameters: [u32; 2], random: &mut CrtRandom) -> Self {
        let columns = size[0].div_ceil(20);
        let rows = size[1].div_ceil(20);
        let edge = parameters[0];
        let mut fragments = Vec::with_capacity((columns * rows) as usize);
        for row in 0..rows {
            for column in 0..columns {
                let x = column * edge;
                let y = row * edge;
                let mut direction = (random.next_value() % 100) as f32 / 10.0 - 5.0 + 180.0 + 270.0;
                if direction >= 360.0 {
                    direction -= 360.0;
                }
                let delay = 70 * (rows - row) + random.next_value() % 60;
                let speed = (random.next_value() % 100) as f32 / 100.0 - 0.5 + 4.0;
                let drift_step = (random.next_value() % 100) as f32 / 50.0 - 1.0;
                let center = edge >> 1;
                let mut draw = DrawState::default();
                draw.words[2..11].copy_from_slice(&[
                    x,
                    y,
                    edge,
                    edge,
                    center,
                    center,
                    x + center,
                    y + center,
                    16,
                ]);
                fragments.push(Fragment {
                    words: [
                        1,
                        fragments.len() as u32,
                        delay,
                        x + center,
                        y + center,
                        direction.to_bits(),
                        speed.to_bits(),
                        0,
                        drift_step.to_bits(),
                    ],
                    object: SceneObject {
                        flags: 0,
                        draw: Some(draw),
                        logical_size: size,
                        ..Default::default()
                    },
                });
            }
        }
        Self {
            parameters,
            elapsed: 0,
            complete: false,
            pool: SceneObject::default(),
            fragments,
        }
    }

    pub(crate) fn advance(&mut self, delta: u32) {
        self.elapsed = self.elapsed.wrapping_add(delta);
        let mut running = false;
        for fragment in &mut self.fragments {
            let words = &mut fragment.words;
            if self.elapsed < words[2] {
                running = true;
                continue;
            }
            let time = self.elapsed - words[2];
            let drift = f32::from_bits(words[7]) + f32::from_bits(words[8]);
            words[7] = drift.to_bits();
            let distance = time as f32 / f32::from_bits(words[6]);
            let y = (words[4] as i32).wrapping_add(distance as i32);
            let draw = fragment.object.draw.as_mut().expect("fragment draw");
            draw.words[8] = words[3].wrapping_add((drift as i32) as u32);
            draw.words[9] = y as u32;
            draw.words[20] = (360u32.wrapping_mul(time) / 10000 % 360) * 100;
            let scale = 1.0 - (time as f32 / 7500.0).clamp(0.0, 1.0);
            draw.words[18..20].fill(scale.to_bits());
            running |= y <= (self.pool.logical_size[1] as i32).wrapping_add(32);
        }
        if !running {
            self.complete = true;
        }
    }
}

impl SceneHost {
    pub fn fragment_effects(&self) -> &BTreeMap<u8, FragmentEffect> {
        &self.fragment_effects
    }

    pub(crate) fn create_fragments(
        &mut self,
        call: &mut Call<'_, '_>,
        random: &mut CrtRandom,
    ) -> Result<(), DecodeError> {
        let slot = call.argument(4, "effect slot")?;
        let source = call.argument(8, "source object")?;
        let parameters = [
            call.argument(12, "fragment edge")?,
            call.argument(16, "fragment parameter")?,
        ];
        if slot >= 4 {
            call.complete(0x4010)?;
            return Ok(());
        }
        let source =
            u8::try_from(source).map_err(|_| call.error("fragment source outside 0..255"))?;
        let Some(pool) = self.object(source) else {
            call.complete(0x4010)?;
            return Ok(());
        };
        let size = pool.image.as_ref().map_or([0; 2], |image| {
            [image.pixels.width(), image.pixels.height()]
        });
        if size[0].div_ceil(20) as u64 * size[1].div_ceil(20) as u64 > 16_384
            || parameters[0] == 0
            || parameters[0] > i16::MAX as u32
            || size.iter().any(|&v| v > i16::MAX as u32)
        {
            return Err(call.error("fragment geometry exceeds research bounds"));
        }
        let mut candidate_random = random.clone();
        let mut effect = FragmentEffect::new(size, parameters, &mut candidate_random);
        call.complete(0x4010)?;
        effect.pool = self.take_object(source).expect("validated source");
        effect.pool.logical_size = size;
        self.particle_effects.remove(&(slot as u8));
        self.falling_effects.remove(&(slot as u8));
        self.collision_effects.remove(&(slot as u8));
        self.fragment_effects.insert(slot as u8, effect);
        *random = candidate_random;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{raster::RgbaImage, scene::ObjectImage, scene_render};

    #[test]
    fn grid_random_draws_delays_and_frame_based_drift_match_native_contract() {
        let mut random = CrtRandom::new(0);
        let mut effect = FragmentEffect::new([41, 21], [20, 1000], &mut random);
        effect.pool.logical_size = [41, 21];
        assert_eq!(effect.fragments.len(), 6);
        // Independent CRT/f32 reference, including the otherwise unused direction draw.
        assert_eq!(
            effect.fragments[0].words,
            [1, 0, 179, 10, 10, 1118935448, 1081627116, 0, 3196395192]
        );
        let mut expected = CrtRandom::new(0);
        for _ in 0..24 {
            expected.next_value();
        }
        assert_eq!(random, expected);
        assert_eq!(
            &effect.fragments[5].object.draw.as_ref().unwrap().words[2..10],
            &[40, 20, 20, 20, 10, 10, 50, 30]
        );
        let fragment = &mut effect.fragments[0];
        fragment.words[2] = 100;
        fragment.words[6] = 4.0f32.to_bits();
        fragment.words[8] = 0.75f32.to_bits();
        effect.advance(99);
        assert_eq!(effect.fragments[0].words[7], 0);
        effect.advance(1);
        assert_eq!(effect.fragments[0].words[7], 0.75f32.to_bits());
        effect.advance(1000);
        let draw = effect.fragments[0].object.draw.as_ref().unwrap();
        assert_eq!(&draw.words[8..10], &[11, 260]);
        assert_eq!(draw.words[20], 3600);
        assert_eq!(draw.words[18], (1.0f32 - 1000.0 / 7500.0).to_bits());
        assert!(effect.complete);
        effect.advance(7500);
        assert_eq!(
            effect.fragments[0].object.draw.as_ref().unwrap().words[18],
            0
        );
    }

    #[test]
    fn fragments_draw_shared_texture_and_respect_parent_visibility() {
        let mut scene = SceneHost::default();
        scene.restore(
            Default::default(),
            BTreeMap::new(),
            Some(crate::scene::Quake {
                mode: 1,
                amplitude: 3,
                period: 100,
                count: 10,
                paused: false,
                elapsed: 0,
                steps: 1,
                offset: [3, 0],
            }),
            None,
            None,
            None,
        );
        let mut effect = FragmentEffect::new([1280, 20], [20, 1000], &mut CrtRandom::new(0));
        effect.pool.logical_size = [1280, 20];
        effect.pool.image = Some(ObjectImage {
            pixels: RgbaImage::new(1280, 20, [200, 40, 10, 255].repeat(1280 * 20)).unwrap(),
            alpha: false,
            surface: false,
            filter: None,
            name: Vec::new(),
        });
        scene.fragment_effects.insert(0, effect);
        let frame = scene_render::prepare_scene(&scene, None, &[], 1280, 720).unwrap();
        assert_eq!(frame.commands().len(), 64);
        let rendered = frame.render_cpu().unwrap();
        assert_eq!(&rendered.pixels()[..4], &[0, 0, 0, 255]);
        assert_eq!(&rendered.pixels()[12..16], &[200, 40, 10, 255]);
        assert_eq!(
            &rendered.pixels()[1280 * 19 * 4 + 12..1280 * 19 * 4 + 16],
            &[200, 40, 10, 255]
        );
        scene.fragment_effects.get_mut(&0).unwrap().pool.flags &= !0x100;
        let rendered = scene_render::prepare_scene(&scene, None, &[], 1280, 720)
            .unwrap()
            .render_cpu()
            .unwrap();
        assert_eq!(&rendered.pixels()[..4], &[200, 40, 10, 255]);
        scene.fragment_effects.get_mut(&0).unwrap().pool.shown = false;
        assert!(
            scene_render::prepare_scene(&scene, None, &[], 1280, 720)
                .unwrap()
                .commands()
                .is_empty()
        );
    }
}

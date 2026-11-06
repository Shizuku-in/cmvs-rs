//! cmvs-20070119 type 3: 0x43cdf0 / 0x421880 / 0x421af0.
use crate::{
    dispatch::Call,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
    snapshot::SavedEffect,
};
use std::error::Error;

#[derive(Debug)]
pub(crate) struct LegacyFlake {
    pub object: SceneObject,
    duration: f32,
}

#[derive(Debug)]
pub(crate) struct LegacyFlakes {
    pub pool: SceneObject,
    pub particles: Vec<LegacyFlake>,
    pub template: SavedEffect,
}
impl LegacyFlakes {
    pub(crate) fn new(
        words: [u32; 4],
        pool: SceneObject,
        name: [u8; 32],
        random: &mut CrtRandom,
    ) -> Self {
        let unit = |r: &mut CrtRandom| f64::from(r.next_value()) / 32767.0;
        let base_z = f32::from_bits(words[1]);
        let duration = f32::from_bits(words[2]);
        // Native children use an independent linked list, not the 256 fixed child slots.
        let particles = (0..words[3])
            .map(|_| {
                let x = (unit(random) * 10.0 - 5.0) as f32;
                let z = (unit(random) * 10.0 + f64::from(base_z)) as f32;
                let y = (unit(random) * 10.0) as f32;
                // Preserve the x87 writer's explicit float stores.
                let spread = (f64::from(duration) / 5.0) as f32;
                let offset = (unit(random) * f64::from(spread) - f64::from(spread) * 0.5) as f32;
                let duration = (f64::from(duration) + f64::from(offset)) as f32;
                let mut draw = DrawState::default();
                draw.words[0] = 2;
                draw.words[2..8].copy_from_slice(&[0, 0, 32, 32, 16, 16]);
                draw.words[11..14].copy_from_slice(&[x.to_bits(), y.to_bits(), z.to_bits()]);
                draw.words[15] = (3.0 + (words[0] & 3) as f32).to_bits();
                draw.words[16] = 3.0f32.to_bits();
                LegacyFlake {
                    object: SceneObject {
                        draw: Some(draw),
                        flags: 0,
                        ..Default::default()
                    },
                    duration,
                }
            })
            .collect();
        Self {
            pool,
            particles,
            template: SavedEffect::LegacyFlakes { words, name },
        }
    }
    pub(crate) fn advance(&mut self, delta: u32) {
        for p in &mut self.particles {
            let draw = p.object.draw.as_mut().expect("flake draw");
            let step = (f64::from(delta.min(500)) * 10.0 / f64::from(p.duration)) as f32;
            let mut y = (f64::from(f32::from_bits(draw.words[12])) - f64::from(step)) as f32;
            // Strictly negative, one wrap even if a very short duration crosses twice.
            if y < 0.0 {
                y = (f64::from(y) + 10.0) as f32;
            }
            draw.words[12] = y.to_bits();
        }
    }
}
impl SceneHost {
    pub(crate) fn create_legacy_flakes(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let reference = call.argument(8, "flake image")?;
        let name = call.vm.resolve_string(reference)?;
        let words = [
            call.argument(12, "projection mode")?,
            call.argument(16, "base Z")?,
            call.argument(20, "fall duration")?,
            call.argument(24, "flake count")?,
        ];
        if slot >= 4 {
            call.complete(0x4018)?;
            return Ok(());
        }
        let field = SavedEffect::name_field(&name)?;
        SavedEffect::LegacyFlakes { words, name: field }.encode()?;
        let image = image::load(source, &name)?.map(|decoded| ObjectImage {
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            filter: None,
            surface: false,
        });
        let mut candidate = random.clone();
        let effect = LegacyFlakes::new(
            words,
            SceneObject {
                image,
                ..Default::default()
            },
            field,
            &mut candidate,
        );
        call.complete(0x4018)?;
        self.clear_effect_slot(slot as u8);
        self.legacy_flakes.insert(slot as u8, effect);
        *random = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(count: u32) -> LegacyFlakes {
        LegacyFlakes::new(
            [0x1003, 2.0f32.to_bits(), 1000.0f32.to_bits(), count],
            SceneObject {
                image: Some(ObjectImage {
                    name: b"flake.pb3".to_vec(),
                    pixels: crate::raster::RgbaImage::new(32, 32, vec![255; 32 * 32 * 4]).unwrap(),
                    alpha: false,
                    filter: None,
                    surface: false,
                }),
                ..Default::default()
            },
            SavedEffect::name_field(b"flake.pb3").unwrap(),
            &mut CrtRandom::new(0),
        )
    }
    #[test]
    fn initialization_matches_crt_order_float_stores_and_projected_geometry() {
        let e = effect(3);
        let first = &e.particles[0];
        let d = first.object.draw.as_ref().unwrap();
        assert_eq!(&d.words[11..14], &[0xc09fa0ff, 0x40cf689f, 0x408b6217]);
        assert_eq!(first.duration.to_bits(), 0x4464b7fb);
        assert_eq!(d.words[0], 2);
        assert_eq!(&d.words[2..8], &[0, 0, 32, 32, 16, 16]);
        assert_eq!(
            [d.words[15], d.words[16]],
            [6.0f32.to_bits(), 3.0f32.to_bits()]
        );
        let mut r = CrtRandom::new(0);
        let e = LegacyFlakes::new(
            [0, 0, 1000.0f32.to_bits(), 300],
            SceneObject::default(),
            [0; 32],
            &mut r,
        );
        assert_eq!(e.particles.len(), 300); // linked children are not capped at 256
        let mut expected = CrtRandom::new(0);
        for _ in 0..1200 {
            expected.next_value();
        }
        assert_eq!(r, expected);
    }
    #[test]
    fn update_clamps_time_wraps_once_only_below_zero_and_keeps_xz() {
        let mut e = effect(1);
        e.particles[0].duration = 1000.0;
        e.particles[0].object.draw.as_mut().unwrap().words[12] = 5.0f32.to_bits();
        let before = e.particles[0].object.draw.as_ref().unwrap().words;
        e.advance(2000);
        assert_eq!(e.particles[0].object.draw.as_ref().unwrap().words[12], 0);
        e.advance(1);
        let d = e.particles[0].object.draw.as_ref().unwrap();
        assert!((f32::from_bits(d.words[12]) - 9.99).abs() < 0.00001);
        assert_eq!((d.words[11], d.words[13]), (before[11], before[13]));
        e.particles[0].duration = 100.0;
        e.particles[0].object.draw.as_mut().unwrap().words[12] = 0;
        e.advance(500);
        assert_eq!(
            f32::from_bits(e.particles[0].object.draw.as_ref().unwrap().words[12]),
            -40.0
        );
    }
    #[test]
    fn native_snapshot_saves_creation_parameters_and_load_regenerates_with_current_rng() {
        let mut e = effect(4);
        let record = e.template.encode().unwrap();
        assert_eq!(record.len(), 52);
        assert_eq!(&record[..4], &[1, 0, 3, 0]);
        e.advance(500);
        assert_eq!(e.template.encode().unwrap(), record);
        assert_eq!(SavedEffect::decode(&record).unwrap(), e.template);
        let mut scene = SceneHost::default();
        scene.legacy_flakes.insert(2, e);
        let records = scene.saved_effect_pools_with_legacy(true).unwrap();
        struct NoImage;
        impl ImageSource for NoImage {
            fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
                Ok(Some(b"NONE".to_vec()))
            }
        }
        let mut r = CrtRandom::new(77);
        let mut expected = r.clone();
        for _ in 0..16 {
            expected.next_value();
        }
        let restored = SceneHost::load_effect_pools(
            &[(2, record.as_slice())],
            &mut NoImage,
            &mut r,
            &mut 0,
            false,
        )
        .unwrap();
        assert_eq!(r, expected);
        assert_eq!(restored.legacy_flakes[&2].particles.len(), 4);
        assert_eq!(
            restored.saved_effect_pools_with_legacy(true).unwrap(),
            records
        );
        scene.clear_effect_slot(2);
        assert!(!scene.has_effect_pools());
        // A real image goes through the shared CPU/GPU draw-command preparation.
        scene.legacy_flakes.insert(0, effect(100));
        let p = crate::profile::Registry::builtin()
            .profile("cmvs-20070119")
            .unwrap();
        let c = crate::camera::CameraHost::for_profile(p);
        let frame = crate::scene_render::prepare_scene(&scene, Some(&c), &[], 1024, 614).unwrap();
        assert!(!frame.commands().is_empty());
        let pixels = frame.render_cpu().unwrap();
        assert!(pixels.pixels().chunks_exact(4).any(|p| p[0] != 0));
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::{
        dispatch::{self, Flow, Hosts, ImageHosts},
        machine::{Machine, Step},
        ps3::Script,
    };
    fn script(slot: u32, duration: u32, count: u32) -> Script {
        let mut code = Vec::new();
        for arg in [slot, 0, 0, 0, duration, count].into_iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
        }
        code.extend(0x22a3u16.to_le_bytes());
        let text = b"flake.pb3\0";
        let mut payload = code.clone();
        payload.extend(text);
        let mut raw = b"PS2A".to_vec();
        for w in [
            48,
            0,
            0,
            0,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(w.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        Script::decode(&raw).unwrap()
    }
    fn pending(vm: &mut Machine<'_>) {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    }
    struct Images(usize, bool);
    impl ImageSource for Images {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            self.0 += 1;
            if self.1 {
                Err("read failed".into())
            } else {
                Ok(Some(b"NONE".to_vec()))
            }
        }
    }
    #[test]
    fn creation_and_failures_preserve_stack_results_rng_and_previous_slot() {
        let p = crate::profile::Registry::builtin()
            .profile("cmvs-20070119")
            .unwrap();
        let semantics = p.semantics(0x2a3);
        assert_eq!(
            semantics,
            crate::profile::Semantics::Variant(crate::profile::VariantId::LegacyEffects)
        );
        let mut scene = SceneHost::default();
        let mut random = CrtRandom::new(0);
        let mut images = Images(0, false);
        for (slot, duration, count, fail) in [
            (2, 1000f32.to_bits(), 300, false),
            (2, 0, 3, true),
            (2, f32::NAN.to_bits(), 3, true),
            (2, 1000f32.to_bits(), 16385, true),
            (2, 1000f32.to_bits(), 1, true),
            (4, 0, u32::MAX, false),
        ] {
            let script = script(slot, duration, count);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.stack().len_bytes());
            let rng = random.clone();
            let records = scene.saved_effect_pools_with_legacy(true).unwrap();
            vm.state_mut().set_system_integer(0, 77).unwrap();
            images.1 = count == 1;
            let old_reads = images.0;
            let out = dispatch::run(
                semantics,
                &mut vm,
                &mut Hosts {
                    images: Some(ImageHosts {
                        scene: &mut scene,
                        source: &mut images,
                        change_images: false,
                    }),
                    random: Some(&mut random),
                    ..Default::default()
                },
            );
            assert_eq!(out.is_err(), fail);
            assert_eq!(vm.state().system_integer(0), Some(77));
            if fail {
                assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
                assert_eq!(random, rng);
                assert_eq!(scene.saved_effect_pools_with_legacy(true).unwrap(), records);
                if count != 1 {
                    assert_eq!(images.0, old_reads);
                }
            } else {
                assert_eq!(out.unwrap(), Some(Flow::Done));
                assert_eq!(vm.stack().len_bytes(), 0);
                assert_eq!(scene.legacy_flakes[&2].particles.len(), 300);
                if slot >= 4 {
                    assert_eq!(random, rng);
                    assert_eq!(images.0, old_reads);
                }
            }
        }
    }
}

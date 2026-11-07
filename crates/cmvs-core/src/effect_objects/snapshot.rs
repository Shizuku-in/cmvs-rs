//! Identity-bound runtime rebuilding of creation-only 0x800 records.
use super::{CollisionEffect, FallingEffect, ParticleEffect};
use crate::{
    FormatError,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{ObjectImage, SceneHost, SceneObject},
    snapshot::SavedEffect,
};
use std::{collections::BTreeMap, error::Error};

impl SceneHost {
    /// Creators and restores only fill slots 0..3.
    pub(crate) fn has_effect_pools(&self) -> bool {
        (0..4).any(|slot| self.has_effect_slot(slot))
    }

    pub(crate) fn saved_effect_pools(&self) -> Result<Vec<(u16, Vec<u8>)>, FormatError> {
        self.saved_effect_pools_with_legacy(false)
    }

    pub(crate) fn saved_effect_pools_with_legacy(
        &self,
        legacy: bool,
    ) -> Result<Vec<(u16, Vec<u8>)>, FormatError> {
        if !legacy && !self.legacy_effects.is_empty() {
            return Err(FormatError(
                "snapshots of cmvs-20070119 legacy effect slots are not implemented",
            ));
        }
        let mut records = BTreeMap::new();
        for (&slot, effect) in &self.legacy_effects {
            use super::LegacyEffect;
            let kind = match effect {
                LegacyEffect::SnowField(_) => 0x50,
                LegacyEffect::Walk(_) => 0x51,
                LegacyEffect::Circle(_) => 0x52,
                LegacyEffect::Curtain(_) => 0x53,
                LegacyEffect::Magic(_) => 0x54,
                LegacyEffect::Sprites(_) => 0x55,
                LegacyEffect::FloatingPlanes(_) => 0x56,
                LegacyEffect::Vortex(_) => 0x57,
            };
            if slot >= 4 {
                return Err(FormatError("legacy effect slot exceeds 3"));
            }
            records.insert(u16::from(slot), vec![1, 0, kind, 0]);
        }

        let mut insert = |slot: u8, template: Option<&SavedEffect>| -> Result<(), FormatError> {
            let template =
                template.ok_or(FormatError("particle creation template is unavailable"))?;
            if slot >= 4
                || records
                    .insert(u16::from(slot), template.encode()?)
                    .is_some()
            {
                return Err(FormatError("saved particle slots overlap or exceed 0..3"));
            }
            Ok(())
        };
        for (&slot, effect) in &self.legacy_flakes {
            insert(slot, Some(&effect.template))?;
        }
        for (&slot, effect) in &self.rain_effects {
            insert(slot, Some(&effect.template))?;
        }
        for (&slot, effect) in &self.falling_effects {
            insert(slot, effect.template.as_ref())?;
        }
        for (&slot, effect) in &self.particle_effects {
            insert(slot, effect.template.as_ref())?;
        }
        for (&slot, effect) in &self.collision_effects {
            insert(slot, effect.template.as_ref())?;
        }
        for &slot in self.fragment_effects.keys() {
            insert(slot, Some(&SavedEffect::Fragments))?;
        }
        Ok(records.into_iter().collect())
    }

    /// Candidate hosts and RNG only. The caller commits after the entire save validates.
    pub(crate) fn load_effect_pools(
        records: &[(u16, &[u8])],
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
        budget: &mut usize,
        x87_arithmetic: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut templates = BTreeMap::new();
        for &(slot, bytes) in records {
            if slot >= 4
                || templates
                    .insert(slot, SavedEffect::decode(bytes)?)
                    .is_some()
            {
                return Err(FormatError("saved particle slots overlap or exceed 0..3").into());
            }
        }
        let mut scene = Self::default();
        for (slot, template) in templates {
            if template == SavedEffect::Fragments {
                // 0x140043120 has no type-8 branch: it resets the type to zero.
                continue;
            }
            let image = image::load(source, template.image_name())?.map(|decoded| ObjectImage {
                name: template.image_name().to_vec(),
                pixels: decoded.pixels,
                alpha: decoded.alpha,
                filter: None,
                surface: false,
            });
            if let Some(image) = &image {
                *budget = budget
                    .checked_sub(image.pixels.pixels().len())
                    .ok_or(FormatError(
                        "saved particle images exceed the restore budget",
                    ))?;
            }
            match &template {
                SavedEffect::LegacyFlakes { words, name } => {
                    scene.legacy_flakes.insert(
                        slot as u8,
                        super::LegacyFlakes::new(
                            *words,
                            SceneObject {
                                image,
                                ..Default::default()
                            },
                            *name,
                            random,
                        ),
                    );
                }
                SavedEffect::Rain { words, name } | SavedEffect::LegacyRain { words, name } => {
                    let construct = if matches!(template, SavedEffect::LegacyRain { .. }) {
                        super::RainEffect::new_legacy
                    } else {
                        super::RainEffect::new
                    };
                    scene.rain_effects.insert(
                        slot as u8,
                        construct(
                            *words,
                            SceneObject {
                                image,
                                ..Default::default()
                            },
                            *name,
                        ),
                    );
                }
                SavedEffect::Falling { words, .. } => {
                    let mut effect = FallingEffect::new_with_arithmetic(
                        words[..4].try_into().unwrap(),
                        words[4],
                        words[5] != 0,
                        image,
                        random,
                        x87_arithmetic,
                    );
                    effect.template = Some(template);
                    scene.falling_effects.insert(slot as u8, effect);
                }
                SavedEffect::Particles { words, .. } => {
                    let mut effect = ParticleEffect::new_with_arithmetic(
                        words[0],
                        words[1..5].try_into().unwrap(),
                        words[5],
                        SceneObject {
                            image,
                            ..Default::default()
                        },
                        random,
                        x87_arithmetic,
                    );
                    effect.template = Some(template);
                    scene.particle_effects.insert(slot as u8, effect);
                }
                SavedEffect::Collision { words, .. }
                | SavedEffect::FadingParticles { words, .. } => {
                    // 0x425850 rebuilds type 7 through 0x432500, restarting its fade clock.
                    let mut effect = CollisionEffect::new_with_arithmetic(
                        words[0],
                        words[1..3].try_into().unwrap(),
                        words[3],
                        SceneObject {
                            image,
                            ..Default::default()
                        },
                        random,
                        x87_arithmetic,
                    );
                    if matches!(template, SavedEffect::FadingParticles { .. }) {
                        effect.fade = Some(super::ParticleFade::new(words[0]));
                    }
                    effect.template = Some(template);
                    scene.collision_effects.insert(slot as u8, effect);
                }
                SavedEffect::Fragments => unreachable!(),
            }
        }
        Ok(scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn puriring_effect_save_keeps_only_native_type_header_without_mutating_state() {
        let mut scene = SceneHost::default();
        let mut sprites = super::super::sprites::Sprites::new(Vec::new());
        sprites.request(8).unwrap();
        scene
            .legacy_effects
            .insert(2, super::super::LegacyEffect::Sprites(Box::new(sprites)));
        assert!(scene.saved_effect_pools().is_err());
        assert_eq!(
            scene.saved_effect_pools_with_legacy(true).unwrap(),
            vec![(2, vec![1, 0, 0x55, 0])]
        );
        let super::super::LegacyEffect::Sprites(sprites) = &scene.legacy_effects[&2] else {
            panic!()
        };
        assert_eq!(sprites.requested, 8);
    }

    #[derive(Default)]
    struct Images(Vec<Vec<u8>>);
    impl ImageSource for Images {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            self.0.push(name.to_vec());
            Ok(Some(b"NONE".to_vec()))
        }
    }
    #[test]
    fn fading_pool_saves_type_seven_template_and_restarts_its_fade_on_restore() {
        let f = f32::to_bits;
        let template = SavedEffect::FadingParticles {
            words: [0x0002_0a05, f(15.0), f(1000.0), 2],
            name: SavedEffect::name_field(b"snow").unwrap(),
        };
        let bytes = template.encode().unwrap();
        assert_eq!((&bytes[..4], bytes.len()), (&[1, 0, 7, 0][..], 52));
        let mut random = CrtRandom::new(0);
        let mut scene = SceneHost::load_effect_pools(
            &[(1, &bytes)],
            &mut Images::default(),
            &mut random,
            &mut 0,
            true,
        )
        .unwrap();
        let fade = scene.collision_effects[&1].fade.clone().unwrap();
        assert_eq!((fade.phase, fade.elapsed, fade.alpha), (2, 0, 255));
        scene
            .advance_effect_pools(1250, &mut random, &crate::camera::CameraHost::default())
            .unwrap();
        assert_eq!(
            scene.collision_effects[&1].fade.as_ref().unwrap().alpha,
            128
        );
        assert_eq!(scene.saved_effect_pools().unwrap(), vec![(1, bytes)]);
    }

    #[test]
    fn mixed_pools_reinitialize_in_slot_order_and_capture_only_the_creation_templates() {
        for x87 in [false, true] {
            let f = f32::to_bits;
            let templates = [
                (
                    2,
                    SavedEffect::Collision {
                        words: [0, f(15.0), f(1000.0), 1],
                        name: SavedEffect::name_field(b"collision").unwrap(),
                    },
                ),
                (
                    0,
                    SavedEffect::Falling {
                        words: [f(100.0), f(80.0), f(2000.0), f(5.0), 1, 7],
                        name: SavedEffect::name_field(b"falling").unwrap(),
                    },
                ),
                (3, SavedEffect::Fragments),
                (
                    1,
                    SavedEffect::Particles {
                        words: [0, f(40.0), f(20.0), f(-20.0), f(2000.0), 1],
                        name: SavedEffect::name_field(b"particles").unwrap(),
                    },
                ),
            ];
            let bytes: Vec<_> = templates
                .iter()
                .map(|(s, t)| (*s, t.encode().unwrap()))
                .collect();
            let records: Vec<_> = bytes.iter().map(|(s, b)| (*s, b.as_slice())).collect();
            let mut images = Images::default();
            let mut random = CrtRandom::new(0);
            let mut scene =
                SceneHost::load_effect_pools(&records, &mut images, &mut random, &mut 0, x87)
                    .unwrap();
            assert_eq!(
                images.0,
                [
                    b"falling".to_vec(),
                    b"particles".to_vec(),
                    b"collision".to_vec()
                ]
            );
            // Native initializers consume 4 / 8 / 5 CRT draws per particle respectively.
            let mut reference = CrtRandom::new(0);
            for _ in 0..17 {
                reference.next_value();
            }
            assert_eq!(random, reference);
            assert_eq!(scene.particle_effects[&1].x87_arithmetic, x87);
            assert!(scene.fragment_effects.is_empty());
            assert!(scene.falling_effects[&0].follow);
            let before = scene.saved_effect_pools().unwrap();
            scene
                .advance_effect_pools(500, &mut random, &crate::camera::CameraHost::default())
                .unwrap();
            assert_eq!(scene.saved_effect_pools().unwrap(), before);
            assert_eq!(
                before.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
                [0, 1, 2]
            );
            for (slot, body) in before {
                assert_eq!(body, bytes.iter().find(|(s, _)| *s == slot).unwrap().1);
            }
            let seed = random.clone();
            let mut invalid = records.clone();
            invalid.push((4, records[0].1));
            assert!(
                SceneHost::load_effect_pools(&invalid, &mut images, &mut random, &mut 0, x87)
                    .is_err()
            );
            assert_eq!(random, seed);
        }
    }
}

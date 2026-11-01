//! Identity-bound runtime rebuilding of Aoitori's creation-only 0x800 records.
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
    pub(crate) fn has_effect_pools(&self) -> bool {
        !self.particle_effects.is_empty()
            || !self.falling_effects.is_empty()
            || !self.collision_effects.is_empty()
            || !self.fragment_effects.is_empty()
    }

    pub(crate) fn saved_effect_pools(&self) -> Result<Vec<(u16, Vec<u8>)>, FormatError> {
        let mut records = BTreeMap::new();
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
                SavedEffect::Falling { words, .. } => {
                    let mut effect = FallingEffect::new(
                        words[..4].try_into().unwrap(),
                        words[4],
                        words[5] != 0,
                        image,
                        random,
                    );
                    effect.template = Some(template);
                    scene.falling_effects.insert(slot as u8, effect);
                }
                SavedEffect::Particles { words, .. } => {
                    let mut effect = ParticleEffect::new(
                        words[0],
                        words[1..5].try_into().unwrap(),
                        words[5],
                        SceneObject {
                            image,
                            ..Default::default()
                        },
                        random,
                    );
                    effect.template = Some(template);
                    scene.particle_effects.insert(slot as u8, effect);
                }
                SavedEffect::Collision { words, .. } => {
                    let mut effect = CollisionEffect::new(
                        words[0],
                        words[1..3].try_into().unwrap(),
                        words[3],
                        SceneObject {
                            image,
                            ..Default::default()
                        },
                        random,
                    );
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
    #[derive(Default)]
    struct Images(Vec<Vec<u8>>);
    impl ImageSource for Images {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            self.0.push(name.to_vec());
            Ok(Some(b"NONE".to_vec()))
        }
    }
    #[test]
    fn mixed_pools_reinitialize_in_slot_order_and_capture_only_the_creation_templates() {
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
            SceneHost::load_effect_pools(&records, &mut images, &mut random, &mut 0).unwrap();
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
        assert!(SceneHost::load_effect_pools(&invalid, &mut images, &mut random, &mut 0).is_err());
        assert_eq!(random, seed);
    }
}

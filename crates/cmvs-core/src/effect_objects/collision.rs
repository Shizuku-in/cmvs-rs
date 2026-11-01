//! Type-6 particles and scene-slot collision planes; executable-bound research semantics.
use super::particles::{hundredth, range as spread};
use crate::{
    bytecode::DecodeError,
    dispatch::Call,
    image::{self, ImageSource},
    random::CrtRandom,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
};
use std::{collections::BTreeMap, error::Error};

pub type Triangle = [[f32; 3]; 3];

/// Original line/triangle intersection (0x140051240): first intersected plane wins,
/// including below the ray's starting Y. Triangle edges are inclusive.
fn height(triangle: &Triangle, x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = *triangle;
    let subtract = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| a[i] - b[i]);
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // Keep the original reversed cross product and normalized plane arithmetic.
    let normal = cross(subtract(c, a), subtract(b, a));
    let length = dot(normal, normal).sqrt();
    let normal = normal.map(|v| v / length);
    let origin = [x, 35.0, z];
    let t = (normal[1] * (a[1] - origin[1])
        + normal[0] * (a[0] - origin[0])
        + normal[2] * (a[2] - origin[2]))
        / normal[1];
    let point = [0.0 * t + x, t + 35.0, 0.0 * t + z];
    for (start, end) in [(a, b), (b, c), (c, a)] {
        let side = cross(subtract(point, start), subtract(end, start));
        if normal[1] * side[1] + normal[0] * side[0] + normal[2] * side[2] < 0.0 {
            return None;
        }
    }
    Some(point[1])
}

fn unit(random: &mut CrtRandom) -> f32 {
    random.next_value() as f32 / 32767.0
}
#[derive(Debug, PartialEq, Eq)]
pub struct CollisionParticle {
    /// Native 44-byte record. Words 6/7/8 are XYZ, 9 speed, 10 cached contact Y.
    pub words: [u32; 11],
    pub object: SceneObject,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CollisionEffect {
    pub mode: u32,
    pub parameters: [u32; 2],
    pub pool: SceneObject,
    pub particles: Vec<CollisionParticle>,
}

impl CollisionEffect {
    pub(crate) fn new(
        mode: u32,
        parameters: [u32; 2],
        count: u32,
        pool: SceneObject,
        random: &mut CrtRandom,
    ) -> Self {
        let [base_z, speed] = parameters.map(f32::from_bits);
        let particles = (0..count)
            .map(|index| {
                let x = unit(random) * 90.0 - 45.0;
                let z = unit(random) * spread(mode) + base_z;
                let y = unit(random) * 75.0;
                random.next_value(); // Discarded by the original before speed randomization.
                let mut words = [0; 11];
                words[0] = 1;
                words[1] = index;
                words[6] = hundredth(x).to_bits();
                words[7] = hundredth(y).to_bits();
                words[8] = hundredth(z).to_bits();
                let variation = speed * 0.25;
                words[9] = ((unit(random) * variation - variation * 0.5) + speed).to_bits();
                let mut draw = DrawState::default();
                draw.words[0] = 2;
                draw.words[2..8].copy_from_slice(&[0, 0, 32, 32, 16, 16]);
                draw.words[11..14].copy_from_slice(&words[6..9]);
                draw.words[15] = match mode >> 28 {
                    4 => 60.0,
                    5 => 50.0,
                    6 => 40.0,
                    7 => 30.0,
                    8 => 20.0,
                    _ => base_z + 5.0,
                }
                .to_bits();
                CollisionParticle {
                    words,
                    object: SceneObject {
                        draw: Some(draw),
                        flags: 0,
                        ..Default::default()
                    },
                }
            })
            .collect();
        Self {
            mode,
            parameters,
            pool,
            particles,
        }
    }

    fn respawn(
        words: &mut [u32; 11],
        draw: &mut DrawState,
        mode: u32,
        base_z: f32,
        random: &mut CrtRandom,
    ) {
        words[6] = hundredth(unit(random) * 90.0 - 45.0).to_bits();
        let z = hundredth(unit(random) * spread(mode) + base_z);
        words[7] = (f32::from_bits(words[7]) + 75.0).to_bits();
        words[8] = z.to_bits();
        words[10] = 0;
        draw.words[17] = 255;
    }

    pub(crate) fn advance(&mut self, delta: u32, triangles: &[Triangle], random: &mut CrtRandom) {
        let time = delta.min(120) as f32 * 10.0;
        let base_z = f32::from_bits(self.parameters[0]);
        for particle in &mut self.particles {
            let words = &mut particle.words;
            let x = f32::from_bits(words[6]);
            let y = f32::from_bits(words[7]) - time / f32::from_bits(words[9]);
            let z = f32::from_bits(words[8]);
            words[7] = y.to_bits();
            let contact = f32::from_bits(words[10]);
            let draw = particle.object.draw.as_mut().expect("particle draw");
            if !triangles.is_empty() && contact >= 1.0 {
                let difference = y - contact;
                if difference <= -0.5 {
                    Self::respawn(words, draw, self.mode, base_z, random);
                } else if difference <= 0.5 {
                    draw.words[17] = ((difference + 0.5) * 255.0) as u32;
                }
            } else if !triangles.is_empty()
                && let Some(hit) = triangles.iter().find_map(|triangle| height(triangle, x, z))
            {
                let difference = y - hit;
                if difference > -0.5 && difference <= 0.5 {
                    draw.words[17] = ((difference + 0.5) * 255.0) as u32;
                    words[10] = hit.to_bits();
                }
            }
            if f32::from_bits(words[7]) < 0.0 {
                Self::respawn(words, draw, self.mode, base_z, random);
            }
            draw.words[11..14].copy_from_slice(&words[6..9]);
        }
    }
}

impl SceneHost {
    pub fn collision_effects(&self) -> &BTreeMap<u8, CollisionEffect> {
        &self.collision_effects
    }

    /// Only scene slots 20..27 contribute; translation is the object's own draw XYZ.
    pub(crate) fn collision_triangles(&self) -> Result<Vec<Triangle>, &'static str> {
        let mut vertices = Vec::new();
        for slot in 20..28 {
            let Some(object) = self.object(slot) else {
                continue;
            };
            let (Some(points), Some(draw)) = (&object.collision_vertices, &object.draw) else {
                continue;
            };
            if vertices.len() + points.len() > 256 {
                return Err("collision vertex count exceeds original frame buffer");
            }
            let offset = [draw.words[11], draw.words[12], draw.words[13]].map(f32::from_bits);
            for point in points {
                let xyz = std::array::from_fn(|i| offset[i] + f32::from_bits(point[i]));
                if xyz.iter().any(|v| !v.is_finite()) {
                    return Err("non-finite collision vertex");
                }
                vertices.push(xyz);
            }
        }
        if vertices.len() % 3 != 0 {
            return Err("collision vertex stream has an incomplete triangle");
        }
        Ok(vertices.as_chunks::<3>().0.to_vec())
    }

    pub(crate) fn handle_collision_mesh(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<(), DecodeError> {
        let raw = call.argument(4, "object slot")?;
        let point = if call.command == 0x332 {
            Some([
                call.argument(8, "collision x")?,
                call.argument(12, "collision y")?,
                call.argument(16, "collision z")?,
            ])
        } else {
            None
        };
        let slot =
            u8::try_from(raw).map_err(|_| call.error("collision object slot outside 0..255"))?;
        if self.object(slot).is_none() {
            return Err(call.error("collision mesh needs an existing scene object"));
        }
        if point.is_some_and(|p| p.iter().any(|&v| !f32::from_bits(v).is_finite())) {
            return Err(call.error("non-finite collision vertex"));
        }
        call.complete(if point.is_some() { 0x4010 } else { 0x4004 })?;
        let object = self.object_mut(slot).expect("validated object");
        match call.command {
            0x330 => object.collision_vertices = Some(Vec::new()),
            0x331 => object.collision_vertices = None,
            _ => {
                if let Some(points) = &mut object.collision_vertices
                    && points.len() < 128
                {
                    points.push(point.expect("vertex"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn create_collision_effect(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let name_id = call.argument(8, "particle image")?;
        let mode = call.argument(12, "particle mode")?;
        let parameters = [
            call.argument(16, "particle base z")?,
            call.argument(20, "particle speed")?,
        ];
        let count = call.argument(24, "particle count")?;
        let name = call.vm.resolve_string(name_id)?;
        if slot >= 4 {
            call.complete(0x4018)?;
            return Ok(());
        }
        if count > 16_384
            || parameters.iter().any(|&v| !f32::from_bits(v).is_finite())
            || f32::from_bits(parameters[1]) <= 0.0
        {
            return Err(call.error("collision particles need bounded count and finite parameters with positive speed").into());
        }
        self.collision_triangles().map_err(|e| call.error(e))?;
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
        let effect = CollisionEffect::new(mode, parameters, count, pool, &mut candidate_random);
        call.complete(0x4018)?;
        self.particle_effects.remove(&(slot as u8));
        self.falling_effects.remove(&(slot as u8));
        self.fragment_effects.remove(&(slot as u8));
        self.collision_effects.insert(slot as u8, effect);
        *random = candidate_random;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(count: u32) -> CollisionEffect {
        CollisionEffect::new(
            0,
            [15.0f32.to_bits(), 5000.0f32.to_bits()],
            count,
            SceneObject::default(),
            &mut CrtRandom::new(0),
        )
    }
    #[test]
    fn initialization_matches_float32_reference_and_consumes_five_draws() {
        let mut random = CrtRandom::new(0);
        let effect = CollisionEffect::new(
            0,
            [15.0f32.to_bits(), 5000.0f32.to_bits()],
            2,
            SceneObject::default(),
            &mut random,
        );
        assert_eq!(
            effect.particles[0].words,
            [
                1, 0, 0, 0, 0, 0, 3258158940, 1111650468, 1100852756, 1167279722, 0
            ]
        );
        assert_eq!(
            effect.particles[1].words,
            [
                1, 1, 0, 0, 0, 0, 3242815652, 1116981166, 1101057229, 1168979540, 0
            ]
        );
        let mut expected = CrtRandom::new(0);
        for _ in 0..10 {
            expected.next_value();
        }
        assert_eq!(random.next_value(), expected.next_value());
        assert_eq!(
            &effect.particles[0].object.draw.as_ref().unwrap().words[2..8],
            &[0, 0, 32, 32, 16, 16]
        );
    }
    #[test]
    fn collisions_use_first_plane_then_fade_and_respawn_with_two_draws() {
        let triangle = [[-2.0, 2.0, -2.0], [2.0, 2.0, -2.0], [0.0, 2.0, 2.0]];
        assert_eq!(height(&triangle, 0.0, 0.0), Some(2.0));
        assert_eq!(height(&triangle, 0.0, -2.0), Some(2.0));
        assert_eq!(height(&triangle, 5.0, 0.0), None);
        let mut a = effect(1);
        a.particles[0].words[6..10].copy_from_slice(&[0, 2.25f32.to_bits(), 0, 100.0f32.to_bits()]);
        let mut random = CrtRandom::new(0);
        a.advance(0, &[triangle], &mut random);
        assert_eq!(a.particles[0].object.draw.as_ref().unwrap().words[17], 191);
        assert_eq!(a.particles[0].words[10], 2.0f32.to_bits());
        a.advance(8, &[triangle], &mut random);
        assert_eq!(a.particles[0].words[10], 0);
        assert!((f32::from_bits(a.particles[0].words[7]) - 76.45).abs() < 0.00001);
        let mut expected = CrtRandom::new(0);
        expected.next_value();
        expected.next_value();
        assert_eq!(random.next_value(), expected.next_value());
        // A higher, earlier plane prevents a later low plane from catching this particle.
        let high = triangle.map(|mut v| {
            v[1] = 10.0;
            v
        });
        a.particles[0].words[6..11].copy_from_slice(&[
            0,
            2.25f32.to_bits(),
            0,
            100.0f32.to_bits(),
            0,
        ]);
        a.advance(0, &[high, triangle], &mut random);
        assert_eq!(a.particles[0].words[10], 0);
        let mut b = effect(1);
        let mut c = effect(1);
        let mut rb = CrtRandom::new(0);
        let mut rc = rb.clone();
        b.advance(120, &[], &mut rb);
        c.advance(1000, &[], &mut rc);
        assert_eq!(b, c);
        assert_eq!(rb.next_value(), rc.next_value());
    }
    #[test]
    fn mesh_collection_uses_only_twenty_to_twenty_seven_and_rejects_unsafe_streams() {
        let point = [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()];
        let mut scene = SceneHost::default();
        for slot in [19, 20, 28] {
            let mut draw = DrawState::default();
            draw.words[11..14].copy_from_slice(&[
                10.0f32.to_bits(),
                20.0f32.to_bits(),
                30.0f32.to_bits(),
            ]);
            scene.replace_object(
                slot,
                SceneObject {
                    collision_vertices: Some(vec![point; 3]),
                    draw: Some(draw),
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            scene.collision_triangles().unwrap(),
            vec![[[11.0, 22.0, 33.0]; 3]]
        );
        assert!(crate::snapshot::SavedScene::from_object(scene.object(20).unwrap()).is_err());
        scene
            .object_mut(20)
            .unwrap()
            .collision_vertices
            .as_mut()
            .unwrap()
            .pop();
        assert!(scene.collision_triangles().is_err());
    }

    #[test]
    fn mixed_pool_respawns_consume_random_in_slot_order() {
        let mut scene = SceneHost::default();
        let mut collision = effect(1);
        collision.particles[0].words[7] = (-1.0f32).to_bits();
        scene.collision_effects.insert(0, collision);
        let mut words = [0; 12];
        words[8] = (-1.0f32).to_bits();
        words[11] = 1.0f32.to_bits();
        scene.particle_effects.insert(
            1,
            crate::effect_objects::ParticleEffect {
                mode: 0,
                parameters: [0; 4],
                pool: SceneObject::default(),
                particles: vec![crate::effect_objects::Particle {
                    words,
                    object: SceneObject {
                        draw: Some(DrawState::default()),
                        ..Default::default()
                    },
                }],
            },
        );
        let mut random = CrtRandom::new(0);
        scene
            .advance_effect_pools(0, &mut random, &Default::default())
            .unwrap();
        let mut expected = CrtRandom::new(0);
        let collision_x = hundredth(unit(&mut expected) * 90.0 - 45.0);
        expected.next_value();
        let particle_x = hundredth(unit(&mut expected) * 120.0 - 60.0);
        expected.next_value();
        assert_eq!(
            scene.collision_effects[&0].particles[0].words[6],
            collision_x.to_bits()
        );
        assert_eq!(
            scene.particle_effects[&1].particles[0].words[7],
            particle_x.to_bits()
        );
        assert_eq!(random.next_value(), expected.next_value());
    }
}

//! PURIRING type 0x54, 0x41ddc0/0x41db10 geometry and 0x41d5a0 animation.
use super::legacy::{LegacyMesh, MeshScene};
use crate::{audio::AudioBackend, image::ImageSource, random::CrtRandom, scene::ObjectImage};
use std::error::Error;
#[derive(Debug, PartialEq, Eq)]
pub struct Spark {
    pub xyz: [u32; 3],
    speed: u32,
    delay: u32,
    clock: u32,
    pub alpha: u32,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Magic {
    pub scene: MeshScene,
    pub sparks: Vec<Spark>,
    pub spark_image: ObjectImage,
    alternate: ObjectImage,
    pub state: u32,
    pub requested: i32,
    clock: u32,
    reveal: u32,
    fade: u32,
    played: bool,
}
impl Magic {
    pub fn new(
        source: &mut dyn ImageSource,
        random: &mut CrtRandom,
    ) -> Result<Self, Box<dyn Error>> {
        let mut images = Vec::new();
        for name in [
            b"try008a.pb3",
            b"try008b.pb3",
            b"try008c.pb3",
            b"try008d.pb3",
            b"try008e.pb3",
        ] {
            images.push(super::curtain::load(source, name)?);
        }
        let spark_image = super::curtain::load(source, b"try008z.pb3")?;
        let alternate = super::curtain::load(source, b"try008z2.pb3")?;
        let meshes = [0.0, 0.1, 0.3, 0.4, 0.5]
            .into_iter()
            .enumerate()
            .map(|(i, y)| {
                let mut mesh = LegacyMesh::new(
                    i,
                    [
                        [-1.0, y, 1.0],
                        [1.0, y, 1.0],
                        [-1.0, y, -1.0],
                        [1.0, y, -1.0],
                    ],
                    [-0.16, 0.97, 2.0],
                )
                .blended(true);
                mesh.rotation[0] = -90.0;
                mesh.alpha = 0;
                mesh
            })
            .collect();
        let sparks = (0..512)
            .map(|_| {
                let x = (f64::from(random.next_value()) / 32767.0 * 4.0 - 2.0) as f32;
                let r = f64::from(random.next_value()) / 32767.0;
                let z = (r + r + 1.0) as f32;
                let y = (f64::from(random.next_value()) / 32767.0 * 10.0 - 5.0) as f32;
                let speed =
                    (f64::from(random.next_value()) / 32767.0 * 2000.0 - 1000.0) as f32 + 8000.0;
                Spark {
                    xyz: [x, y, z].map(f32::to_bits),
                    speed: speed.to_bits(),
                    delay: random.next_value() % 1000,
                    clock: 0,
                    alpha: 0,
                }
            })
            .collect();
        Ok(Self {
            scene: MeshScene { images, meshes },
            sparks,
            spark_image,
            alternate,
            state: 0,
            requested: -1,
            clock: 0,
            reveal: 0,
            fade: 0,
            played: false,
        })
    }
    pub fn request(&mut self, value: u32) {
        match value {
            10 | 11 | 20 | 22 => self.requested = value as i32,
            50 => self.spark_image = self.alternate.clone(),
            _ => {}
        }
    }
    pub fn advance(
        &mut self,
        delta: u32,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        let delta = delta.min(200);
        self.clock = self.clock.wrapping_add(delta);
        let angle = (self.clock.wrapping_mul(3600) / 6000 % 3600) as f32 / 10.0;
        for (i, mesh) in self.scene.meshes.iter_mut().enumerate() {
            mesh.rotation[1] = if i == 0 || i == 2 {
                angle
            } else {
                360.0 - angle
            };
        }
        if self.requested != -1 && self.requested as u32 != self.state {
            self.state = self.requested as u32;
            self.requested = -1;
        }
        if self.state < 10 || self.state == 11 {
            self.reveal = self.reveal.wrapping_add(delta);
            let time = if self.state == 11 {
                Some(self.reveal)
            } else if self.reveal > 1500 {
                Some(self.reveal - 1500)
            } else {
                None
            };
            if let Some(time) = time {
                let mut complete = 0;
                for (i, mesh) in self.scene.meshes.iter_mut().enumerate() {
                    let delay = 200 * (4 - i as u32);
                    if time > delay {
                        if !self.played {
                            backend.play_preloaded(16, false, volume)?;
                            self.played = true;
                        }
                        let t = time - delay;
                        let duration = 500 + 50 * i as u32;
                        if t >= duration {
                            mesh.scale = 1.0;
                            mesh.alpha = 256;
                            complete += 1;
                        } else {
                            mesh.scale = t as f32 / duration as f32;
                            mesh.alpha = (t << 8) / duration;
                        }
                    }
                }
                if self.state == 11 && complete == 5 {
                    self.state = 12;
                }
            }
        }
        if matches!(self.state, 20 | 22) {
            self.fade = self.fade.wrapping_add(delta);
        }
        for mesh in &mut self.scene.meshes {
            if self.state == 20 {
                if self.fade >= 1000 {
                    self.state = 21;
                    mesh.alpha = 0;
                } else {
                    mesh.alpha = 256 - (self.fade << 8) / 1000;
                }
            } else if self.state == 21 {
                mesh.alpha = 0;
            }
        }
        for spark in &mut self.sparks {
            let y = f32::from_bits(spark.xyz[1])
                + (f64::from(delta) * 10.0 / f64::from(f32::from_bits(spark.speed))) as f32;
            spark.xyz[1] = (if y >= 5.0 { y - 10.0 } else { y }).to_bits();
            if spark.delay != 0 {
                spark.delay = spark.delay.saturating_sub(delta);
            } else {
                spark.clock = spark.clock.wrapping_add(delta);
            }
            if spark.clock == 0 {
                continue;
            }
            let t = spark.clock;
            let mut alpha = if t < 500 {
                (t << 7) / 500
            } else {
                let p = (t - 500) % 1000;
                if p < 500 {
                    (p << 7) / 500 + 128
                } else {
                    256 - ((p - 500) << 7) / 500
                }
            };
            if matches!(self.state, 20 | 22) {
                if self.fade >= 1000 {
                    self.state += 1;
                    alpha = 0;
                } else {
                    alpha -= alpha * self.fade / 1000;
                }
            } else if matches!(self.state, 21 | 23) {
                alpha = 0;
            }
            spark.alpha = alpha.min(255);
        }
        for mesh in &mut self.scene.meshes {
            mesh.alpha = mesh.alpha.min(255);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect() -> Magic {
        let image = ObjectImage {
            filter: None,
            name: vec![],
            pixels: crate::raster::RgbaImage::new(1, 1, vec![255; 4]).unwrap(),
            alpha: true,
            surface: false,
        };
        Magic {
            scene: MeshScene {
                images: vec![image.clone(); 5],
                meshes: (0..5)
                    .map(|i| {
                        let mut m = LegacyMesh::new(i, [[0.0; 3]; 4], [0.0; 3]);
                        m.alpha = 0;
                        m
                    })
                    .collect(),
            },
            sparks: vec![Spark {
                xyz: [0f32, 4.99, 2.0].map(f32::to_bits),
                speed: 8000f32.to_bits(),
                delay: 200,
                clock: 0,
                alpha: 0,
            }],
            spark_image: image.clone(),
            alternate: image,
            state: 0,
            requested: -1,
            clock: 0,
            reveal: 0,
            fade: 0,
            played: false,
        }
    }
    #[test]
    fn magic_layers_reveal_in_order_and_particle_only_fade_preserves_them() {
        let mut effect = effect();
        let mut audio = crate::audio::RecordingAudio::default();
        effect.request(10);
        effect.advance(200, &mut audio, 128).unwrap();
        assert_eq!(effect.state, 10);
        assert_eq!(effect.reveal, 0);
        assert_eq!(effect.sparks[0].clock, 0);
        assert!(f32::from_bits(effect.sparks[0].xyz[1]) < 0.0);
        effect.request(11);
        effect.advance(200, &mut audio, 128).unwrap();
        assert_eq!(effect.scene.meshes[0].alpha, 0);
        assert!(effect.scene.meshes[4].alpha > 0);
        for _ in 0..6 {
            effect.advance(200, &mut audio, 128).unwrap();
        }
        assert_eq!(effect.state, 12);
        assert!(effect.scene.meshes.iter().all(|m| m.alpha == 255));
        assert_eq!(audio.requests().len(), 1);
        effect.request(22);
        for _ in 0..5 {
            effect.advance(200, &mut audio, 128).unwrap();
        }
        assert_eq!(effect.state, 23);
        assert_eq!(effect.sparks[0].alpha, 0);
        assert!(effect.scene.meshes.iter().all(|m| m.alpha == 255));
    }
}

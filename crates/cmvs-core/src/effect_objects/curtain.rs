//! PURIRING type 0x53: 0x41c630/0x41ceb0 geometry, 0x41cbb0 update, 0x42b400 reveal.
use super::legacy::{LegacyMesh, MeshScene, quarter};
use crate::{
    image::{self, ImageSource},
    raster::RgbaImage,
    scene::{DrawState, ObjectImage, SceneObject},
};
use std::error::Error;

#[derive(Debug, PartialEq, Eq)]
pub struct Curtain {
    pub scene: MeshScene,
    pub background: SceneObject,
    pub state: u32,
    pub requested: i32,
    visible: bool,
    clock: u32,
    phase: u32,
    action_clock: u32,
    angle: u32,
    reveal: Option<u32>,
    masks: [Vec<u8>; 2],
    alpha: [Vec<u8>; 2],
}
pub(super) fn load(
    source: &mut dyn ImageSource,
    name: &[u8],
) -> Result<ObjectImage, Box<dyn Error>> {
    let decoded = image::load(source, name)?.ok_or("curtain image did not decode")?;
    Ok(ObjectImage {
        filter: None,
        name: name.to_vec(),
        pixels: decoded.pixels,
        alpha: decoded.alpha,
        surface: false,
    })
}
impl Curtain {
    pub fn new(source: &mut dyn ImageSource) -> Result<Self, Box<dyn Error>> {
        let mut images = Vec::new();
        let mut masks = [Vec::new(), Vec::new()];
        let mut alpha = [Vec::new(), Vec::new()];
        for i in 0..2 {
            let mut image = load(source, [b"try007c3.pb3", b"try007c4.pb3"][i])?;
            let fallback = image.pixels.width() != 512;
            if fallback {
                image = load(source, [b"try007c1.pb3", b"try007c2.pb3"][i])?;
            }
            let mask = load(
                source,
                if fallback {
                    [b"try007d1.pb3", b"try007d2.pb3"][i]
                } else {
                    [b"try007d3.pb3", b"try007d4.pb3"][i]
                },
            )?;
            if image.pixels.width() != mask.pixels.width()
                || image.pixels.height() != mask.pixels.height()
            {
                return Err("curtain mask dimensions differ".into());
            }
            masks[i] = mask
                .pixels
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[1])
                .collect();
            alpha[i] = image
                .pixels
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[3])
                .collect();
            image.alpha = true;
            images.push(image);
        }
        let mut draw = DrawState::default();
        draw.words[0] = 1;
        draw.words[4..6].copy_from_slice(&[1024, 614]);
        draw.words[10] = 64;
        let background = SceneObject {
            image: Some(load(source, b"__try007a.pb3")?),
            draw: Some(draw),
            ..Default::default()
        };
        let meshes = vec![
            LegacyMesh::new(
                0,
                [
                    [-20.0, 12.5, 0.0],
                    [0.0, 12.5, 0.0],
                    [-20.0, -7.5, 0.0],
                    [0.0, -7.5, 0.0],
                ],
                [-0.3, 0.0, 28.0],
            )
            .blended(true),
            LegacyMesh::new(
                1,
                [
                    [0.0, 12.5, 0.0],
                    [20.0, 12.5, 0.0],
                    [0.0, -7.5, 0.0],
                    [20.0, -7.5, 0.0],
                ],
                [0.1, 0.0, 28.0],
            )
            .blended(true),
        ];
        Ok(Self {
            scene: MeshScene { images, meshes },
            background,
            state: 0,
            requested: -1,
            visible: false,
            clock: 0,
            phase: 0,
            action_clock: 0,
            angle: 0,
            reveal: None,
            masks,
            alpha,
        })
    }
    pub fn quads(&self) -> Vec<super::legacy::WorldQuad<'_>> {
        if self.visible {
            self.scene.quads()
        } else {
            Vec::new()
        }
    }
    pub fn advance(&mut self, delta: u32) -> Result<(), Box<dyn Error>> {
        if self.state == 0 && self.requested > 0 {
            self.state = self.requested as u32;
            self.requested = -1;
        }
        match self.state {
            1 => {
                self.reveal = Some(0);
                self.visible = true;
                self.state = 10;
            }
            2 => {
                self.visible = false;
                self.state = 0;
            }
            3 => {
                self.visible = true;
                self.state = 0;
            }
            4 | 5 => {
                self.action_clock = 0;
                self.state = if self.state == 4 { 11 } else { 12 };
            }
            10 if self.reveal.is_none() => self.state = 0,
            11 => {
                self.action_clock = self.action_clock.wrapping_add(delta);
                self.angle = if self.action_clock >= 800 {
                    self.state = 0;
                    5f32.to_bits()
                } else {
                    quarter((f64::from(self.action_clock) * 90.0 / 800.0) as f32, 5.0)
                        .1
                        .to_bits()
                };
            }
            12 => {
                self.action_clock = self.action_clock.wrapping_add(delta);
                if self.action_clock >= 500 {
                    self.visible = false;
                    self.state = 0;
                } else {
                    for mesh in &mut self.scene.meshes {
                        mesh.alpha = 256 - (self.action_clock << 8) / 500;
                    }
                }
            }
            _ => {}
        }
        self.clock = self.clock.wrapping_add(delta);
        let offset = match self.phase {
            0 | 2 => {
                let t = (self.clock as f64 / 2000.0).min(1.0) as f32;
                let value = if self.phase == 0 { t } else { 1.0 - t };
                if self.clock >= 2000 {
                    self.clock = 0;
                    self.phase += 1;
                }
                value
            }
            1 => {
                if self.clock > 200 {
                    self.clock = 0;
                    self.phase = 2;
                }
                1.0
            }
            _ => {
                if self.clock > 200 {
                    self.clock = 0;
                    self.phase = 0;
                }
                0.0
            }
        };
        let angle = f32::from_bits(self.angle);
        self.scene.meshes[0].rotation[1] = angle + offset;
        self.scene.meshes[1].rotation[1] = angle - offset;
        // 0x41c710 updates the pixel masks after the effect state logic.
        if let Some(elapsed) = self.reveal {
            let elapsed = elapsed.wrapping_add(delta).min(5000);
            for i in 0..2 {
                let image = &mut self.scene.images[i];
                let mut pixels = image.pixels.pixels().to_vec();
                for ((pixel, &mask), &alpha) in pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(&self.masks[i])
                    .zip(&self.alpha[i])
                {
                    let start = u32::from(mask) * 4000 / 255;
                    pixel[3] =
                        (u32::from(alpha) * elapsed.saturating_sub(start).min(1000) / 1000) as u8;
                }
                image.pixels = RgbaImage::new(image.pixels.width(), image.pixels.height(), pixels)?;
            }
            self.reveal = if elapsed == 5000 { None } else { Some(elapsed) };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect() -> Curtain {
        let image = || ObjectImage {
            filter: None,
            name: vec![],
            pixels: RgbaImage::new(2, 1, vec![255; 8]).unwrap(),
            alpha: true,
            surface: false,
        };
        Curtain {
            scene: MeshScene {
                images: vec![image(), image()],
                meshes: vec![
                    LegacyMesh::new(0, [[0.0; 3]; 4], [0.0; 3]),
                    LegacyMesh::new(1, [[0.0; 3]; 4], [0.0; 3]),
                ],
            },
            background: SceneObject::default(),
            state: 0,
            requested: -1,
            visible: false,
            clock: 0,
            phase: 0,
            action_clock: 0,
            angle: 0,
            reveal: None,
            masks: [vec![0, 255], vec![0, 255]],
            alpha: [vec![128, 255], vec![128, 255]],
        }
    }
    #[test]
    fn reveal_uses_mask_delays_and_becomes_idle_on_the_following_frame() {
        let mut effect = effect();
        effect.requested = 1;
        effect.advance(500).unwrap();
        assert_eq!(effect.state, 10);
        assert_eq!(effect.scene.images[0].pixels.pixels()[3], 64);
        assert_eq!(effect.scene.images[0].pixels.pixels()[7], 0);
        effect.advance(4000).unwrap();
        assert_eq!(effect.scene.images[0].pixels.pixels()[3], 128);
        assert_eq!(effect.scene.images[0].pixels.pixels()[7], 127);
        effect.advance(500).unwrap();
        assert_eq!(effect.state, 10);
        assert_eq!(effect.scene.images[0].pixels.pixels()[7], 255);
        effect.advance(16).unwrap();
        assert_eq!(effect.state, 0);
        effect.requested = 5;
        effect.advance(16).unwrap();
        effect.advance(250).unwrap();
        assert_eq!(effect.scene.meshes[0].alpha, 128);
        effect.advance(250).unwrap();
        assert_eq!(effect.state, 0);
        assert!(effect.quads().is_empty());
    }
}

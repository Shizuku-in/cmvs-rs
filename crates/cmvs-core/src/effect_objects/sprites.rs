//! PURIRING type 0x55, 0x418bd0/0x418fa0/0x41ab40.
use crate::{
    audio::AudioBackend,
    scene::{
        Animation, AnimationFrame, DrawState, MotionKey, MotionPlayback, ObjectImage, RotationKey,
        RotationTrack, SceneObject, WorldMotion,
    },
};
use std::error::Error;

#[derive(Debug, PartialEq, Eq)]
pub struct Sprites {
    pub objects: [SceneObject; 3],
    pub state: u32,
    pub requested: i32,
    // The original stores target coordinates independently of the live draw position.
    positions: [[u32; 3]; 3],
    base: [u32; 3],
    phase: u32,
    clock: u32,
    sound_phase: bool,
    second_clock: u32,
    exit_clock: u32,
    segment: u32,
    exit_segment: Option<u32>,
    second_started: bool,
    exit_sound: bool,
}
impl Sprites {
    pub fn new(images: Vec<ObjectImage>) -> Self {
        let mut images = images.into_iter();
        Self {
            objects: std::array::from_fn(|_| SceneObject {
                image: images.next(),
                shown: false,
                ..Default::default()
            }),
            state: 0,
            requested: 0,
            positions: [[0; 3]; 3],
            base: [0; 3],
            phase: 0,
            clock: 0,
            sound_phase: false,
            second_clock: 0,
            exit_clock: 0,
            segment: 0,
            exit_segment: None,
            second_started: false,
            exit_sound: false,
        }
    }
    fn initialize(&mut self, index: usize) {
        let object = &mut self.objects[index];
        let mut draw = DrawState::default();
        draw.words[0] = 2;
        draw.words[15] = 5f32.to_bits();
        let mut animation = Animation::default();
        for frame in 0..[24, 12, 8][index] {
            animation.frames[frame] = AnimationFrame {
                next: (frame / 4 * 4 + (frame + 1) % 4) as i32,
                words: [
                    (frame / 4 * 256) as i16,
                    (frame % 4 * 192) as i16,
                    256,
                    192,
                    128,
                    96,
                ],
                duration: 100,
            };
        }
        animation.start(0, &mut draw.words);
        draw.animation = Some(Box::new(animation));
        if index == 1 {
            let mut track = RotationTrack {
                playing: true,
                base: 0,
                start: 0,
                current: 0,
                elapsed: 0,
                keys: [RotationKey::default(); 8],
            };
            for (key, (next, mode, duration, value)) in track.keys.iter_mut().zip([
                (1, 1, 500, 500),
                (2, 0x8001, 1000, -500),
                (1, 1, 1000, 500),
            ]) {
                *key = RotationKey {
                    next,
                    mode,
                    duration,
                    value,
                };
            }
            draw.rotation_track = Some(Box::new(track));
        }
        object.draw = Some(draw);
        object.shown = true;
        self.position(
            index,
            [[-4.0, 1.5, 7.0], [3.3, 4.0, 5.0], [3.0, 3.5, 3.0]][index],
        );
    }
    fn position(&mut self, index: usize, xyz: [f32; 3]) {
        self.positions[index] = xyz.map(f32::to_bits);
        if let Some(draw) = self.objects[index].draw.as_mut() {
            draw.words[11..14].copy_from_slice(&xyz.map(f32::to_bits));
        }
    }
    fn move_to(&mut self, index: usize, xyz: [f32; 3], mode: u32, duration: u32) {
        self.positions[index] = xyz.map(f32::to_bits);
        if let Some(draw) = self.objects[index].draw.as_mut() {
            let mut motion = WorldMotion {
                legacy_20070119: true,
                ..Default::default()
            };
            motion.keys[0] = MotionKey {
                next: -1,
                xyz: xyz.map(f32::to_bits),
                mode,
                duration,
            };
            motion.playback = Some(MotionPlayback {
                base: draw.words[11..14].try_into().unwrap(),
                start: 0,
                current: 0,
                elapsed: 0,
            });
            draw.world_motion = Some(Box::new(motion));
        }
    }
    fn group(&mut self, index: usize, group: usize) {
        if let Some(draw) = self.objects[index].draw.as_mut()
            && let Some(animation) = draw.animation.as_mut()
        {
            animation.start(group * 4, &mut draw.words);
        }
    }
    fn rotations(&mut self, index: usize, keys: &[(i32, u32, u32, i32)]) {
        if let Some(draw) = self.objects[index].draw.as_mut() {
            let mut track = RotationTrack {
                playing: true,
                base: draw.words[20] as i32,
                start: 0,
                current: 0,
                elapsed: 0,
                keys: [RotationKey::default(); 8],
            };
            for (key, &(next, mode, duration, degrees)) in track.keys.iter_mut().zip(keys) {
                *key = RotationKey {
                    next,
                    mode,
                    duration,
                    value: degrees * 100,
                };
            }
            draw.rotation_track = Some(Box::new(track));
        }
    }
    fn moving(&self, index: usize) -> bool {
        self.objects[index]
            .draw
            .as_ref()
            .is_some_and(|d| d.world_motion.is_some())
    }
    fn stop_rotation(&mut self, index: usize) {
        if let Some(draw) = self.objects[index].draw.as_mut() {
            draw.rotation_track = None;
            draw.words[20] = 0;
        }
    }
    /// 0x41a6e0: approach, then two small steps per 700 ms; request 10 exits the loop.
    fn stepping(
        &mut self,
        delta: u32,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        if self.phase == 0 {
            self.position(0, [6.0, 3.3, 5.0]);
            self.move_to(0, [0.5, 3.3, 5.0], 0, 800);
            self.phase = 1;
            self.sound_phase = false;
        }
        if self.phase == 1 && !self.moving(0) {
            self.phase = 2;
            self.rotations(
                0,
                &[
                    (1, 0, 100, 5),
                    (2, 0x8000, 100, 0),
                    (3, 0, 100, 5),
                    (4, 0x8000, 100, 0),
                    (0, 255, 300, 0),
                ],
            );
            self.base = self.positions[0];
        }
        if self.phase == 2 {
            self.clock = self.clock.wrapping_add(delta);
            let t = self.clock % 700;
            if (t < 100 && !self.sound_phase) || ((200..300).contains(&t) && self.sound_phase) {
                backend.play_preloaded(16, false, volume)?;
                self.sound_phase = !self.sound_phase;
            }
            if f32::from_bits(self.positions[0][0]) > -1.0 {
                let first = f64::from(self.clock / 700) * f64::from(0.2f32)
                    + if t < 100 {
                        f64::from(t) * f64::from(0.1f32) / 100.0
                    } else {
                        f64::from(0.1f32)
                    };
                let second = if (200..300).contains(&t) {
                    f64::from(t - 200) * f64::from(0.1f32) / 100.0
                } else {
                    f64::from(0.1f32)
                };
                let [x, y, z] = self.base.map(f32::from_bits);
                self.position(
                    0,
                    [
                        (f64::from(x) - (f64::from(first as f32) + second)) as f32,
                        y,
                        z,
                    ],
                );
            }
            if self.requested == 10 {
                self.state = 10;
                self.requested = -1;
                self.stop_rotation(0);
                let [x, y, z] = self.positions[0].map(f32::from_bits);
                self.move_to(0, [x - 3.0, y + 3.0, z], 0, 300);
                self.phase = 0;
                self.clock = 0;
                self.group(0, 4);
            }
        }
        if self.state != 10 {
            let [x, y, _] = self.positions[0].map(f32::from_bits);
            self.position(1, [x - 1.0, y - 0.1, 5.0]);
        }
        Ok(())
    }
    /// 0x41a2d0: alternate swoops around sprite 1, leaving on request 11.
    fn swooping(
        &mut self,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        if self.moving(0) {
            return Ok(());
        }
        let [x, y, z] = self.positions[1].map(f32::from_bits);
        if self.phase.is_multiple_of(2) {
            if self.requested == 11 {
                self.requested = -1;
                self.position(0, [6.0, 0.5, -1.0]);
                self.move_to(0, [-6.0, 3.5, 11.0], 0, 300);
                self.rotations(
                    1,
                    &[
                        (1, 0, 400, 0),
                        (2, 0, 400, 0),
                        (3, 1, 500, 5),
                        (4, 0x8001, 1000, -5),
                        (3, 1, 1000, 5),
                    ],
                );
                self.move_to(1, [-0.5, 2.8, 5.0], 1, 1000);
                self.state = 95;
                backend.play_preloaded(17, false, volume)?;
            } else {
                self.position(
                    0,
                    [x + if self.phase == 0 { -4.0 } else { 4.0 }, y + 5.0, z],
                );
                self.move_to(0, [x, y + 0.8, z], 0, 300);
                self.phase += 1;
            }
        } else {
            self.move_to(
                0,
                [x + if self.phase == 1 { 4.0 } else { -4.0 }, y + 5.0, z],
                0,
                300,
            );
            self.phase = (self.phase + 1) % 4;
            backend.play_preloaded(16, false, volume)?;
            if y > 2.0 {
                self.position(1, [x, y - 0.1, z]);
            }
        }
        Ok(())
    }
    /// 0x419ba0: crossing flight and bouncing spin; request 17 exits at a segment boundary.
    fn crossing(
        &mut self,
        delta: u32,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        self.clock = self.clock.wrapping_add(delta);
        if self.state == 17 {
            if !self.exit_sound {
                backend.play_preloaded(17, false, volume)?;
                self.exit_sound = true;
            }
            self.exit_clock = self.exit_clock.wrapping_add(delta);
            let t = self.exit_clock;
            self.position(
                0,
                if t >= 300 {
                    [0.0, 3.1, 0.5]
                } else {
                    [0.0, 3.05, (11.0 - f64::from(t) * 10.5 / 300.0) as f32]
                },
            );
        } else {
            let segment = self.clock % 2000 / 500;
            if segment != self.segment {
                backend.play_preloaded(17, false, volume)?;
            }
            let t = (self.clock % 500) as f64 / 500.0;
            let x = 9.0 * t - if segment.is_multiple_of(2) { 6.0 } else { 3.0 };
            let y = if segment < 2 {
                6.5 - 6.0 * t
            } else {
                0.5 + 6.0 * t
            };
            let z = if segment.is_multiple_of(2) {
                11.0 - 12.0 * t
            } else {
                12.0 * t - 1.0
            };
            self.position(0, [x as f32, y as f32, z as f32]);
            self.segment = segment;
        }
        self.second_clock = self.second_clock.wrapping_add(delta);
        if self.state == 17 {
            let [x, y, z] = self.base.map(f32::from_bits);
            let t = self.exit_clock.min(500) as f64 / 500.0;
            if self.exit_clock < 500 {
                if !self.sound_phase {
                    backend.play_preloaded(16, false, volume)?;
                    self.sound_phase = true;
                }
                self.angle(1, (720u32.wrapping_mul(self.exit_clock) / 500 % 360) as i32);
            }
            self.position(
                1,
                [
                    (f64::from(x) + 4.0 * t) as f32,
                    (f64::from(y) + t) as f32,
                    (f64::from(z) - 6.0 * t) as f32,
                ],
            );
        } else {
            if !self.second_started && self.second_clock >= 300 {
                self.second_clock -= 300;
                self.second_started = true;
                self.stop_rotation(1);
            }
            if self.second_started {
                let t = self.second_clock % 1000;
                let second = t >= 500;
                if second == self.sound_phase {
                    backend.play_preloaded(16, false, volume)?;
                    self.sound_phase = !self.sound_phase;
                }
                let phase = t % 500;
                let y = 3.8 + super::legacy::quarter(phase as f32 * 180.0 / 500.0, 0.8).1;
                let x = if second {
                    1.5 - phase as f32 * 3.0 / 500.0
                } else {
                    phase as f32 * 3.0 / 500.0 - 1.5
                };
                self.position(1, [x, y, 5.0]);
                let angle = 720u32.wrapping_mul(phase) / 500 % 360;
                self.angle(1, if second { 360 - angle } else { angle } as i32);
            }
        }
        if self.requested == 17 {
            let initial = *self.exit_segment.get_or_insert(self.segment);
            if initial != self.segment {
                self.exit_segment = Some(self.segment);
                // 0x419ba0 reads +24/+28/+32: the saved base of sprite 0.
                self.state = 17;
                self.exit_clock = self.clock % 500;
                self.sound_phase = false;
            }
        }
        Ok(())
    }
    fn angle(&mut self, index: usize, degrees: i32) {
        if let Some(draw) = self.objects[index].draw.as_mut() {
            draw.words[20] = (degrees * 100) as u32;
        }
    }
    pub fn request(&mut self, value: u32) -> Result<(), Box<dyn Error>> {
        match value {
            1..=4 | 6..=8 | 10..=14 | 17 => self.requested = value as i32,
            5 | 15 | 16 => {
                let (index, group) = match value {
                    5 => (1, 1),
                    15 => (1, 2),
                    _ => (0, 5),
                };
                if let Some(draw) = self.objects[index].draw.as_mut()
                    && let Some(animation) = draw.animation.as_mut()
                {
                    animation.start(group * 4, &mut draw.words);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        delta: u32,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        if self.state == 0 && self.requested > 0 {
            self.state = self.requested as u32;
            self.requested = -1;
        }
        match self.state {
            1..=3 => {
                let index = (self.state - 1) as usize;
                self.initialize(index);
                self.move_to(
                    index,
                    [[-0.7, 3.2, 5.0], [1.0, 4.0, 5.0], [-1.6, 2.5, 5.0]][index],
                    [1, 0, 1][index],
                    [500, 3000, 1000][index],
                );
                self.state = 90 + index as u32;
            }
            4 => {
                self.move_to(0, [1.0, 2.8, -1.0], 0, 500);
                self.move_to(2, [-2.5, 3.0, -2.0], 0, 500);
                self.move_to(1, [-1.0, 2.7, 5.0], 0, 5000);
                self.state = 90;
            }
            6 => {
                self.group(0, 2);
                self.position(0, [6.0, 4.0, -1.0]);
                self.move_to(0, [-6.0, 4.7, 11.0], 0, 1000);
                self.rotations(0, &[(-1, 0, 1000, 0)]);
                backend.play_preloaded(17, false, volume)?;
                self.state = 95;
            }
            7 | 12 | 13 => {
                let state = self.state;
                if state == 7 {
                    self.group(0, 1);
                }
                self.position(
                    0,
                    if state == 7 {
                        [-6.0, 4.0, 11.0]
                    } else {
                        [6.0, 4.5, 11.0]
                    },
                );
                self.move_to(
                    0,
                    if state == 7 {
                        [1.0, 2.5, -1.0]
                    } else if state == 12 {
                        [-1.0, 2.5, -1.0]
                    } else {
                        [-6.0, 2.5, -1.0]
                    },
                    0,
                    300,
                );
                if state == 7
                    && let Some(draw) = self.objects[1].draw.as_mut()
                {
                    draw.words[20] = 0;
                }
                let mode = if state == 7 { 0 } else { 0x8000 };
                self.rotations(
                    1,
                    &[
                        (1, mode, 400, 0),
                        (2, mode, 400, 0),
                        (3, 1, 500, 5),
                        (4, 0x8001, 1000, -5),
                        (3, 1, 1000, 5),
                    ],
                );
                self.move_to(
                    1,
                    if state == 7 {
                        [-0.5, 3.2, 5.0]
                    } else if state == 12 {
                        [-1.0, 3.2, 5.0]
                    } else {
                        [-1.5, 3.8, 5.0]
                    },
                    1,
                    1000,
                );
                if state == 12 {
                    backend.play_preloaded(16, false, volume)?;
                }
                backend.play_preloaded(17, false, volume)?;
                self.state = 95;
            }
            8 => {
                self.phase = 0;
                self.clock = 0;
                self.state = 9;
                self.stepping(delta, backend, volume)?;
            }
            9 => self.stepping(delta, backend, volume)?,
            10 => self.swooping(backend, volume)?,
            14 => {
                self.state = 20;
                self.clock = 0;
                self.second_clock = 0;
                self.segment = 0;
                self.exit_segment = None;
                self.exit_sound = false;
                self.sound_phase = false;
                self.second_started = false;
                self.crossing(delta, backend, volume)?;
            }
            17 | 20 => self.crossing(delta, backend, volume)?,
            95 if self
                .objects
                .iter()
                .all(|o| o.draw.as_ref().is_none_or(|d| d.world_motion.is_none())) =>
            {
                self.state = 0;
            }
            90..=92
                if self.objects[(self.state - 90) as usize]
                    .draw
                    .as_ref()
                    .is_none_or(|d| d.world_motion.is_none()) =>
            {
                self.state = 0;
            }
            _ => {}
        }
        // 0x418d20 updates object tracks after the effect's state logic.
        for object in &mut self.objects {
            object.advance_effects(delta);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sprite_effect_renders_through_the_puriring_camera_without_scene_slots() {
        let profile = crate::profile::Registry::builtin()
            .profile("cmvs-20070119")
            .unwrap();
        let cameras = crate::camera::CameraHost::for_profile(profile);
        let image = ObjectImage {
            name: b"sprite.pb3".to_vec(),
            pixels: crate::raster::RgbaImage::new(256, 768, [255, 0, 0, 255].repeat(256 * 768))
                .unwrap(),
            alpha: true,
            surface: false,
            filter: None,
        };
        let mut sprites = Sprites::new(vec![image]);
        sprites.initialize(0);
        sprites.position(0, [0.0, 3.0, 5.0]);
        sprites.objects[0].shown = true;
        let mut scene = crate::scene::SceneHost::default();
        scene
            .legacy_effects
            .insert(0, super::super::LegacyEffect::Sprites(Box::new(sprites)));
        let frame =
            crate::scene_render::render_scene(&scene, Some(&cameras), &[], 1024, 614).unwrap();
        let center = (307 * 1024 + 512) * 4;
        assert_eq!(&frame.pixels()[center..center + 4], &[255, 0, 0, 255]);
        assert!(scene.objects().is_empty());
    }

    #[test]
    fn continuous_actions_accept_exit_requests_at_their_native_boundaries() {
        let mut sprites = Sprites::new(Vec::new());
        let mut backend = crate::audio::RecordingAudio::default();
        sprites.initialize(0);
        sprites.initialize(1);
        sprites.request(8).unwrap();
        sprites.advance(100, &mut backend, 128).unwrap();
        assert_eq!(sprites.state, 9);
        sprites.request(10).unwrap();
        for _ in 0..7 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!(sprites.state, 9); // arrival is observed on the next frame
        sprites.advance(100, &mut backend, 128).unwrap();
        assert_eq!((sprites.state, sprites.requested), (10, -1));
        sprites.request(11).unwrap();
        for _ in 0..3 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!((sprites.state, sprites.requested), (95, -1));
        for _ in 0..11 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!(sprites.state, 0);
        sprites.request(14).unwrap();
        sprites.advance(100, &mut backend, 128).unwrap();
        assert_eq!(sprites.state, 20);
        sprites.request(17).unwrap();
        for _ in 0..3 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!(sprites.state, 20);
        sprites.advance(100, &mut backend, 128).unwrap();
        assert_eq!(sprites.state, 17);
        for _ in 0..6 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!(sprites.positions[0].map(f32::from_bits), [0.0, 3.1, 0.5]);
        // The original retains request 17; queries continue to report -1.
        assert_eq!(sprites.requested, 17);
    }

    #[test]
    fn flight_waits_for_all_motions_and_plays_the_preloaded_sound() {
        let mut sprites = Sprites::new(Vec::new());
        let mut backend = crate::audio::RecordingAudio::default();
        sprites.initialize(0);
        sprites.initialize(1);
        sprites.request(7).unwrap();
        sprites.advance(100, &mut backend, 128).unwrap();
        assert_eq!(sprites.state, 95);
        assert_eq!(backend.requests().len(), 1);
        for _ in 0..2 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert!(
            sprites.objects[0]
                .draw
                .as_ref()
                .unwrap()
                .world_motion
                .is_none()
        );
        assert!(
            sprites.objects[1]
                .draw
                .as_ref()
                .unwrap()
                .world_motion
                .is_some()
        );
        assert_eq!(sprites.state, 95);
        for _ in 0..8 {
            sprites.advance(100, &mut backend, 128).unwrap();
        }
        assert_eq!(sprites.state, 0);
        let before = sprites.requested;
        sprites.request(9).unwrap();
        assert_eq!(sprites.requested, before);
    }

    #[test]
    fn entrances_wait_for_the_correct_track_and_preserve_sprite_frames() {
        let mut sprites = Sprites::new(Vec::new());
        let mut backend = crate::audio::RecordingAudio::default();
        assert!(sprites.objects.iter().all(|o| !o.shown));
        sprites.request(1).unwrap();
        sprites.advance(100, &mut backend, 256).unwrap();
        assert_eq!((sprites.state, sprites.requested), (90, -1));
        assert!(sprites.objects[0].shown);
        assert!(!sprites.objects[1].shown);
        assert_eq!(sprites.objects[0].draw.as_ref().unwrap().words[3], 192);
        for _ in 0..4 {
            sprites.advance(100, &mut backend, 256).unwrap();
        }
        assert_eq!(sprites.state, 90);
        assert_eq!(
            sprites.objects[0].draw.as_ref().unwrap().words[11..14],
            [-0.7f32, 3.2, 5.0].map(f32::to_bits)
        );
        sprites.advance(100, &mut backend, 256).unwrap();
        assert_eq!(sprites.state, 0);
        sprites.request(16).unwrap();
        let draw = sprites.objects[0].draw.as_ref().unwrap();
        assert_eq!(draw.words[2..8], [1280, 0, 256, 192, 128, 96]);
        assert!(backend.requests().is_empty());
    }
}

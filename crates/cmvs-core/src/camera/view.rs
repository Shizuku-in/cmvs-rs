//! Device +32/+40: independent 3D view pose, distinct from the mode 2/3 screen cameras.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    /// +0..+24: XYZ, three angular words and field of view, preserved as raw float bits.
    pub words: [u32; 7],
    /// +92, writer 0x1400501d0 version 2; reset leaves it unchanged.
    pub snapshot_mode: u32,
    /// +28..+88 transient motion; only the hash-bound cmvs-20070119 commands start it.
    pub motion: Option<ViewMotion>,
}

/// cmvs-20070119 view motion (0x423240 / 0x4232a0, frame update 0x4233b0). Either the
/// position (words 0..2) or the X/Y angles (words 3/4) move from the words current at the
/// start toward `target`; words outside the moving group are left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewMotion {
    /// +32: 0 linear, 1 sine ease-out, 2 cosine ease-in; other values hold until the end.
    pub mode: u32,
    /// +56 duration and +88 elapsed milliseconds.
    pub duration: u32,
    pub elapsed: u32,
    /// +60 moves the position, +64 the angles.
    pub position: bool,
    pub angles: bool,
    /// +68..+84 start words 0..4 and +36..+52 targets.
    pub start: [u32; 5],
    pub target: [u32; 5],
}

/// 0x422170: cosine and sine of `degrees` times `length`, each truncated to hundredths.
fn quarter(degrees: f32, length: f32) -> (f32, f32) {
    let radians = if degrees == 0.0 {
        0.0
    } else {
        f64::from(degrees) / 180.0 * f64::from(std::f32::consts::PI)
    };
    let hundredths = |value: f64| {
        let value = value as f32;
        ((f64::from(value) * 100.0) as i32) as f64 / 100.0
    };
    (
        hundredths(radians.cos() * f64::from(length)) as f32,
        hundredths(radians.sin() * f64::from(length)) as f32,
    )
}

impl ViewMotion {
    fn finish(&self, words: &mut [u32; 7]) {
        if self.position {
            words[..3].copy_from_slice(&self.target[..3]);
        }
        if self.angles {
            words[3..5].copy_from_slice(&self.target[3..]);
        }
    }
    /// One frame of 0x4233b0; returns false once the motion has ended.
    fn advance(&mut self, delta: u32, words: &mut [u32; 7]) -> bool {
        self.elapsed = self.elapsed.wrapping_add(delta);
        if self.elapsed >= self.duration {
            self.finish(words);
            return false;
        }
        let t = self.elapsed as f32;
        let d = self.duration as f32;
        let ranges = [(self.position, 0..3), (self.angles, 3..5)];
        for (_, range) in ranges.into_iter().filter(|(moving, _)| *moving) {
            for i in range {
                let start = f32::from_bits(self.start[i]);
                let difference = f32::from_bits(self.target[i]) - start;
                let offset = match self.mode {
                    0 => {
                        let scaled = f64::from(difference) * f64::from(t) / f64::from(d);
                        (((scaled * 100.0) as i32) as f64 / 100.0) as f32
                    }
                    1 => quarter((f64::from(t) * 90.0 / f64::from(d)) as f32, difference).1,
                    2 => {
                        let degrees =
                            (((90.0 * f64::from(t) / f64::from(d)) * 100.0) as i32) as f64 / 100.0;
                        difference - quarter(degrees as f32, difference).0
                    }
                    _ => continue,
                };
                words[i] = (offset + start).to_bits();
            }
        }
        true
    }
}

/// The device view and projection of one frame: LookAtLH from the quantized view direction
/// (0x1400546c0 / cmvs-20070119 0x422cb0) and PerspectiveFovLH with near 0.1 and far 100.
#[derive(Debug, Clone, Copy)]
pub struct ViewProjection {
    eye: [f32; 3],
    axes: [[f32; 3]; 3],
    scale: [f32; 2],
    viewport: [f32; 2],
}
impl ViewProjection {
    pub const NEAR: f32 = 0.1;
    pub const FAR: f32 = 100.0;
    /// Reject a quad only when all corners are outside the same side plane.
    /// Test before perspective division so cells crossing the near plane are safe.
    pub(crate) fn outside_sides(&self, points: &[[f32; 3]; 4]) -> bool {
        (0..2).any(|axis| {
            [-1.0, 1.0].into_iter().any(|sign| {
                points
                    .iter()
                    .all(|p| sign * p[axis] * self.scale[axis] > p[2])
            })
        })
    }
    /// View-space X/Y/Z of a world point; Z is the depth along the view direction.
    pub fn view(&self, point: [f32; 3]) -> [f32; 3] {
        self.axes.map(|axis| {
            (point[0] * axis[0] + point[1] * axis[1] + point[2] * axis[2])
                - (self.eye[0] * axis[0] + self.eye[1] * axis[1] + self.eye[2] * axis[2])
        })
    }
    /// Device coordinates of a view-space point in front of the near plane; device
    /// viewport pixel centers are integers.
    pub fn screen(&self, [x, y, z]: [f32; 3]) -> [f32; 2] {
        let [width, height] = self.viewport;
        [
            (x * self.scale[0] / z + 1.0) * width * 0.5,
            (1.0 - y * self.scale[1] / z) * height * 0.5,
        ]
    }
}

impl DeviceView {
    pub fn projection(&self, viewport: [u32; 2]) -> Result<ViewProjection, &'static str> {
        let values = self.words.map(f32::from_bits);
        if values.iter().any(|v| !v.is_finite())
            || !(0.0..180.0).contains(&values[6])
            || values[6] == 0.0
            || viewport.contains(&0)
        {
            return Err("invalid device view parameters");
        }
        let radians = |degrees: f32| f64::from(degrees / 180.0 * std::f32::consts::PI);
        let (pitch, yaw, roll) = (radians(values[3]), radians(values[4]), radians(values[5]));
        let quantize = |value: f64| ((value * 1000.0) as i32) as f32 / 1000.0;
        let direction = [
            quantize(yaw.sin() * pitch.cos()),
            quantize(-pitch.sin()),
            quantize(yaw.cos() * pitch.cos()),
        ];
        let up = [quantize(-roll.sin()), quantize(roll.cos()), 0.0];
        let eye = [values[0], values[1], values[2]];
        let normalize = |v: [f32; 3]| {
            let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|x| x / length)
        };
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let forward = normalize(std::array::from_fn(|i| (eye[i] + direction[i]) - eye[i]));
        let right = normalize(cross(up, forward));
        let up = cross(forward, right);
        if ![forward, right, up].iter().flatten().all(|v| v.is_finite()) {
            return Err("degenerate device view basis");
        }
        let [width, height] = viewport.map(|v| v as f32);
        let scale_y = 1.0 / (values[6] * (std::f32::consts::PI / 180.0) * 0.5).tan();
        Ok(ViewProjection {
            eye,
            axes: [right, up, forward],
            scale: [scale_y / (width / height), scale_y],
            viewport: [width, height],
        })
    }

    /// Type-2 effects use inverse-view billboards (0x14002a530), so all four
    /// corners have the same view depth. Device viewport pixel centers are integers.
    pub fn billboard(
        &self,
        position: [f32; 3],
        size: [f32; 2],
        viewport: [u32; 2],
    ) -> Result<Option<[f32; 4]>, &'static str> {
        let values = self.words.map(f32::from_bits);
        if values
            .iter()
            .chain(position.iter())
            .chain(size.iter())
            .any(|v| !v.is_finite())
            || !(0.0..180.0).contains(&values[6])
            || values[6] == 0.0
            || viewport.contains(&0)
            || size.iter().any(|&v| v <= 0.0)
        {
            return Err("invalid world billboard or device view parameters");
        }
        // 0x1400546c0 computes angles in f32, trig in f64, then truncates each
        // direction component to a thousandth before LookAtLH normalizes it.
        let radians = |degrees: f32| f64::from(degrees / 180.0 * std::f32::consts::PI);
        let (pitch, yaw, roll) = (radians(values[3]), radians(values[4]), radians(values[5]));
        let quantize = |value: f64| ((value * 1000.0) as i32) as f32 / 1000.0;
        let direction = [
            quantize(yaw.sin() * pitch.cos()),
            quantize(-pitch.sin()),
            quantize(yaw.cos() * pitch.cos()),
        ];
        let up = [quantize(-roll.sin()), quantize(roll.cos()), 0.0];
        let eye = [values[0], values[1], values[2]];
        let normalize = |v: [f32; 3]| {
            let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|x| x / length)
        };
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let forward = normalize(std::array::from_fn(|i| (eye[i] + direction[i]) - eye[i]));
        let right = normalize(cross(up, forward));
        let up = cross(forward, right);
        let dot = |axis: [f32; 3]| {
            (position[0] * axis[0] + position[1] * axis[1] + position[2] * axis[2])
                - (eye[0] * axis[0] + eye[1] * axis[1] + eye[2] * axis[2])
        };
        let [x, y, z] = [dot(right), dot(up), dot(forward)];
        if ![x, y, z].iter().all(|v| v.is_finite()) {
            return Err("degenerate device view basis");
        }
        // PerspectiveFovLH uses near=0.1, far=100; queue 5 tests LESS with no Z writes.
        if z < 0.1 || z >= 100.0 {
            return Ok(None);
        }
        let [width, height] = viewport.map(|v| v as f32);
        let scale_y = 1.0 / (values[6] * (std::f32::consts::PI / 180.0) * 0.5).tan();
        let scale_x = scale_y / (width / height);
        let horizontal = |offset: f32| ((x + offset) * scale_x / z + 1.0) * width * 0.5;
        let vertical = |offset: f32| (1.0 - (y + offset) * scale_y / z) * height * 0.5;
        let quad = [
            horizontal(-size[0] * 0.5),
            vertical(size[1] * 0.5),
            horizontal(size[0] * 0.5),
            vertical(-size[1] * 0.5),
        ];
        if !quad.iter().all(|v| v.is_finite()) {
            return Err("non-finite world billboard extent");
        }
        Ok(Some(quad))
    }

    pub(super) fn new(index: usize) -> Self {
        let mut view = Self {
            words: [0; 7],
            snapshot_mode: u32::from(index == 0),
            motion: None,
        };
        view.reset();
        view
    }
    /// 0x140050180 resets the pose; cmvs-20070119 0x422df0 also zeroes the transient motion
    /// words +28..+88, so no motion survives. `snapshot_mode` (+92) is retained.
    pub(super) fn reset(&mut self) {
        self.words = [0, 1.0f32.to_bits(), 0, 0, 0, 0, 45.0f32.to_bits()];
        self.motion = None;
    }
}

impl CameraHost {
    pub fn view(&self, index: usize) -> Option<&DeviceView> {
        self.views.get(index)
    }

    /// Direct writes of 0x422ea0 / 0x422ee0 / 0x422ef0 from engine code (not the VM).
    pub(crate) fn set_view_position(&mut self, position: [f32; 3]) {
        for (word, value) in self.views[0].words[..3].iter_mut().zip(position) {
            *word = value.to_bits();
        }
    }
    pub(crate) fn set_view_angle(&mut self, axis: usize, degrees: f32) {
        self.views[0].words[3 + axis] = degrees.to_bits();
    }
    pub(super) fn advance_view(&mut self, delta: u32) {
        let view = &mut self.views[0];
        if let Some(motion) = view.motion.as_mut()
            && !motion.advance(delta, &mut view.words)
        {
            view.motion = None;
        }
    }
    pub fn view_moving(&self) -> bool {
        self.views[0].motion.is_some()
    }
    fn stop_view(&mut self, finish: bool) {
        let view = &mut self.views[0];
        if let Some(motion) = view.motion.take()
            && finish
        {
            motion.finish(&mut view.words);
        }
    }

    /// cmvs-20070119 view motion on the single device view (engine +1900 -> +12):
    /// - 0x2270 / 0x43d070: stop; SP-4 nonzero jumps to the targets (0x423200); pops 4.
    /// - 0x2272 / 0x43d160: system result = moving; pops nothing.
    /// - 0x2273 / 0x43d190: SP-4 mode, SP-8..SP-16 XYZ, SP-20 duration; pops 20.
    /// - 0x2274 / 0x43d200: SP-4 mode, SP-8/SP-12 X/Y angles, SP-16 duration; pops 16.
    pub(super) fn view_motion(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let command = call.command;
        match command {
            0x270 => {
                let finish = call.argument(4, "finish")? != 0;
                call.complete(0x4004)?;
                self.stop_view(finish);
            }
            0x272 => {
                call.complete(0x4000)?;
                let moving = u32::from(self.view_moving());
                call.vm
                    .state_mut()
                    .set_system_integer(0, moving)
                    .expect("result 0 exists");
            }
            _ => {
                let count = if command == 0x273 { 5 } else { 4 };
                let mut args = [0u32; 5];
                const NAMES: [&str; 5] =
                    ["mode", "first target", "second target", "third", "fourth"];
                for (i, arg) in args.iter_mut().take(count).enumerate() {
                    *arg = call.argument((i + 1) * 4, NAMES[i])?;
                }
                call.complete(0x4000 | (count * 4) as u16)?;
                let view = &mut self.views[0];
                let mut start = [0; 5];
                start.copy_from_slice(&view.words[..5]);
                // Targets of the group that does not move keep the previous motion's values.
                let mut target = view.motion.as_ref().map_or(start, |m| m.target);
                let (position, angles, duration) = if command == 0x273 {
                    target[..3].copy_from_slice(&args[1..4]);
                    (true, false, args[4])
                } else {
                    target[3..].copy_from_slice(&args[1..3]);
                    (false, true, args[3])
                };
                view.motion = Some(ViewMotion {
                    mode: args[0],
                    duration,
                    elapsed: 0,
                    position,
                    angles,
                    start,
                    target,
                });
            }
        }
        Ok(())
    }

    /// cmvs-20070119 0x2271 / 0x43d0a0: wait on the view motion with the gates of 0x2069.
    pub(crate) fn handle_wait_view(
        &mut self,
        call: &mut Call<'_, '_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Flow, DecodeError> {
        let skippable = call.argument(4, "skippable")? != 0;
        if !self.view_moving() {
            let flow = call.finish(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, 0)
                .expect("result 0 exists");
            return Ok(flow);
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let released = skippable && !immediate && input.released(0);
        if skippable && (immediate || released) {
            let flow = call.finish(0xc004)?;
            if released {
                input.consume(0);
            }
            call.vm
                .state_mut()
                .set_system_integer(0, 1)
                .expect("result 0 exists");
            self.stop_view(true);
            return Ok(flow);
        }
        let flow = call.wait(4)?;
        call.vm
            .state_mut()
            .set_system_integer(0, 0)
            .expect("result 0 exists");
        Ok(flow)
    }

    pub(super) fn set_view(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let command = call.command;
        let count = if command == 0x260 { 3 } else { 1 };
        let mut values = [0; 3];
        for (i, value) in values[..count].iter_mut().enumerate() {
            *value = call.argument((i + 1) * 4, argument_names(call.command)[i])?;
        }
        call.complete(0x4000 | (count * 4) as u16)?;
        let words = &mut self.views[0].words;
        if command == 0x260 {
            words[..3].copy_from_slice(&values);
        } else {
            words[3 + usize::from(command - 0x261)] = values[0];
        }
        Ok(())
    }

    /// Record 0x882/0x883, 0x1400501d0: u16 2, +92, then seven raw float words.
    pub(crate) fn saved_view(&self, index: usize) -> Vec<u8> {
        let view = &self.views[index];
        let legacy = self.config.snapshot_format == super::CameraSnapshotFormat::V1;
        let mut out = (if legacy { 1u16 } else { 2u16 }).to_le_bytes().to_vec();
        if !legacy {
            out.extend(view.snapshot_mode.to_le_bytes());
        }
        for word in view.words {
            out.extend(word.to_le_bytes());
        }
        out
    }

    /// 0x140050270: version 1 lacks +92 and keeps the current setting; version 2
    /// supplies it. Both copy all seven words and stop view motion.
    pub(crate) fn restore_view(&mut self, index: usize, record: &[u8]) -> Result<(), &'static str> {
        let version = record
            .get(..2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()));
        let offset = match (version, record.len()) {
            (Some(1), 30) => 2,
            (Some(2), 34) => 6,
            _ => return Err("unsupported 3D view snapshot version or size"),
        };
        let word = |at| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let view = &mut self.views[index];
        view.words = std::array::from_fn(|i| word(offset + i * 4));
        view.motion = None;
        if offset == 6 {
            view.snapshot_mode = word(2);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn billboards_project_through_the_device_view_and_clip_near_and_far() {
        let mut view = DeviceView::new(0);
        view.words[6] = 90.0f32.to_bits();
        let quad = view
            .billboard([0.0, 1.0, 1.0], [0.1, 0.1], [40, 40])
            .unwrap()
            .unwrap();
        for (actual, expected) in quad.into_iter().zip([19.0, 19.0, 21.0, 21.0]) {
            assert!((actual - expected).abs() < 0.0001);
        }
        for z in [-1.0, 0.0, 0.099, 100.0, 101.0] {
            assert_eq!(
                view.billboard([0.0, 1.0, z], [0.1, 0.1], [40, 40]).unwrap(),
                None
            );
        }
        view.words[4] = 90.0f32.to_bits();
        let quad = view
            .billboard([1.0, 1.0, 0.0], [0.1, 0.1], [40, 40])
            .unwrap()
            .unwrap();
        assert!((quad[0] - 19.0).abs() < 0.0001);
        view.words[6] = f32::NAN.to_bits();
        assert!(view.billboard([0.0; 3], [0.1; 2], [40, 40]).is_err());
    }

    #[test]
    fn view_records_round_trip_both_versions_and_reset_retains_the_snapshot_mode() {
        let mut host = CameraHost::default();
        let words = [1, 0x8000_0000, 0x7fc0_1234, 4, 5, 6, 7];
        for index in 0..2 {
            host.views[index].words = words;
            host.views[index].snapshot_mode = 17;
            let bytes = host.saved_view(index);
            let mut restored = CameraHost::default();
            restored.restore_view(index, &bytes).unwrap();
            assert_eq!(restored.views[index], host.views[index]);
            let mut v1_record = 1u16.to_le_bytes().to_vec();
            v1_record.extend_from_slice(&bytes[6..]);
            restored.views[index].snapshot_mode = 23;
            restored.restore_view(index, &v1_record).unwrap();
            assert_eq!(restored.views[index].words, words);
            assert_eq!(restored.views[index].snapshot_mode, 23);
            let before = restored.views[index].clone();
            for bad in [&bytes[..33], &v1_record[..29], &[3u8, 0][..]] {
                assert!(restored.restore_view(index, bad).is_err());
                assert_eq!(restored.views[index], before);
            }
            restored.reset();
            assert_eq!(restored.views[index].words, DeviceView::new(index).words);
            assert_eq!(restored.views[index].snapshot_mode, 23);
        }
    }

    #[test]
    fn reset_stops_a_running_view_motion() {
        let mut host = CameraHost::default();
        let target = [5.0f32, 6.0, 7.0, 0.0, 0.0].map(f32::to_bits);
        host.views[0].snapshot_mode = 9;
        host.views[0].motion = Some(ViewMotion {
            mode: 0,
            duration: 1000,
            elapsed: 0,
            position: true,
            angles: false,
            start: host.views[0].words[..5].try_into().unwrap(),
            target,
        });
        host.advance_view(100);
        assert!(host.view_moving());
        host.reset();
        assert!(!host.view_moving());
        assert_eq!(host.views[0].snapshot_mode, 9);
        host.advance_view(500);
        assert_eq!(host.views[0].words, DeviceView::new(0).words);
    }
}

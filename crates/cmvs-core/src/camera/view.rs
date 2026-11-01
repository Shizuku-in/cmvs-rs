//! Device +32/+40: independent 3D view pose, distinct from the mode 2/3 screen cameras.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    /// +0..+24: XYZ, three angular words and field of view, preserved as raw float bits.
    pub words: [u32; 7],
    /// +92, writer 0x1400501d0 version 2; reset leaves it unchanged.
    pub snapshot_mode: u32,
}

impl DeviceView {
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
        };
        view.reset();
        view
    }
    /// 0x140050180; transient motion words +28..+88 are currently unreachable.
    pub(super) fn reset(&mut self) {
        self.words = [0, 1.0f32.to_bits(), 0, 0, 0, 0, 45.0f32.to_bits()];
    }
}

impl CameraHost {
    pub fn view(&self, index: usize) -> Option<&DeviceView> {
        self.views.get(index)
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
        let mut out = 2u16.to_le_bytes().to_vec();
        out.extend(view.snapshot_mode.to_le_bytes());
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
            let mut old = 1u16.to_le_bytes().to_vec();
            old.extend_from_slice(&bytes[6..]);
            restored.views[index].snapshot_mode = 23;
            restored.restore_view(index, &old).unwrap();
            assert_eq!(restored.views[index].words, words);
            assert_eq!(restored.views[index].snapshot_mode, 23);
            let before = restored.views[index].clone();
            for bad in [&bytes[..33], &old[..29], &[3u8, 0][..]] {
                assert!(restored.restore_view(index, bad).is_err());
                assert_eq!(restored.views[index], before);
            }
            restored.reset();
            assert_eq!(restored.views[index].words, DeviceView::new(index).words);
            assert_eq!(restored.views[index].snapshot_mode, 23);
        }
    }
}

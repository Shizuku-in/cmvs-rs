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

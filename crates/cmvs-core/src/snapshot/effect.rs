//! 0x800 configuration records: Aoitori 0x140042fd0 / 0x140043120;
//! MIRANOSU type 5: 0x140037130 / 0x140037260 (other types remain gated).
//! KUNADO type 5: 0x1400525b0 / 0x140052e80 (other types remain gated).
use crate::FormatError;

/// The native writer stores creation parameters, not the live particle simulation.
/// Name padding is retained for lossless decoding; only bytes before NUL name a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedEffect {
    Rain {
        words: [u32; 6],
        name: [u8; 32],
    },
    Falling {
        words: [u32; 6],
        name: [u8; 32],
    },
    Particles {
        words: [u32; 6],
        name: [u8; 32],
    },
    Collision {
        words: [u32; 4],
        name: [u8; 32],
    },
    /// Type 8 is written without a body and the native reader discards it.
    Fragments,
}

impl SavedEffect {
    pub(crate) fn name_field(name: &[u8]) -> Result<[u8; 32], FormatError> {
        let name = name.split(|&b| b == 0).next().unwrap_or_default();
        if name.len() >= 32 {
            return Err(FormatError(
                "saved particle image name exceeds its 32-byte field",
            ));
        }
        let mut field = [0; 32];
        field[..name.len()].copy_from_slice(name);
        Ok(field)
    }

    pub fn image_name(&self) -> &[u8] {
        match self {
            Self::Rain { name, .. }
            | Self::Falling { name, .. }
            | Self::Particles { name, .. }
            | Self::Collision { name, .. } => {
                &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())]
            }
            Self::Fragments => &[],
        }
    }

    fn validate(&self) -> Result<(), FormatError> {
        let (parameters, count, name) = match self {
            Self::Rain { words, name } => {
                if words[5] == 0
                    || ((words[0] >> 8) & 0xf) > 6
                    || !matches!(
                        words[0] & 0xf0,
                        0 | 0x10 | 0x20 | 0x30 | 0x40 | 0x50 | 0x60 | 0x70 | 0x80 | 0xa0
                    )
                {
                    return Err(FormatError("unsupported rain mode or emission interval"));
                }
                (&words[1..5], 0, name)
            }
            Self::Falling { words, name } => (&words[..4], words[4], name),
            Self::Particles { words, name } => (&words[1..5], words[5], name),
            Self::Collision { words, name } => (&words[1..3], words[3], name),
            Self::Fragments => return Ok(()),
        };
        if count > 16_384 || !name.contains(&0) {
            return Err(FormatError(
                "saved particle count or image name exceeds its supported bound",
            ));
        }
        let values: Vec<_> = parameters.iter().map(|&v| f32::from_bits(v)).collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(FormatError("saved particle parameters must be finite"));
        }
        let valid = match self {
            Self::Falling { .. } => values[..3].iter().all(|&v| v > 0.0) && values[3] >= 0.0,
            _ => *values.last().expect("particle speed") > 0.0,
        };
        if !valid {
            return Err(FormatError(
                "saved particle dimensions or speed are invalid",
            ));
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() < 4 || bytes[..2] != [1, 0] {
            return Err(FormatError("unsupported saved particle record version"));
        }
        let kind = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
        let words = match kind {
            2 | 4 | 5 => 6,
            6 => 4,
            8 if bytes.len() == 4 => return Ok(Self::Fragments),
            _ => return Err(FormatError("unsupported saved particle effect type")),
        };
        if bytes.len() != 4 + words * 4 + 32 {
            return Err(FormatError(
                "saved particle record has an unexpected length",
            ));
        }
        let word = |i| u32::from_le_bytes(bytes[4 + i * 4..8 + i * 4].try_into().unwrap());
        let name = bytes[4 + words * 4..].try_into().unwrap();
        let effect = match kind {
            2 => Self::Falling {
                words: std::array::from_fn(word),
                name,
            },
            4 => Self::Rain {
                words: std::array::from_fn(word),
                name,
            },
            5 => Self::Particles {
                words: std::array::from_fn(word),
                name,
            },
            6 => Self::Collision {
                words: std::array::from_fn(word),
                name,
            },
            _ => unreachable!(),
        };
        effect.validate()?;
        Ok(effect)
    }

    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        self.validate()?;
        let (kind, words, name): (u16, &[u32], &[u8]) = match self {
            Self::Rain { words, name } => (4, words, name),
            Self::Falling { words, name } => (2, words, name),
            Self::Particles { words, name } => (5, words, name),
            Self::Collision { words, name } => (6, words, name),
            Self::Fragments => (8, &[], &[]),
        };
        let mut bytes = 1u16.to_le_bytes().to_vec();
        bytes.extend(kind.to_le_bytes());
        for word in words {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(name);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: u16, words: &[u32]) -> Vec<u8> {
        let mut bytes = vec![1, 0];
        bytes.extend(kind.to_le_bytes());
        for word in words {
            bytes.extend(word.to_le_bytes());
        }
        let mut name = [0xa5; 32];
        name[..10].copy_from_slice(b"flake.pb3\0");
        bytes.extend(name);
        bytes
    }

    #[test]
    fn creation_records_preserve_raw_parameters_and_name_padding_without_particle_positions() {
        let f = f32::to_bits;
        for (kind, words) in [
            (2, vec![f(120.0), f(100.0), f(2000.0), f(5.0), 3, 7]),
            (
                5,
                vec![0x85000000, f(-40.0), f(15.0), f(-20.0), f(2000.0), 8],
            ),
            (4, vec![0x1000, 0, 0, f(12.0), f(250.0), 16]),
            (6, vec![0x41000000, f(10.0), f(1000.0), 4]),
        ] {
            let bytes = record(kind, &words);
            let saved = SavedEffect::decode(&bytes).unwrap();
            assert_eq!(saved.image_name(), b"flake.pb3");
            assert_eq!(saved.encode().unwrap(), bytes);
            for end in 0..bytes.len() {
                assert!(SavedEffect::decode(&bytes[..end]).is_err());
            }
        }
        assert_eq!(
            SavedEffect::decode(&[1, 0, 8, 0]).unwrap(),
            SavedEffect::Fragments
        );
        assert_eq!(SavedEffect::Fragments.encode().unwrap(), [1, 0, 8, 0]);
        assert!(SavedEffect::decode(&[1, 0, 8, 0, 0]).is_err());
    }

    #[test]
    fn malformed_effect_templates_do_not_grant_unbounded_allocation_or_unknown_formats() {
        let base = record(5, &[0, 0, 0, 0, 1000f32.to_bits(), 1]);
        for (offset, value) in [(24, u32::MAX), (8, f32::NAN.to_bits()), (20, 0)] {
            let mut bad = base.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(SavedEffect::decode(&bad).is_err());
        }
        let mut bad = base.clone();
        bad[28..].fill(b'x');
        assert!(SavedEffect::decode(&bad).is_err());
        for prefix in [[2, 0, 5, 0], [1, 0, 9, 0]] {
            bad = base.clone();
            bad[..4].copy_from_slice(&prefix);
            assert!(SavedEffect::decode(&bad).is_err());
        }
        assert!(SavedEffect::name_field(&[b'x'; 32]).is_err());
    }
}

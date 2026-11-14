//! CSS0, the CVNS system save (`save000.dat`): cvns-20090203 writer 0x420460, reader
//! 0x4208e0. The persistent ranges are the tails of the variable banks from the starts set
//! by CVNS 0x210b..0x210d; the same LZSS as CSS1 packs them.
use super::decompress;
use crate::{FormatError, MAX_PAYLOAD, evaluate::State};

pub const CSS0_HEADER: usize = 600;
const KEY: &[u8] = b"PURPLE YUKO KAWAI ARC CHECK";
const BIT_BYTES: u32 = 0x8000;
const INTEGERS: u32 = 4096;
const STRINGS: u32 = 256;

/// First persistent bit, integer and string. Constructor 0x41f320 defaults them to
/// 0x1000, 0x400 and 0x20 until CVNS 0x210b..0x210d run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistentStarts {
    pub bit: u32,
    pub integer: u32,
    pub string: u32,
}

impl Default for PersistentStarts {
    fn default() -> Self {
        Self {
            bit: 0x1000,
            integer: 0x400,
            string: 0x20,
        }
    }
}

impl PersistentStarts {
    fn checked(self) -> Result<Self, FormatError> {
        if self.bit >> 3 > BIT_BYTES || self.integer > INTEGERS || self.string > STRINGS {
            return Err(FormatError(
                "CSS0 persistent start beyond its variable bank",
            ));
        }
        Ok(self)
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().expect("validated CSS0 header"))
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("validated CSS0 header"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Css0Save {
    starts: PersistentStarts,
    /// Bytes of the bit bank from `starts.bit >> 3` to its end.
    bits: Vec<u8>,
    integers: Vec<i32>,
    strings: Vec<Vec<u8>>,
    /// Name lists +0x6058/+0x605c (0x4217c0 clears, 0x4218d0 appends).
    lists: [Vec<Vec<u8>>; 2],
}

impl Css0Save {
    pub fn lists(&self) -> &[Vec<Vec<u8>>; 2] {
        &self.lists
    }
    /// The original checks the trailing sum only when the recorded end matches the file
    /// length and skips the load on a mismatch; any other inconsistency is rejected here.
    pub fn decode(raw: &[u8], starts: PersistentStarts) -> Result<Self, FormatError> {
        let starts = starts.checked()?;
        if raw.len() < CSS0_HEADER + 2 || raw.len() > MAX_PAYLOAD {
            return Err(FormatError(
                "CSS0 file is truncated or exceeds research bounds",
            ));
        }
        if &raw[..4] != b"CSS0" || u32_at(raw, 4) != CSS0_HEADER as u32 || u16_at(raw, 544) != 1 {
            return Err(FormatError("unsupported CSS0 header or encoding mode"));
        }
        let end = u32_at(raw, 540) as usize;
        let packed = u32_at(raw, 560) as usize;
        let decoded = u32_at(raw, 580) as usize;
        let counts = [u32_at(raw, 552) as usize, u32_at(raw, 556) as usize];
        if end != CSS0_HEADER + packed || raw.len() != end + 2 || decoded > MAX_PAYLOAD {
            return Err(FormatError("CSS0 lengths disagree with the file"));
        }
        if counts.iter().any(|&n| n > 4096) {
            return Err(FormatError("CSS0 name list exceeds research bounds"));
        }
        let packed = &raw[CSS0_HEADER..end];
        let sum = packed
            .iter()
            .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
        if sum != u16_at(raw, end) {
            return Err(FormatError("CSS0 payload checksum mismatch"));
        }
        let plain: Vec<u8> = packed
            .iter()
            .enumerate()
            .map(|(i, b)| {
                (raw[546].wrapping_add(KEY[i % KEY.len()]) ^ b.wrapping_sub(raw[548]))
                    .wrapping_sub(64)
            })
            .collect();
        let payload = decompress(&plain, decoded)?;
        let bit_bytes = (BIT_BYTES - (starts.bit >> 3)) as usize;
        let integer_bytes = 4 * (INTEGERS - starts.integer) as usize;
        let fixed = bit_bytes + integer_bytes;
        if payload.len() < fixed {
            return Err(FormatError("CSS0 payload shorter than its variable ranges"));
        }
        let bits = payload[..bit_bytes].to_vec();
        let integers = payload[bit_bytes..fixed]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| i32::from_le_bytes(*b))
            .collect();
        let mut cursor = fixed;
        let mut read_string = |bounded: bool| -> Result<Vec<u8>, FormatError> {
            let remaining = &payload[cursor..];
            let n = remaining
                .iter()
                .position(|b| *b == 0)
                .ok_or(FormatError("unterminated CSS0 string"))?;
            if bounded && n >= 1024 {
                return Err(FormatError("CSS0 string exceeds its 1024-byte slot"));
            }
            cursor += n + 1;
            Ok(remaining[..n].to_vec())
        };
        let strings = (starts.string..STRINGS)
            .map(|_| read_string(true))
            .collect::<Result<Vec<_>, _>>()?;
        let mut lists = [Vec::new(), Vec::new()];
        for (list, count) in lists.iter_mut().zip(counts) {
            for _ in 0..count {
                list.push(read_string(false)?);
            }
        }
        if cursor != payload.len() {
            return Err(FormatError("uninterpreted trailing CSS0 payload"));
        }
        Ok(Self {
            starts,
            bits,
            integers,
            strings,
            lists,
        })
    }

    /// Returns a new state only on complete success; expression and system registers stay.
    pub fn applied(&self, state: &State) -> Result<State, FormatError> {
        let mut candidate = state.clone();
        let first_bit = self.starts.bit >> 3 << 3;
        for (i, byte) in self.bits.iter().enumerate() {
            for bit in 0..8 {
                candidate
                    .set_bit(first_bit + (i * 8 + bit) as u32, byte & (1 << bit) != 0)
                    .map_err(FormatError)?;
            }
        }
        for (i, value) in self.integers.iter().enumerate() {
            candidate
                .set_integer(self.starts.integer + i as u32, *value)
                .map_err(FormatError)?;
        }
        for (i, bytes) in self.strings.iter().enumerate() {
            candidate
                .set_string(self.starts.string + i as u32, bytes)
                .map_err(FormatError)?;
        }
        Ok(candidate)
    }
}

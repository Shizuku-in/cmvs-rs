//! Local variable records from 0x140069270 / 0x140069fd0, not an engine snapshot.
use super::{SnapshotRecord, parse_records, unique_body};
use crate::{FormatError, evaluate::State};

const INTEGERS: usize = 2048;
const FLOATS: usize = 1024;
const BIT_BYTES: usize = 256;
const STRINGS: usize = 64;

/// Only the local banks persisted in records 0x280..0x283. Higher/persistent
/// variables, expression registers, objects and device state are separate domains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVariables {
    integers: Vec<i32>,
    floats: Vec<u32>,
    bits: Vec<u8>,
    strings: Vec<Vec<u8>>,
}

impl LocalVariables {
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::from_records(bytes, &parse_records(bytes)?)
    }

    pub(super) fn from_records(
        bytes: &[u8],
        records: &[SnapshotRecord],
    ) -> Result<Self, FormatError> {
        let ints = unique_body(bytes, records, 0x280)?;
        let floats = unique_body(bytes, records, 0x281)?;
        let bits = unique_body(bytes, records, 0x282)?;
        let raw_strings = unique_body(bytes, records, 0x283)?;
        if ints.len() != INTEGERS * 4 || floats.len() != FLOATS * 4 || bits.len() != BIT_BYTES {
            return Err(FormatError(
                "snapshot local bank length differs from verified layout",
            ));
        }
        if raw_strings.len() > STRINGS * 1024 {
            return Err(FormatError(
                "snapshot string bank exceeds 64 bounded strings",
            ));
        }
        let mut strings = Vec::with_capacity(STRINGS);
        let mut tail = raw_strings;
        for _ in 0..STRINGS {
            let end = tail
                .iter()
                .position(|&b| b == 0)
                .filter(|&end| end < 1024)
                .ok_or(FormatError(
                    "snapshot string is missing its bounded terminator",
                ))?;
            strings.push(tail[..end].to_vec());
            tail = &tail[end + 1..];
        }
        if !tail.is_empty() {
            return Err(FormatError("bytes remain after 64 snapshot strings"));
        }
        Ok(Self {
            integers: ints
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| i32::from_le_bytes(*b))
                .collect(),
            floats: floats
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect(),
            bits: bits.to_vec(),
            strings,
        })
    }

    /// Copy known local values only. No implicit zeroes and no bit-0 save policy:
    /// a future full capture must separately set captured bit 0 and clear live bit 0.
    pub fn from_state(state: &State) -> Result<Self, FormatError> {
        let mut integers = Vec::with_capacity(INTEGERS);
        let mut floats = Vec::with_capacity(FLOATS);
        let mut bits = vec![0; BIT_BYTES];
        let mut strings = Vec::with_capacity(STRINGS);
        for i in 0..INTEGERS {
            integers.push(
                state
                    .integer(i as u32)
                    .ok_or(FormatError("snapshot capture reads unknown local integer"))?,
            );
        }
        for i in 0..FLOATS {
            floats.push(
                state
                    .float_bits(i as u32)
                    .ok_or(FormatError("snapshot capture reads unknown local float"))?,
            );
        }
        for i in 0..BIT_BYTES * 8 {
            let value = state
                .bit(i as u32)
                .ok_or(FormatError("snapshot capture reads unknown local bit"))?;
            bits[i / 8] |= u8::from(value) << (i % 8);
        }
        for i in 0..STRINGS {
            strings.push(
                state
                    .string(i as u32)
                    .ok_or(FormatError("snapshot capture reads unknown local string"))?
                    .to_vec(),
            );
        }
        Ok(Self {
            integers,
            floats,
            bits,
            strings,
        })
    }

    /// The engine capture: the banks are zero-initialized statics (0x140109ef0, 0x14012df00,
    /// 0x14012ff00, 0x14010df00 lie in the unloaded part of .data), and every script write
    /// the original performs on a path Rust runs is also a Rust write, so a local Rust never
    /// wrote holds zero (an empty string) in the original.
    pub fn capture(state: &State) -> Self {
        let mut bits = vec![0; BIT_BYTES];
        for i in 0..BIT_BYTES * 8 {
            bits[i / 8] |= u8::from(state.bit(i as u32).unwrap_or(false)) << (i % 8);
        }
        Self {
            integers: (0..INTEGERS)
                .map(|i| state.integer(i as u32).unwrap_or(0))
                .collect(),
            floats: (0..FLOATS)
                .map(|i| state.float_bits(i as u32).unwrap_or(0))
                .collect(),
            bits,
            strings: (0..STRINGS)
                .map(|i| state.string(i as u32).unwrap_or_default().to_vec())
                .collect(),
        }
    }

    pub fn integers(&self) -> &[i32] {
        &self.integers
    }
    /// Raw bits preserve NaNs, infinities and signed zero.
    pub fn float_bits(&self) -> &[u32] {
        &self.floats
    }
    pub fn packed_bits(&self) -> &[u8] {
        &self.bits
    }
    pub fn strings(&self) -> &[Vec<u8>] {
        &self.strings
    }

    /// The snapshot writer sets bit 0 for the captured copy (0x140085ce0(0, 1)).
    pub fn set_bit0(&mut self) {
        if let Some(first) = self.bits.first_mut() {
            *first |= 1;
        }
    }

    /// Encode just these four records, not a complete snapshot or CSV2 file.
    pub fn encode_records(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let ints: Vec<u8> = self.integers.iter().flat_map(|v| v.to_le_bytes()).collect();
        let floats: Vec<u8> = self.floats.iter().flat_map(|v| v.to_le_bytes()).collect();
        let strings: Vec<u8> = self
            .strings
            .iter()
            .flat_map(|s| s.iter().copied().chain([0]))
            .collect();
        for (tag, bytes) in [
            (0x280u16, ints.as_slice()),
            (0x281, floats.as_slice()),
            (0x282, self.bits.as_slice()),
            (0x283, strings.as_slice()),
        ] {
            out.extend(tag.to_le_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
        }
        out
    }

    /// Atomic local-bank installation only. Keeps persistent/higher banks,
    /// expression and system registers, string accumulator and all hosts unchanged.
    /// This must not be presented as completing 0x22b5 engine restoration.
    pub fn apply_to_state(&self, state: &mut State) -> Result<(), FormatError> {
        let mut candidate = state.clone();
        for (i, &value) in self.integers.iter().enumerate() {
            candidate
                .set_integer(i as u32, value)
                .map_err(FormatError)?;
        }
        for (i, &value) in self.floats.iter().enumerate() {
            candidate
                .fill_floats(i as u32, 1, value)
                .map_err(FormatError)?;
        }
        for i in 0..BIT_BYTES * 8 {
            candidate
                .set_bit(i as u32, self.bits[i / 8] & (1 << (i % 8)) != 0)
                .map_err(FormatError)?;
        }
        for (i, value) in self.strings.iter().enumerate() {
            candidate.set_string(i as u32, value).map_err(FormatError)?;
        }
        *state = candidate;
        Ok(())
    }
}

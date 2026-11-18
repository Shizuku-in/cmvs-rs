//! CSS0, the CVNS system save (`save000.dat`): cvns-20090203 writer 0x420460, reader
//! 0x4208e0. The persistent ranges are the tails of the variable banks from the starts set
//! by CVNS 0x210b..0x210d; the same LZSS as CSS1 packs them.
use super::decompress;
use crate::{FormatError, MAX_PAYLOAD, evaluate::State, random::CrtRandom};

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
    /// Inspects literal `0x50b..=0x50d` (`bit`, `integer`, `string` starts) from the startup script prefix.
    pub fn from_start(
        script: &crate::ps3::Script,
        map: &crate::cvns::CommandMap,
    ) -> Result<Self, FormatError> {
        use crate::bytecode::{self, InstructionKind, Term};
        let instructions =
            bytecode::decode_layout(script.code(), script.header().entry_pc, &script.labels())
                .map_err(|_| FormatError("invalid startup script layout"))?;
        let mut starts = Self::default();
        let mut value = None;
        let mut pushed = None;
        for instruction in instructions {
            if instruction.pc < script.header().entry_pc as usize {
                continue;
            }
            match instruction.kind {
                InstructionKind::Expression(expression) if expression.kind == 0x200 => {
                    value = match expression.terms.as_slice() {
                        [
                            Term::Scalar {
                                tag: 0x100, raw, ..
                            },
                        ] => Some(*raw),
                        _ => None,
                    };
                }
                InstructionKind::Simple if instruction.opcode == 0x430 => pushed = value.take(),
                InstructionKind::System { command } => {
                    use crate::cvns::cmd;
                    match map.runtime(command) {
                        cmd::PERSISTENT_BIT_START => {
                            starts.bit = pushed
                                .take()
                                .ok_or(FormatError("dynamic CVNS persistent bit start"))?;
                        }
                        cmd::PERSISTENT_INT_START => {
                            starts.integer = pushed
                                .take()
                                .ok_or(FormatError("dynamic CVNS persistent integer start"))?;
                        }
                        cmd::PERSISTENT_STR_START => {
                            starts.string = pushed
                                .take()
                                .ok_or(FormatError("dynamic CVNS persistent string start"))?;
                        }
                        cmd::SYSTEM_SAVE_READ => break,
                        _ => pushed = None,
                    }
                }
                InstructionKind::Jump { .. }
                | InstructionKind::ConditionalJump { .. }
                | InstructionKind::CompareJump { .. }
                | InstructionKind::LongJump { .. }
                | InstructionKind::ScriptReturn { .. } => break,
                _ => {
                    value = None;
                    pushed = None;
                }
            }
        }
        starts.checked()
    }

    pub(crate) fn checked(self) -> Result<Self, FormatError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Css0Parameters {
    pub key_add: u8,
    pub byte_add: u8,
}

impl Css0Parameters {
    /// Two CRT draws matching cvns-20090203 0x420460: `[546] = (u8)rand()`,
    /// `[548] = (rand() & 0x7f) + 128`.
    pub fn draw(random: &mut CrtRandom) -> Self {
        let key_add = random.next_value() as u8;
        let byte_add = (random.next_value() as u8 & 127) + 128;
        Self { key_add, byte_add }
    }
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

    /// Encodes a CSS0 `save000.dat` payload (cvns-20090203 0x420460).
    pub fn encode_state(
        state: &State,
        starts: PersistentStarts,
        title: &[u8],
        parameters: Css0Parameters,
        lists: [&[Vec<u8>]; 2],
    ) -> Result<Vec<u8>, FormatError> {
        let starts = starts.checked()?;
        if title.contains(&0) || title.len() >= 256 {
            return Err(FormatError("CSS0 title contains NUL or exceeds capacity"));
        }
        if lists.iter().any(|list| list.len() > 4096) {
            return Err(FormatError("CSS0 output list exceeds research bounds"));
        }
        let mut payload = Vec::with_capacity(64 * 1024);
        let first_bit = starts.bit >> 3 << 3;
        for base in (first_bit..BIT_BYTES * 8).step_by(8) {
            let mut byte = 0u8;
            for bit in 0..8 {
                let set = state
                    .bit(base + bit)
                    .ok_or(FormatError("unknown persistent CSS0 bit"))?;
                byte |= u8::from(set) << bit;
            }
            payload.push(byte);
        }
        for index in starts.integer..INTEGERS {
            let value = state
                .integer(index)
                .ok_or(FormatError("unknown persistent CSS0 integer"))?;
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for index in starts.string..STRINGS {
            let value = state
                .string(index)
                .ok_or(FormatError("unknown persistent CSS0 string"))?;
            super::write::add_string(&mut payload, value, true)?;
        }
        for list in lists {
            for name in list {
                super::write::add_string(&mut payload, name, false)?;
            }
        }
        if payload.len() > MAX_PAYLOAD {
            return Err(FormatError("CSS0 payload exceeds research bounds"));
        }
        let packed = super::pack::compress(&payload);
        let end = CSS0_HEADER + packed.len();
        if end + 2 > MAX_PAYLOAD {
            return Err(FormatError("CSS0 file exceeds research bounds"));
        }
        let mut raw = vec![0u8; CSS0_HEADER];
        raw[..4].copy_from_slice(b"CSS0");
        raw[16..16 + title.len()].copy_from_slice(title);
        for (at, value) in [
            (4, CSS0_HEADER as u32),
            (540, end as u32),
            (552, lists[0].len() as u32),
            (556, lists[1].len() as u32),
            (560, packed.len() as u32),
            (580, payload.len() as u32),
        ] {
            raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        raw[544..546].copy_from_slice(&1u16.to_le_bytes());
        raw[546..548].copy_from_slice(&u16::from(parameters.key_add).to_le_bytes());
        raw[548..550].copy_from_slice(&u16::from(parameters.byte_add).to_le_bytes());
        let mut sum = 0u16;
        for (i, byte) in packed.into_iter().enumerate() {
            let coded = (byte.wrapping_add(64)
                ^ parameters.key_add.wrapping_add(KEY[i % KEY.len()]))
            .wrapping_add(parameters.byte_add);
            sum = sum.wrapping_add(u16::from(coded));
            raw.push(coded);
        }
        raw.extend_from_slice(&sum.to_le_bytes());
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css0_encode_decode_applied_round_trips_persistent_banks_and_lists() {
        fn filled_state() -> State {
            let mut state = State::new(4096, 262_144);
            state.set_string_slots(256);
            state.fill_bits(0, 262_144, false).unwrap();
            state.fill_integers(0, 4096, 0).unwrap();
            state.fill_strings(0, 256, b"").unwrap();
            state
        }
        let starts = PersistentStarts {
            bit: 0x1013,
            integer: 0x410,
            string: 0x24,
        };
        let mut state = filled_state();
        state.set_bit(0x1008, true).unwrap();
        state.set_bit(0x1010, true).unwrap();
        state.set_bit(0x1013, true).unwrap();
        state.set_bit(262_143, true).unwrap();
        state.set_integer(0x410, -123_456).unwrap();
        state.set_integer(4095, 987_654).unwrap();
        state.set_string(0x24, b"flag_alpha").unwrap();
        state.set_string(255, b"tail_str").unwrap();
        // Volatile variables outside persistent ranges are ignored on save/apply.
        state.set_integer(0x10, 777).unwrap();

        let mut rng = CrtRandom::new(0x1234_5678);
        let params = Css0Parameters::draw(&mut rng);
        assert!(params.byte_add >= 128);
        let l0 = vec![b"ev01".to_vec(), b"ev02".to_vec()];
        let l1 = vec![b"bgm03".to_vec()];
        let raw = Css0Save::encode_state(&state, starts, b"miharu", params, [&l0, &l1]).unwrap();
        let decoded = Css0Save::decode(&raw, starts).unwrap();
        assert_eq!(decoded.lists(), &[l0, l1]);

        let mut fresh = filled_state();
        fresh.set_integer(0x10, 42).unwrap();
        let applied = decoded.applied(&fresh).unwrap();
        // Bits before (starts.bit >> 3 << 3) == 0x1010 stay untouched; the partial byte
        // starting at 0x1010 and tail bits up to 262143 persist.
        assert_eq!(applied.bit(0x1008), Some(false));
        assert_eq!(applied.bit(0x1010), Some(true));
        assert_eq!(applied.bit(0x1013), Some(true));
        assert_eq!(applied.bit(262_143), Some(true));
        assert_eq!(applied.integer(0x410), Some(-123_456));
        assert_eq!(applied.integer(4095), Some(987_654));
        assert_eq!(applied.integer(0x10), Some(42));
        assert_eq!(applied.string(0x24), Some(b"flag_alpha".as_slice()));
        assert_eq!(applied.string(255), Some(b"tail_str".as_slice()));

        assert!(
            Css0Save::decode(
                &raw,
                PersistentStarts {
                    integer: starts.integer + 1,
                    ..starts
                }
            )
            .is_err()
        );
        let mut corrupt = raw.clone();
        corrupt[CSS0_HEADER] ^= 1;
        assert!(Css0Save::decode(&corrupt, starts).is_err());
    }
}

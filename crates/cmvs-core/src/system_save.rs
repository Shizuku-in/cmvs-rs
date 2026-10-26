//! CSS1 persistent variable data. No system-save writing or gameplay snapshot restore.
use crate::{FormatError, MAX_PAYLOAD, evaluate::State, machine::Machine, vm::SystemResult};
use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const PAYLOAD_OFFSET: usize = 360;
const KEY: &[u8] = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
const STRING_OFFSET: usize = 44800;

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().expect("validated CSS1 header"))
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("validated CSS1 header"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Css1Layout {
    pub game_id: u32,
    pub decoded_size: usize,
    pub packed_size: usize,
    pub list_counts: [usize; 2],
}

impl Css1Layout {
    pub fn parse(header: &[u8], file_size: u64) -> Result<Self, FormatError> {
        if header.len() < PAYLOAD_OFFSET {
            return Err(FormatError("truncated CSS1 metadata"));
        }
        if &header[..4] != b"CSS1" || u32_at(header, 4) != 104 || u16_at(header, 76) != 1 {
            return Err(FormatError("unsupported CSS1 header or encoding mode"));
        }
        let end = u64::from(u32_at(header, 100));
        let packed = u64::from(u32_at(header, 96));
        let decoded = u64::from(u32_at(header, 92));
        let counts = [u32_at(header, 88), u32_at(header, 84)];
        if file_size > MAX_PAYLOAD as u64
            || decoded > MAX_PAYLOAD as u64
            || end != PAYLOAD_OFFSET as u64 + packed
            || file_size != end + 2
        {
            return Err(FormatError("CSS1 lengths exceed bounds or disagree"));
        }
        if counts.iter().any(|n| *n > 4096)
            || decoded < STRING_OFFSET as u64 + 64 + u64::from(counts[0]) + u64::from(counts[1])
        {
            return Err(FormatError(
                "CSS1 decoded layout or list counts exceed bounds",
            ));
        }
        Ok(Self {
            game_id: u32_at(header, 68),
            decoded_size: decoded as usize,
            packed_size: packed as usize,
            list_counts: counts.map(|n| n as usize),
        })
    }
}

fn decompress(input: &[u8], expected: usize) -> Result<Vec<u8>, FormatError> {
    let mut ring = [0u8; 1024];
    let (mut pos, mut cursor, mut flags) = (976, 0, 0u16);
    let mut out = Vec::with_capacity(expected);
    while cursor < input.len() {
        flags >>= 1;
        if flags & 256 == 0 {
            flags = u16::from(input[cursor]) | 0xff00;
            cursor += 1;
        }
        if flags & 1 != 0 {
            let value = *input
                .get(cursor)
                .ok_or(FormatError("truncated CSS1 LZSS literal"))?;
            cursor += 1;
            if out.len() == expected {
                return Err(FormatError("CSS1 LZSS output exceeds declared size"));
            }
            out.push(value);
            ring[pos] = value;
            pos = (pos + 1) & 1023;
        } else {
            let pair = input
                .get(cursor..cursor + 2)
                .ok_or(FormatError("truncated CSS1 LZSS reference"))?;
            cursor += 2;
            let offset = usize::from(pair[0]) | (usize::from(pair[1] & 192) << 2);
            let count = usize::from(pair[1] & 63) + 2;
            if count > expected - out.len() {
                return Err(FormatError("CSS1 LZSS output exceeds declared size"));
            }
            for j in 0..count {
                let value = ring[(offset + j) & 1023];
                out.push(value);
                ring[pos] = value;
                pos = (pos + 1) & 1023;
            }
        }
    }
    if out.len() != expected {
        return Err(FormatError("CSS1 LZSS decoded length mismatch"));
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Css1Save {
    layout: Css1Layout,
    payload: Vec<u8>,
    strings: Vec<Vec<u8>>,
    /// Original object lists +12016, +12024; opaque byte names, not filesystem writes.
    lists: [Vec<Vec<u8>>; 2],
}

impl Css1Save {
    pub fn layout(&self) -> &Css1Layout {
        &self.layout
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn strings(&self) -> &[Vec<u8>] {
        &self.strings
    }
    pub fn lists(&self) -> &[Vec<Vec<u8>>; 2] {
        &self.lists
    }
    pub fn decode(raw: &[u8]) -> Result<Self, FormatError> {
        let layout = Css1Layout::parse(raw, raw.len() as u64)?;
        let end = PAYLOAD_OFFSET + layout.packed_size;
        let packed = &raw[PAYLOAD_OFFSET..end];
        let sum = packed
            .iter()
            .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
        if sum != u16_at(raw, end) {
            return Err(FormatError("CSS1 payload checksum mismatch"));
        }
        let shift = u32::from((u16_at(raw, 82) >> ((u16_at(raw, 78) >> 1) & 7)) & 3) + 2;
        let plain: Vec<u8> = packed
            .iter()
            .enumerate()
            .map(|(i, b)| {
                (b.rotate_right(shift).wrapping_sub(raw[78])
                    ^ raw[80].wrapping_add(KEY[i % KEY.len()]))
                .wrapping_sub(64)
            })
            .collect();
        let payload = decompress(&plain, layout.decoded_size)?;
        let mut cursor = STRING_OFFSET;
        let mut read_string = |bounded: bool| -> Result<Vec<u8>, FormatError> {
            let remaining = &payload[cursor..];
            let n = remaining
                .iter()
                .position(|b| *b == 0)
                .ok_or(FormatError("unterminated CSS1 string"))?;
            if bounded && n >= 1024 {
                return Err(FormatError("CSS1 string exceeds destination capacity"));
            }
            let value = remaining[..n].to_vec();
            cursor += n + 1;
            Ok(value)
        };
        let strings = (0..64)
            .map(|_| read_string(true))
            .collect::<Result<Vec<_>, _>>()?;
        let mut lists = [Vec::new(), Vec::new()];
        for (list, count) in lists.iter_mut().zip(layout.list_counts) {
            for _ in 0..count {
                list.push(read_string(false)?);
            }
        }
        if cursor != payload.len() {
            return Err(FormatError("uninterpreted trailing CSS1 payload"));
        }
        Ok(Self {
            layout,
            payload,
            strings,
            lists,
        })
    }

    /// Returns a new state only on complete success; preserves expression/system registers.
    pub fn applied(
        &self,
        state: &State,
        expected_game_id: Option<u32>,
    ) -> Result<State, FormatError> {
        if self.layout.game_id != 0 && expected_game_id != Some(self.layout.game_id) {
            return Err(FormatError("CSS1 game identifier differs or is unknown"));
        }
        let mut candidate = state.clone();
        for (i, byte) in self.payload[..32512].iter().enumerate() {
            for bit in 0..8 {
                candidate
                    .set_bit(2048 + (i * 8 + bit) as u32, byte & (1 << bit) != 0)
                    .map_err(FormatError)?;
            }
        }
        for (i, bytes) in self.payload[32512..40704].chunks_exact(4).enumerate() {
            candidate
                .set_integer(
                    2048 + i as u32,
                    i32::from_le_bytes(bytes.try_into().expect("four bytes")),
                )
                .map_err(FormatError)?;
        }
        for (i, bytes) in self.payload[40704..44800].chunks_exact(4).enumerate() {
            candidate
                .fill_floats(
                    1024 + i as u32,
                    1,
                    u32::from_le_bytes(bytes.try_into().expect("four bytes")),
                )
                .map_err(FormatError)?;
        }
        for (i, bytes) in self.strings.iter().enumerate() {
            candidate
                .fill_strings(64 + i as u32, 1, bytes)
                .map_err(FormatError)?;
        }
        Ok(candidate)
    }
}

pub trait SystemSaveSource {
    fn read(&mut self) -> io::Result<Option<Vec<u8>>>;
}
pub struct FileSystemSave {
    path: PathBuf,
}
impl FileSystemSave {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
        }
    }
}
impl SystemSaveSource for FileSystemSave {
    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut header = [0; PAYLOAD_OFFSET];
        file.read_exact(&mut header)?;
        let size = file.metadata()?.len();
        Css1Layout::parse(&header, size).map_err(io::Error::other)?;
        let mut raw = header.to_vec();
        file.take(MAX_PAYLOAD as u64 + 1 - PAYLOAD_OFFSET as u64)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 != size {
            return Err(io::Error::other("CSS1 file changed length while reading"));
        }
        Ok(Some(raw))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemSaveEvent {
    Missing,
    Applied,
}

#[derive(Default)]
pub struct SystemSaveHost {
    pub loaded: Option<Css1Save>,
    /// Zero-count lists leave the previous list unchanged in the original loader.
    pub lists: [Option<Vec<Vec<u8>>>; 2],
}
impl SystemSaveHost {
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut impl SystemSaveSource,
        game_id: Option<u32>,
    ) -> Result<Option<SystemSaveEvent>, Box<dyn Error>> {
        if !matches!(vm.pending_system(), Some((_, 0x128))) {
            return Ok(None);
        }
        let Some(raw) = source.read()? else {
            vm.complete_system(SystemResult(0x4000))?;
            return Ok(Some(SystemSaveEvent::Missing));
        };
        let save = Css1Save::decode(&raw)?;
        let candidate = save.applied(vm.state(), game_id)?;
        vm.complete_system(SystemResult(0x4000))?;
        *vm.state_mut() = candidate;
        for (current, new) in self.lists.iter_mut().zip(&save.lists) {
            if !new.is_empty() {
                *current = Some(new.clone());
            }
        }
        self.loaded = Some(save);
        Ok(Some(SystemSaveEvent::Applied))
    }
}

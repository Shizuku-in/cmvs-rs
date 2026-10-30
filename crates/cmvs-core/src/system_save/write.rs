//! Mode-1 CSS1 output from fully known state (0x1400179e0). Original game stays read-only.
use super::{Css1Save, KEY, PAYLOAD_OFFSET, pack};
use crate::{FormatError, evaluate::State, machine::Machine, random::CrtRandom, vm::SystemResult};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Original writer's four CRT draws, including the wrapping 16-bit product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Css1Parameters {
    pub byte_add: u8,
    pub key_add: u8,
    pub rotation_word: u16,
}
impl Css1Parameters {
    pub fn draw(random: &mut CrtRandom) -> Self {
        let byte_add = random.next_value() as u8;
        let key_add = (random.next_value() as u8 & 127) + 128;
        let product = (random.next_value() as u16).wrapping_mul(random.next_value() as u16);
        Self {
            byte_add,
            key_add,
            rotation_word: product.wrapping_sub(26797),
        }
    }
}

fn add_string(payload: &mut Vec<u8>, value: &[u8], bounded: bool) -> Result<(), FormatError> {
    if value.contains(&0) || (bounded && value.len() >= 1024) {
        return Err(FormatError(
            "CSS1 output string contains NUL or exceeds capacity",
        ));
    }
    // Original serialization allocates a 1 MiB scratch buffer.
    if payload.len().saturating_add(value.len()).saturating_add(1) > 1024 * 1024 {
        return Err(FormatError("CSS1 output exceeds original scratch capacity"));
    }
    payload.extend_from_slice(value);
    payload.push(0);
    Ok(())
}

impl Css1Save {
    /// Reuse only the source's opaque annotation. Output flags/extra region are zero;
    /// this is the writer branch before unimplemented 0x212c can enable extra metadata.
    pub fn encode_state(
        &self,
        state: &State,
        game_id: u32,
        parameters: Css1Parameters,
        lists: [&[Vec<u8>]; 2],
    ) -> Result<Vec<u8>, FormatError> {
        Self::encode_with_marker(&self.marker, state, game_id, parameters, lists)
    }

    /// First save without a source file. The annotation is not consumed by the
    /// native loader (0x140018da0); identify our output instead of inventing a
    /// loaded template. Persistent variables must still all be known.
    pub fn encode_initial_state(
        state: &State,
        game_id: u32,
        parameters: Css1Parameters,
        lists: [&[Vec<u8>]; 2],
    ) -> Result<Vec<u8>, FormatError> {
        let mut marker = [0; 60];
        marker[..7].copy_from_slice(b"cmvs-rs");
        Self::encode_with_marker(&marker, state, game_id, parameters, lists)
    }

    fn encode_with_marker(
        marker: &[u8; 60],
        state: &State,
        game_id: u32,
        parameters: Css1Parameters,
        lists: [&[Vec<u8>]; 2],
    ) -> Result<Vec<u8>, FormatError> {
        if lists.iter().any(|list| list.len() > 4096) {
            return Err(FormatError("CSS1 output list exceeds research bound"));
        }
        let mut payload = Vec::with_capacity(44864);
        for base in (2048..262144).step_by(8) {
            let mut byte = 0;
            for bit in 0..8 {
                let set = state
                    .bit(base + bit)
                    .ok_or(FormatError("unknown persistent CSS1 bit"))?;
                byte |= u8::from(set) << bit;
            }
            payload.push(byte);
        }
        for index in 2048..4096 {
            let value = state
                .integer(index)
                .ok_or(FormatError("unknown persistent CSS1 integer"))?;
            payload.extend(value.to_le_bytes());
        }
        for index in 1024..2048 {
            let value = state
                .float_bits(index)
                .ok_or(FormatError("unknown persistent CSS1 float"))?;
            payload.extend(value.to_le_bytes());
        }
        for index in 64..128 {
            add_string(
                &mut payload,
                state
                    .string(index)
                    .ok_or(FormatError("unknown persistent CSS1 string"))?,
                true,
            )?;
        }
        for list in lists {
            for name in list {
                add_string(&mut payload, name, false)?;
            }
        }
        let packed = pack::compress(&payload);
        let mut raw = vec![0u8; PAYLOAD_OFFSET];
        raw[..4].copy_from_slice(b"CSS1");
        raw[8..68].copy_from_slice(marker);
        for (at, value) in [
            (4, 104),
            (68, game_id),
            (84, lists[1].len() as u32),
            (88, lists[0].len() as u32),
            (92, payload.len() as u32),
            (96, packed.len() as u32),
            (100, (PAYLOAD_OFFSET + packed.len()) as u32),
        ] {
            raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        raw[76..78].copy_from_slice(&1u16.to_le_bytes());
        raw[78..80].copy_from_slice(&u16::from(parameters.byte_add).to_le_bytes());
        raw[80..82].copy_from_slice(&u16::from(parameters.key_add).to_le_bytes());
        raw[82..84].copy_from_slice(&parameters.rotation_word.to_le_bytes());
        let shift =
            u32::from((parameters.rotation_word >> ((parameters.byte_add >> 1) & 7)) & 3) + 2;
        let mut sum = 0u16;
        for (i, byte) in packed.into_iter().enumerate() {
            let coded = ((byte.wrapping_add(64)
                ^ parameters.key_add.wrapping_add(KEY[i % KEY.len()]))
            .wrapping_add(parameters.byte_add))
            .rotate_left(shift);
            sum = sum.wrapping_add(u16::from(coded));
            raw.push(coded);
        }
        raw.extend(sum.to_le_bytes());
        Ok(raw)
    }
}

/// Complete the output before returning Ok; Err leaves VM/RNG pending for retry.
pub trait SystemSaveSink {
    fn write(&mut self, raw: &[u8]) -> io::Result<()>;
}

/// An explicitly selected new file, outside the canonical read-only game root.
/// This export policy never replaces an existing file or creates a game backup.
pub struct NewSystemSave {
    path: PathBuf,
}
impl NewSystemSave {
    pub fn new(path: &Path, game_root: &Path) -> io::Result<Self> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::other("CSS1 output needs a filename"))?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize()?;
        if parent.starts_with(game_root.canonicalize()?) {
            return Err(io::Error::other(
                "CSS1 output must be outside the read-only game root",
            ));
        }
        let path = parent.join(name);
        if path.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "CSS1 output already exists",
            ));
        }
        Ok(Self { path })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl SystemSaveSink for NewSystemSave {
    fn write(&mut self, raw: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)?;
        // A failed write retains its partial file; retries cannot silently overwrite it.
        file.write_all(raw)?;
        file.sync_all()
    }
}

impl super::SystemSaveHost {
    /// 0x2129: runtime flags 0x212a/b/c remain unsupported, so both name lists and
    /// extra metadata stay disabled. Fully known persistent state is required.
    pub fn try_write(
        &mut self,
        vm: &mut Machine<'_>,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<bool, Box<dyn Error>> {
        if !matches!(vm.pending_system(), Some((_, 0x129))) {
            return Ok(false);
        }
        self.write_now(vm.state(), sink, random, game_id)?;
        vm.complete_system(SystemResult(0x4000))?;
        Ok(true)
    }

    /// The CSS1 writer 0x1400179e0 itself, which the slot writer 0x140080b20 also calls.
    /// Random draws are committed only after the sink accepted the file.
    pub fn write_now(
        &mut self,
        state: &crate::evaluate::State,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<(), Box<dyn Error>> {
        let game_id = game_id.ok_or("CSS1 write requires a known game identifier")?;
        if self.loaded.as_ref().is_some_and(|template| {
            template.layout.game_id != 0 && template.layout.game_id != game_id
        }) {
            return Err("CSS1 write game identifier mismatch".into());
        }
        let mut candidate = random.clone();
        let parameters = Css1Parameters::draw(&mut candidate);
        let raw = match &self.loaded {
            Some(template) => template.encode_state(state, game_id, parameters, [&[], &[]])?,
            None => Css1Save::encode_initial_state(state, game_id, parameters, [&[], &[]])?,
        };
        sink.write(&raw)?;
        *random = candidate;
        Ok(())
    }
}

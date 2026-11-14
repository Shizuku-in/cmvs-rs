//! Script containers and decoded label, code, data, and text segments.
//!
//! [`Script::decode`] accepts PS2A (`.ps2`/`.ps3`) and CVNS-era PSS0 (`.pss`) files.
//! It validates segment bounds and decompresses the payload without executing code.
//! Use [`crate::bytecode`] to inspect instructions or [`crate::runner`] to run a game.
//!
//! Text remains encoded bytes because the header does not identify its encoding.
//! [`Script::from_memory_image`] reads an already-decoded save image; it must not be
//! given an ordinary packed script file.
use crate::{FormatError, MAX_PAYLOAD, lzss};

/// Size of the on-disk and memory-image script header, in bytes.
pub const HEADER_SIZE: usize = 48;

/// Which container a script came from. PSS0 stores zero in the two fields PS2A
/// uses as transform parameters and applies no byte transform before LZSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Ps2a,
    Pss0,
}

impl Container {
    pub fn magic(self) -> &'static [u8; 4] {
        match self {
            Self::Ps2a => b"PS2A",
            Self::Pss0 => b"PSS0",
        }
    }
}

/// Script container metadata; sizes and entry offsets are measured in bytes.
#[derive(Debug, Clone)]
pub struct Header {
    pub container: Container,
    pub version_raw: u32,
    pub key: u32,
    pub label_count: u32,
    pub code_size: u32,
    pub unknown_size: u32,
    pub text_size: u32,
    pub entry_pc: u32,
    pub compressed_size: u32,
    pub decoded_size: u32,
    pub reserved: u32,
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, FormatError> {
    let bytes = data
        .get(at..at + 4)
        .ok_or(FormatError("truncated 32-bit field"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl Header {
    /// Parse the fixed header without decompressing or validating payload segments.
    ///
    /// Returns an error for an unknown signature or an invalid header length.
    pub fn parse(raw: &[u8]) -> Result<Self, FormatError> {
        let container = match raw.get(..4) {
            Some(b"PS2A") => Container::Ps2a,
            Some(b"PSS0") => Container::Pss0,
            _ => return Err(FormatError("expected PS2A or PSS0 header")),
        };
        if raw.len() < HEADER_SIZE {
            return Err(FormatError("truncated script header"));
        }
        if u32_at(raw, 4)? != HEADER_SIZE as u32 {
            return Err(FormatError("unsupported script header length"));
        }
        Ok(Self {
            container,
            version_raw: u32_at(raw, 8)?,
            key: u32_at(raw, 12)?,
            label_count: u32_at(raw, 16)?,
            code_size: u32_at(raw, 20)?,
            unknown_size: u32_at(raw, 24)?,
            text_size: u32_at(raw, 28)?,
            entry_pc: u32_at(raw, 32)?,
            compressed_size: u32_at(raw, 36)?,
            decoded_size: u32_at(raw, 40)?,
            reserved: u32_at(raw, 44)?,
        })
    }

    fn segment_size(&self) -> u64 {
        u64::from(self.label_count) * 4
            + u64::from(self.code_size)
            + u64::from(self.unknown_size)
            + u64::from(self.text_size)
    }
}

/// A validated script container with an owned, decoded payload.
///
/// Access [`Self::code`], [`Self::data`], and [`Self::text`] for the individual
/// segments. VM execution keeps its own mutable copy of the data segment.
/// Header lengths are read-only so they cannot invalidate the decoded segments.
///
/// ```compile_fail
/// # use cmvs_core::ps3::Script;
/// fn change_length(script: &mut Script) {
///     script.header().code_size = 3;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Script {
    header: Header,
    payload: Vec<u8>,
    /// Number of bytes after the declared compressed payload; zero for uncompressed input.
    pub trailing_file_bytes: usize,
}

/// A nonempty string borrowed from a script's decoded text segment.
#[derive(Debug, PartialEq, Eq)]
pub struct TextEntry<'a> {
    /// Byte offset relative to the start of the text segment.
    pub offset: usize,
    /// Text in the game's encoding, without the NUL terminator.
    pub bytes: &'a [u8],
}

impl Script {
    /// Original header fields, including the container's packed-size metadata.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Read an already-decoded script memory image from a gameplay save.
    ///
    /// CSV2 section 2 retains the original header key and packed-size fields, but
    /// its payload must not be decrypted or decompressed again.
    pub fn from_memory_image(raw: &[u8]) -> Result<Self, FormatError> {
        let header = Header::parse(raw)?;
        let total = header.segment_size();
        if total > MAX_PAYLOAD as u64 || total != u64::from(header.decoded_size) {
            return Err(FormatError("script memory segment sizes disagree"));
        }
        if raw.len() as u64 != HEADER_SIZE as u64 + total {
            return Err(FormatError(
                "script memory image length differs from header",
            ));
        }
        if header.code_size < 2 || header.entry_pc >= header.code_size {
            return Err(FormatError("script entry outside code segment"));
        }
        let script = Self {
            header,
            payload: raw[HEADER_SIZE..].to_vec(),
            trailing_file_bytes: 0,
        };
        if script
            .labels()
            .iter()
            .any(|&pc| pc >= script.header.code_size)
        {
            return Err(FormatError("label outside code segment"));
        }
        Ok(script)
    }

    /// Serialize a save memory image using `data` as the current mutable segment.
    ///
    /// Inverse of [`Self::from_memory_image`]. `data` must match the original data
    /// segment's byte length. The original header fields are preserved.
    pub fn memory_image(&self, data: &[u8]) -> Result<Vec<u8>, FormatError> {
        if data.len() != self.header.unknown_size as usize {
            return Err(FormatError("script data length differs from its segment"));
        }
        let h = &self.header;
        let mut out = h.container.magic().to_vec();
        for word in [
            HEADER_SIZE as u32,
            h.version_raw,
            h.key,
            h.label_count,
            h.code_size,
            h.unknown_size,
            h.text_size,
            h.entry_pc,
            h.compressed_size,
            h.decoded_size,
            h.reserved,
        ] {
            out.extend(word.to_le_bytes());
        }
        let start = self.code_start() + h.code_size as usize;
        out.extend(&self.payload[..start]);
        out.extend(data);
        out.extend(&self.payload[start + data.len()..]);
        Ok(out)
    }

    pub(crate) fn apply_snapshot_data(&mut self, data: &[u8]) -> Result<(), FormatError> {
        if data.len() != self.header.unknown_size as usize {
            return Err(FormatError("snapshot main data length differs from script"));
        }
        let start = self.code_start() + self.header.code_size as usize;
        self.payload[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Decode an on-disk script and validate segment sizes, entry point, and labels.
    ///
    /// Applies the container's byte transform and LZSS decompression when needed.
    /// Returns an error for malformed, truncated, or oversized data. Instruction
    /// decoding is a separate step performed by [`crate::bytecode`].
    pub fn decode(raw: &[u8]) -> Result<Self, FormatError> {
        let header = Header::parse(raw)?;
        let total = header.segment_size();
        if total > MAX_PAYLOAD as u64 || u64::from(header.decoded_size) > MAX_PAYLOAD as u64 {
            return Err(FormatError("script exceeds research allocation limit"));
        }
        if header.code_size < 2 || header.entry_pc >= header.code_size {
            return Err(FormatError("script entry outside code segment"));
        }
        let packed = header.compressed_size as usize;
        let length = if packed == 0 {
            raw.len() - HEADER_SIZE
        } else {
            packed
        };
        if length > MAX_PAYLOAD {
            return Err(FormatError(
                "encoded payload exceeds research allocation limit",
            ));
        }
        let input = raw
            .get(HEADER_SIZE..HEADER_SIZE + length)
            .ok_or(FormatError("truncated script payload"))?;
        let xor = ((header.key >> 3).wrapping_add(header.key >> 24)) as u8;
        let shift = (header.key >> 20) % 5 + 1;
        let transformed: Vec<u8> = match header.container {
            Container::Ps2a => input
                .iter()
                .map(|v| (v.wrapping_sub(124) ^ xor).rotate_right(shift))
                .collect(),
            Container::Pss0 => input.to_vec(),
        };
        let payload = if packed == 0 {
            transformed
        } else {
            lzss::decode(&transformed, header.decoded_size as usize)?
        };
        if payload.len() as u64 != total {
            return Err(FormatError("script segment lengths disagree"));
        }
        let script = Self {
            header,
            payload,
            trailing_file_bytes: raw.len() - HEADER_SIZE - length,
        };
        if script
            .labels()
            .iter()
            .any(|&label| label >= script.header.code_size)
        {
            return Err(FormatError("label outside code segment"));
        }
        Ok(script)
    }

    /// Complete decoded payload, beginning with the label table.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Label targets as byte offsets into [`Self::code`].
    pub fn labels(&self) -> Vec<u32> {
        self.payload[..self.code_start()]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    fn code_start(&self) -> usize {
        self.header.label_count as usize * 4
    }

    /// Encoded VM instructions, without the container header or label table.
    pub fn code(&self) -> &[u8] {
        &self.payload[self.code_start()..self.code_start() + self.header.code_size as usize]
    }

    /// Initial contents of the script's mutable data segment.
    pub fn data(&self) -> &[u8] {
        let start = self.code_start() + self.header.code_size as usize;
        &self.payload[start..start + self.header.unknown_size as usize]
    }

    /// Compatibility name for [`Self::data`].
    pub fn unknown(&self) -> &[u8] {
        self.data()
    }

    /// Raw text segment, including NUL terminators.
    pub fn text(&self) -> &[u8] {
        let start =
            self.code_start() + self.header.code_size as usize + self.header.unknown_size as usize;
        &self.payload[start..]
    }

    /// Split the text segment into nonempty entries without decoding their encoding.
    ///
    /// Returns an error if the last entry has no NUL terminator.
    pub fn strings(&self) -> Result<Vec<TextEntry<'_>>, FormatError> {
        let mut entries = Vec::new();
        let mut offset = 0;
        for bytes in self.text().split_inclusive(|&b| b == 0) {
            if bytes.last() != Some(&0) {
                return Err(FormatError("unterminated text string"));
            }
            if bytes.len() > 1 {
                entries.push(TextEntry {
                    offset,
                    bytes: &bytes[..bytes.len() - 1],
                });
            }
            offset += bytes.len();
        }
        Ok(entries)
    }
}

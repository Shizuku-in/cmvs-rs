//! PS2A containers, including files using the .ps3 extension.
//! Text is deliberately exposed as bytes: the script header does not identify its encoding.
use crate::{FormatError, MAX_PAYLOAD, lzss};

pub const HEADER_SIZE: usize = 48;

#[derive(Debug, Clone)]
pub struct Header {
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
    pub fn parse(raw: &[u8]) -> Result<Self, FormatError> {
        if raw.len() < HEADER_SIZE || &raw[..4] != b"PS2A" {
            return Err(FormatError("expected PS2A header"));
        }
        if u32_at(raw, 4)? != HEADER_SIZE as u32 {
            return Err(FormatError("unsupported PS2A header length"));
        }
        Ok(Self {
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

#[derive(Debug)]
pub struct Script {
    pub header: Header,
    payload: Vec<u8>,
    pub trailing_file_bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextEntry<'a> {
    pub offset: usize,
    pub bytes: &'a [u8],
}

impl Script {
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
        let transformed: Vec<u8> = input
            .iter()
            .map(|v| (v.wrapping_sub(124) ^ xor).rotate_right(shift))
            .collect();
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

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn labels(&self) -> Vec<u32> {
        self.payload[..self.code_start()]
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    fn code_start(&self) -> usize {
        self.header.label_count as usize * 4
    }

    pub fn code(&self) -> &[u8] {
        &self.payload[self.code_start()..self.code_start() + self.header.code_size as usize]
    }

    pub fn unknown(&self) -> &[u8] {
        let start = self.code_start() + self.header.code_size as usize;
        &self.payload[start..start + self.header.unknown_size as usize]
    }

    pub fn text(&self) -> &[u8] {
        let start =
            self.code_start() + self.header.code_size as usize + self.header.unknown_size as usize;
        &self.payload[start..]
    }

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

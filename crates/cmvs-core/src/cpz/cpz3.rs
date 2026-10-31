//! CPZ3's flat index and shared index/resource cipher.
use super::{ArchiveError, Entry};
use crate::FormatError;
use std::io::{Read, Seek, SeekFrom};

pub const CPZ3_HEADER_SIZE: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz3Layout {
    pub file_count: u32,
    pub index_bytes: u32,
    pub index_checksum_raw: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("bounded CPZ3 field"))
}

/// CPZ3 uses a different checksum seed from CPZ5/6/7.
pub fn cpz3_checksum(bytes: &[u8]) -> u32 {
    let (words, tail) = bytes.as_chunks::<4>();
    let sum = words
        .iter()
        .fold(0x78341256u32, |sum, b| sum.wrapping_add(word(b)));
    tail.iter()
        .fold(sum, |sum, &b| sum.wrapping_add(u32::from(b)))
}

fn decrypt(bytes: &mut [u8], key: u32) {
    // Sixteen little-endian words derived from the 64-byte cipher phrase.
    const TABLE: [u32; 16] = [
        0x4d0d4a5e, 0xb3abf3e1, 0x3c37336d, 0x86c3f5f3, 0x7d4f9b89, 0x58d7de11, 0x6367778d,
        0xa5f34629, 0x067fa4b5, 0xed0ae742, 0xb19450cc, 0xe7204a5a, 0xd9af04f5, 0x5d3b687f,
        0xc1c7a6fd, 0xfc502289,
    ];
    let table = TABLE.map(|v| v.wrapping_add(key));
    let shift = ((0..8).fold(0xfdu32, |v, n| v ^ (key >> (4 * n))) & 15) + 8;
    let mut flag = 3;
    let (chunks, tail) = bytes.as_chunks_mut::<4>();
    for chunk in chunks {
        let value = (word(chunk) ^ table[flag])
            .wrapping_add(0x6e58a5c2)
            .rotate_left(shift);
        chunk.copy_from_slice(&value.to_le_bytes());
        flag = (flag + 1) & 15;
    }
    let length = tail.len();
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = (*byte ^ (table[flag] >> (4 * (length - i))) as u8).wrapping_add(0x52);
        flag = (flag + 1) & 15;
    }
}

impl Cpz3Layout {
    /// Bounds-only inspection; CPZ3 has no header checksum.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ3_HEADER_SIZE || &header[..4] != b"CPZ3" {
            return Err(FormatError("expected 20-byte CPZ3 header"));
        }
        let file_count = word(&header[4..]) ^ 0x5e9c4f37;
        let index_bytes = word(&header[8..]) ^ 0xf32aed17;
        let data_offset = CPZ3_HEADER_SIZE as u64 + u64::from(index_bytes);
        if data_offset > file_bytes {
            return Err(FormatError("CPZ3 index exceeds archive length"));
        }
        if u64::from(file_count) * 25 > u64::from(index_bytes) {
            return Err(FormatError(
                "CPZ3 file records cannot fit in declared index",
            ));
        }
        Ok(Self {
            file_count,
            index_bytes,
            index_checksum_raw: word(&header[12..]),
            data_offset,
            data_bytes: file_bytes - data_offset,
        })
    }
}

#[derive(Debug)]
pub struct Cpz3Index {
    pub layout: Cpz3Layout,
    entries: Vec<Entry>,
    entry_keys: Vec<u32>,
}

impl Cpz3Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ3_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz3Layout::parse(&header, file_bytes)?;
        if layout.index_bytes as u64 > crate::MAX_PAYLOAD as u64 || layout.file_count > 1_000_000 {
            return Err(FormatError("CPZ3 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; layout.index_bytes as usize];
        reader.read_exact(&mut bytes)?;
        if cpz3_checksum(&bytes) != layout.index_checksum_raw {
            return Err(FormatError("CPZ3 index checksum mismatch").into());
        }
        decrypt(&mut bytes, word(&header[16..]) ^ 0xdddddddd ^ 0x7bf4a539);
        let mut entries = Vec::new();
        let mut entry_keys = Vec::new();
        let directory: std::sync::Arc<[u8]> = [].into();
        let mut offset = 0usize;
        for _ in 0..layout.file_count {
            let remaining = &bytes[offset..];
            if remaining.len() < 25 {
                return Err(FormatError("truncated CPZ3 file record").into());
            }
            let length = word(remaining) as usize;
            let rec = remaining
                .get(..length)
                .filter(|b| b.len() >= 25)
                .ok_or(FormatError("CPZ3 record exceeds index or is too short"))?;
            let name_bytes = &rec[24..];
            let end = name_bytes
                .iter()
                .position(|&b| b == 0)
                .ok_or(FormatError("unterminated CPZ3 filename"))?;
            if end == 0 || end > 4096 {
                return Err(FormatError("invalid CPZ3 filename length").into());
            }
            let size = word(&rec[4..]);
            // The x86 reader uses only the low offset word at +8, not +12.
            let absolute = layout.data_offset + u64::from(word(&rec[8..]));
            if absolute + u64::from(size) > file_bytes {
                return Err(FormatError("CPZ3 resource exceeds archive length").into());
            }
            entries.push(Entry {
                directory: directory.clone(),
                name: name_bytes[..end].to_vec(),
                size,
                offset: absolute,
                checksum_raw: word(&rec[16..]),
            });
            entry_keys.push(word(&rec[20..]) ^ 0xc7f5da63);
            offset += length;
        }
        // The original stops at file_count. Real indexes can have a large
        // encrypted padding region; data_offset includes that whole region.
        Ok(Self {
            layout,
            entries,
            entry_keys,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        let mut matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == name);
        let first = matches
            .next()
            .ok_or(FormatError("CPZ3 filename not found"))?
            .0;
        if matches.next().is_some() {
            return Err(FormatError("ambiguous CPZ3 basename"));
        }
        Ok(first)
    }

    /// Fold ASCII query bytes, preserving Shift-JIS pairs, and select the first match.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        if name.len() >= 64 || name.contains(&0) {
            return Err(FormatError("invalid CPZ3 lookup name"));
        }
        let key = super::lookup_key(name)?;
        Ok(self.entries.iter().position(|e| e.name == key))
    }

    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        self.read_resource(reader, index, false)
    }

    /// Check encrypted resource bytes using CPZ3's checksum seed before decrypting.
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        self.read_resource(reader, index, true)
    }

    fn read_resource<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
        verify: bool,
    ) -> Result<Vec<u8>, ArchiveError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(FormatError("CPZ3 entry index out of bounds"))?;
        if entry.size as u64 > crate::MAX_PAYLOAD as u64 {
            return Err(FormatError("CPZ3 resource exceeds research allocation limit").into());
        }
        reader.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0; entry.size as usize];
        reader.read_exact(&mut bytes)?;
        if verify && cpz3_checksum(&bytes) != entry.checksum_raw {
            return Err(FormatError("CPZ3 resource checksum mismatch").into());
        }
        decrypt(&mut bytes, self.entry_keys[index]);
        Ok(bytes)
    }
}

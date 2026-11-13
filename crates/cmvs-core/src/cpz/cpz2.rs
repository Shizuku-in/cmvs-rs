//! CVNS CPZ2 (2005–2009): CPZ3's masked 20-byte header, flat index and checksum,
//! with its own word cipher constants.
use super::flat::{self, word};
use super::{ArchiveError, Entry, cpz3_checksum};
use crate::FormatError;
use std::io::{Read, Seek, SeekFrom};

pub const CPZ2_HEADER_SIZE: usize = 20;

/// The 64-byte Shift-JIS phrase embedded in cvns.exe. The reader XORs it with a
/// byte stream starting at 55 and stepping by -41, then uses it as sixteen
/// little-endian words.
const TABLE: [u32; 16] = {
    let phrase = *b"\x88\xc3\x8d\x86\x82\xcd\x82\xcb\x81\x41\x89\xf0\x90\xcd\x82\xb5\x82\xbf\x82\xe1\x83\x5f\x83\x81\x82\xc8\x82\xcc\x81\x63\x82\xd9\x82\xf1\x82\xc6\x82\xbe\x82\xe6\x81\xf4\x82\xa0\x82\xea\x81\x63\x92\x4e\x82\xc9\x8c\xbe\x82\xc1\x82\xc4\x82\xe9\x82\xcc\x81\x48";
    let mut table = [0; 16];
    let mut i = 0;
    while i < 64 {
        let byte = phrase[i] ^ 55u8.wrapping_sub((i as u8).wrapping_mul(41));
        table[i / 4] |= (byte as u32) << (8 * (i % 4));
        i += 1;
    }
    table
};

fn decrypt(bytes: &mut [u8], key: u32) {
    let table = TABLE.map(|v| v.wrapping_add(key));
    let shift = ((0..8).fold(5u32, |v, n| v ^ (key >> (4 * n))) & 15) + 8;
    let mut flag = 0;
    let (chunks, tail) = bytes.as_chunks_mut::<4>();
    for chunk in chunks {
        let value = (word(chunk) ^ table[flag])
            .wrapping_sub(0x0015_c3e7)
            .rotate_right(shift);
        chunk.copy_from_slice(&value.to_le_bytes());
        flag = (flag + 1) & 15;
    }
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = (*byte ^ (table[flag] >> (4 * i)) as u8).wrapping_add(55);
        flag = (flag + 1) & 15;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz2Layout {
    pub file_count: u32,
    pub index_bytes: u32,
    pub index_checksum_raw: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

impl Cpz2Layout {
    /// Bounds-only inspection; the index checksum is not verified here.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ2_HEADER_SIZE || &header[..4] != b"CPZ2" {
            return Err(FormatError("expected 20-byte CPZ2 header"));
        }
        let file_count = word(&header[4..]) ^ 0xe47c59f3;
        let index_bytes = word(&header[8..]) ^ 0x3f71de2a;
        let data_offset = flat::data_offset(CPZ2_HEADER_SIZE, file_count, index_bytes, file_bytes)?;
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
pub struct Cpz2Index {
    pub layout: Cpz2Layout,
    entries: Vec<Entry>,
    entry_keys: Vec<u32>,
}

impl Cpz2Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ2_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz2Layout::parse(&header, file_bytes)?;
        if layout.index_bytes as u64 > crate::MAX_PAYLOAD as u64 || layout.file_count > 1_000_000 {
            return Err(FormatError("CPZ2 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; layout.index_bytes as usize];
        reader.read_exact(&mut bytes)?;
        if cpz3_checksum(&bytes) != layout.index_checksum_raw {
            return Err(FormatError("CPZ2 index checksum mismatch").into());
        }
        decrypt(&mut bytes, word(&header[16..]) ^ 0x77777777 ^ 0x37a9f45b);
        let (entries, keys) =
            flat::records(&bytes, layout.file_count, layout.data_offset, file_bytes)?;
        Ok(Self {
            layout,
            entries,
            entry_keys: keys.into_iter().map(|key| key ^ 0x796c3afd).collect(),
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        flat::find(&self.entries, name)
    }

    /// First entry whose name equals `name`, folding ASCII case on both sides.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        flat::lookup(&self.entries, name, false)
    }

    /// The original's extension-less search: first entry whose name starts with `stem`.
    pub fn lookup_prefix(&self, stem: &[u8]) -> Result<Option<usize>, FormatError> {
        flat::lookup(&self.entries, stem, true)
    }

    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        self.read_resource(reader, index, false)
    }

    /// Check the encrypted bytes against the record checksum before decrypting.
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
        let mut bytes = flat::read_raw(reader, &self.entries, index)?;
        if verify && cpz3_checksum(&bytes) != self.entries[index].checksum_raw {
            return Err(FormatError("CPZ2 resource checksum mismatch").into());
        }
        decrypt(&mut bytes, self.entry_keys[index]);
        Ok(bytes)
    }
}

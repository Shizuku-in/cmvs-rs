//! CVNS CPZ1 (2004): plain 16-byte header, flat index, byte cipher, no checksums.
use super::flat::{self, word};
use super::{ArchiveError, Entry};
use crate::FormatError;
use std::io::{Read, Seek, SeekFrom};

pub const CPZ1_HEADER_SIZE: usize = 16;

/// The 64-byte Shift-JIS phrase embedded in cvns.exe; the reader adds 10 to every
/// byte when it is constructed. CPZ2 uses a copy that differs at index 47.
const KEY: [u8; 64] = {
    let mut key = *b"\x88\xc3\x8d\x86\x82\xcd\x82\xcb\x81\x41\x89\xf0\x90\xcd\x82\xb5\x82\xbf\x82\xe1\x83\x5f\x83\x81\x82\xc8\x82\xcc\x81\x63\x82\xd9\x82\xf1\x82\xc6\x82\xbe\x82\xe6\x81\xf4\x82\xa0\x82\xea\x81\x41\x92\x4e\x82\xc9\x8c\xbe\x82\xc1\x82\xc4\x82\xe9\x82\xcc\x81\x48";
    let mut i = 0;
    while i < 64 {
        key[i] = key[i].wrapping_add(10);
        i += 1;
    }
    key
};

/// The key index restarts for every buffer (index or resource).
fn decrypt(bytes: &mut [u8]) {
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = (*byte ^ KEY[i & 63]).wrapping_sub(108);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz1Layout {
    pub file_count: u32,
    pub index_bytes: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

impl Cpz1Layout {
    /// Bounds-only inspection; CPZ1 has no checksums.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ1_HEADER_SIZE || &header[..4] != b"CPZ1" {
            return Err(FormatError("expected 16-byte CPZ1 header"));
        }
        let file_count = word(&header[4..]);
        let index_bytes = word(&header[8..]);
        let data_offset = flat::data_offset(CPZ1_HEADER_SIZE, file_count, index_bytes, file_bytes)?;
        Ok(Self {
            file_count,
            index_bytes,
            data_offset,
            data_bytes: file_bytes - data_offset,
        })
    }
}

#[derive(Debug)]
pub struct Cpz1Index {
    pub layout: Cpz1Layout,
    entries: Vec<Entry>,
}

impl Cpz1Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ1_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz1Layout::parse(&header, file_bytes)?;
        if layout.index_bytes as u64 > crate::MAX_PAYLOAD as u64 || layout.file_count > 1_000_000 {
            return Err(FormatError("CPZ1 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; layout.index_bytes as usize];
        reader.read_exact(&mut bytes)?;
        decrypt(&mut bytes);
        let (entries, _) =
            flat::records(&bytes, layout.file_count, layout.data_offset, file_bytes)?;
        Ok(Self { layout, entries })
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

    /// CPZ1 stores no checksum, so there is no separate verified read.
    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        let mut bytes = flat::read_raw(reader, &self.entries, index)?;
        decrypt(&mut bytes);
        Ok(bytes)
    }
}

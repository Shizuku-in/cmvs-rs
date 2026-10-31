//! Bounded CPZ6 layout, index and resource decoding. Header and index digests are not
//! verified; resource checksums only through `read_entry_verified`.
//! Header XOR fields follow xmoezzz/CMVS-Engine, pinned in docs/references.md.
use super::{ArchiveError, Entry, crypto, data_checksum};
use crate::FormatError;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub const CPZ6_HEADER_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz6Layout {
    pub directory_count: u32,
    pub directory_index_bytes: u32,
    pub file_index_bytes: u32,
    pub encryption_mode: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

/// Parsed index and derived cipher keys. Game resources remain in the caller's reader.
#[derive(Debug)]
pub struct Cpz6Index {
    pub layout: Cpz6Layout,
    entries: Vec<Entry>,
    entry_keys: Vec<(u32, u32)>,
    keys: crypto::Keys,
    file_bytes: u64,
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let end = offset
        .checked_add(4)
        .ok_or(FormatError("CPZ6 field offset overflow"))?;
    let b = bytes
        .get(offset..end)
        .ok_or(FormatError("truncated CPZ6 field"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn record(bytes: &[u8], offset: usize, minimum: usize) -> Result<&[u8], FormatError> {
    let len = read_u32(bytes, offset)? as usize;
    if len < minimum {
        return Err(FormatError("CPZ6 record is too short"));
    }
    let end = offset
        .checked_add(len)
        .ok_or(FormatError("CPZ6 record length overflow"))?;
    bytes
        .get(offset..end)
        .ok_or(FormatError("CPZ6 record exceeds its index block"))
}

fn name(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or(FormatError("unterminated CPZ6 filename"))?;
    if end > 4096 {
        return Err(FormatError("CPZ6 filename exceeds research limit"));
    }
    Ok(bytes[..end].to_vec())
}

struct Directory {
    name: Arc<[u8]>,
    count: u32,
    offset: usize,
    key: u32,
}

impl Cpz6Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ6_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz6Layout::parse(&header, file_bytes)?;
        if layout.encryption_mode != 1 {
            return Err(FormatError("unsupported CPZ6 encryption mode").into());
        }
        let index_len = layout.data_offset - CPZ6_HEADER_SIZE as u64;
        if index_len > crate::MAX_PAYLOAD as u64 {
            return Err(FormatError("CPZ6 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; index_len as usize];
        reader.read_exact(&mut bytes)?;
        let keys = crypto::Keys::from_header(&header);
        crypto::outer_index(&mut bytes, keys.key);
        let (dirs, files) = bytes.split_at_mut(layout.directory_index_bytes as usize);
        crypto::directory_index(dirs, &keys);
        let mut directories = Vec::new();
        let mut offset = 0;
        for _ in 0..layout.directory_count {
            let rec = record(dirs, offset, 17)?;
            directories.push(Directory {
                name: name(&rec[16..])?.into(),
                count: read_u32(rec, 4)?,
                offset: read_u32(rec, 8)? as usize,
                key: read_u32(rec, 12)?,
            });
            offset += rec.len();
        }
        if offset != dirs.len() {
            return Err(FormatError("CPZ6 directory count/length mismatch").into());
        }
        if directories.first().is_some_and(|d| d.offset != 0)
            || (directories.is_empty() && !files.is_empty())
        {
            return Err(FormatError("CPZ6 file index has an unassigned prefix").into());
        }
        let mut entries = Vec::new();
        let mut entry_keys = Vec::new();
        for (i, dir) in directories.iter().enumerate() {
            let end = directories.get(i + 1).map_or(files.len(), |d| d.offset);
            let block = files
                .get_mut(dir.offset..end)
                .ok_or(FormatError("invalid CPZ6 directory file-index offsets"))?;
            if u64::from(dir.count) * 25 > block.len() as u64
                || entries.len() as u64 + u64::from(dir.count) > 1_000_000
            {
                return Err(FormatError("CPZ6 file count exceeds index or research limit").into());
            }
            crypto::file_index(block, dir.key, &keys);
            let mut offset = 0;
            for _ in 0..dir.count {
                let rec = record(block, offset, 25)?;
                let name = name(&rec[24..])?;
                if name.is_empty() {
                    return Err(FormatError("empty CPZ6 filename").into());
                }
                let relative = u64::from(read_u32(rec, 4)?) | (u64::from(read_u32(rec, 8)?) << 32);
                let size = read_u32(rec, 12)?;
                let absolute = layout
                    .data_offset
                    .checked_add(relative)
                    .ok_or(FormatError("CPZ6 data offset overflow"))?;
                if absolute
                    .checked_add(u64::from(size))
                    .is_none_or(|end| end > file_bytes)
                {
                    return Err(FormatError("CPZ6 resource exceeds archive length").into());
                }
                entries.push(Entry {
                    directory: dir.name.clone(),
                    name,
                    size,
                    offset: absolute,
                    checksum_raw: read_u32(rec, 16)?,
                });
                entry_keys.push((dir.key, read_u32(rec, 20)?));
                offset += rec.len();
            }
            if offset != block.len() {
                return Err(FormatError("CPZ6 file count/length mismatch").into());
            }
        }
        Ok(Self {
            layout,
            entries,
            entry_keys,
            keys,
            file_bytes,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Basename lookup only; duplicate basenames require selecting an entry index.
    pub fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        let mut matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name == name);
        let first = matches
            .next()
            .ok_or(FormatError("CPZ6 filename not found"))?
            .0;
        if matches.next().is_some() {
            return Err(FormatError("ambiguous CPZ6 basename"));
        }
        Ok(first)
    }

    /// 0x140002090 with an empty directory: the first entry, in index order across all
    /// directories, whose stored name equals `lookup_key(name)` byte for byte.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        let key = lookup_key(name)?;
        Ok(self.entries.iter().position(|entry| entry.name == key))
    }

    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        let (_, mut bytes) = self.read_raw(reader, index)?;
        self.decrypt(index, &mut bytes);
        Ok(bytes)
    }

    /// PACK_DATA_CHECK path of 0x140002090: the stored checksum must match the encrypted
    /// bytes before decryption; the original shows a MessageBox and exits otherwise.
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        let (entry, mut bytes) = self.read_raw(reader, index)?;
        if data_checksum(&bytes) != entry.checksum_raw {
            return Err(FormatError("CPZ6 resource checksum mismatch").into());
        }
        self.decrypt(index, &mut bytes);
        Ok(bytes)
    }

    fn read_raw<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<(&Entry, Vec<u8>), ArchiveError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(FormatError("CPZ6 entry index out of bounds"))?;
        if entry.size as usize > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ6 resource exceeds research allocation limit").into());
        }
        if entry
            .offset
            .checked_add(u64::from(entry.size))
            .is_none_or(|end| end > self.file_bytes)
        {
            return Err(FormatError("CPZ6 resource exceeds archive length").into());
        }
        reader.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0; entry.size as usize];
        reader.read_exact(&mut bytes)?;
        Ok((entry, bytes))
    }

    fn decrypt(&self, index: usize, bytes: &mut [u8]) {
        let (directory_key, file_key) = self.entry_keys[index];
        let seed = self.keys.seed
            ^ (self.keys.key ^ directory_key.wrapping_add(file_key))
                .wrapping_add(self.layout.directory_count)
                .wrapping_add(0xa3d61785);
        crypto::resource(bytes, &self.keys, seed);
    }
}

/// Query folding of 0x140002090: ASCII A-Z become lowercase, Shift-JIS lead bytes
/// (0x81..0x9f, 0xe0..0xfc) skip their trail byte. The original copies into 64 bytes.
pub fn lookup_key(name: &[u8]) -> Result<Vec<u8>, FormatError> {
    if name.len() >= 64 || name.contains(&0) {
        return Err(FormatError(
            "CPZ6 lookup name exceeds the original 64-byte buffer",
        ));
    }
    let mut key = name.to_vec();
    let mut at = 0;
    while at < key.len() {
        if matches!(key[at], 0x81..=0x9f | 0xe0..=0xfc) {
            at += 2;
        } else {
            key[at] = key[at].to_ascii_lowercase();
            at += 1;
        }
    }
    Ok(key)
}

impl Cpz6Layout {
    /// `file_bytes` is the actual archive length, not the amount read into memory.
    /// Does not authenticate the header or validate the encrypted index contents.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ6_HEADER_SIZE || &header[..4] != b"CPZ6" {
            return Err(FormatError("expected 64-byte CPZ6 header"));
        }
        let field = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let directory_count = field(4) ^ 0xfe3a53da;
        let directory_index_bytes = field(8) ^ 0x37f298e8;
        let file_index_bytes = field(12) ^ 0x7a6f3a2d;
        let encryption_mode = field(52) ^ 0xfb73a956;
        let data_offset = CPZ6_HEADER_SIZE as u64
            + u64::from(directory_index_bytes)
            + u64::from(file_index_bytes);
        if data_offset > file_bytes {
            return Err(FormatError("CPZ6 indexes exceed archive length"));
        }
        // Each directory record has 16 fixed bytes and at least a NUL filename.
        if u64::from(directory_count) * 17 > u64::from(directory_index_bytes) {
            return Err(FormatError(
                "CPZ6 directory records cannot fit in declared index",
            ));
        }
        Ok(Self {
            directory_count,
            directory_index_bytes,
            file_index_bytes,
            encryption_mode,
            data_offset,
            data_bytes: file_bytes - data_offset,
        })
    }
}

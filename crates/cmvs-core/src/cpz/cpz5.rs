//! Bounded CPZ5 decoding with packing modes 5 through 9.
//! Header checksum and encrypted index MD5 are always verified.
use super::{ArchiveError, Entry, crypto, data_checksum};
use crate::FormatError;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub const CPZ5_HEADER_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Cpz5Mode {
    /// HAROUSE / ASUNANA.
    Mode5 = 5,
    /// MEMORIA.
    Mode6 = 6,
    /// NATSUUTA.
    Mode7 = 7,
    /// HASSAKU / MIRANOSU.
    Mode8 = 8,
    /// KAZOKUBU / HAPYMAHER / HAPYMAHERFD and their RE editions.
    #[default]
    Mode9 = 9,
}

impl TryFrom<u32> for Cpz5Mode {
    type Error = FormatError;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            5 => Ok(Self::Mode5),
            6 => Ok(Self::Mode6),
            7 => Ok(Self::Mode7),
            8 => Ok(Self::Mode8),
            9 => Ok(Self::Mode9),
            _ => Err(FormatError("unsupported CPZ5 packing mode")),
        }
    }
}

impl Cpz5Mode {
    fn table_bytes(self) -> &'static [u8; 96] {
        if (self as u32) < 8 {
            &crypto::TABLE_BYTES
        } else {
            &LATE_TABLE
        }
    }

    fn keys(self, header: &[u8]) -> crypto::Keys {
        let masks = [0x43de7c19, 0xcc65f415, 0xd016a93c, 0x97a3ba9a];
        let input = std::array::from_fn(|i| word(&header[32 + i * 4..]) ^ masks[i]);
        let initial = match self {
            Self::Mode5 => [0xc74a2b01, 0xe7c8ab8f, 0xd8bedc4e, 0x7302a4c5],
            Self::Mode6 => [0xa79463f9, 0xb6e755c5, 0xc696af21, 0x6983e978],
            Self::Mode7 => [0x63fe9a7c, 0xc2b93e98, 0xef91ba5c, 0x72c9a82e],
            Self::Mode8 => [0x64fe9a8c, 0xc2f93ea8, 0xef81ba5d, 0x92c8a72f],
            Self::Mode9 => [0x53fe9b2c, 0xf2c93ea8, 0xee81ba59, 0xa2c8973e],
        };
        let s = crypto::custom_digest(input, initial);
        let digest = match self {
            Self::Mode5 => [s[3], s[1], s[2], s[0]],
            Self::Mode6 => [s[1], s[2], s[3], s[0]],
            Self::Mode7 => [
                s[1].wrapping_add(0x45876329),
                s[2] ^ 0x54f36d6c,
                s[3].wrapping_add(0x4387a749),
                s[0] ^ 0xe3f9a742,
            ],
            Self::Mode8 => [
                s[1] ^ 0x49876325,
                s[2].wrapping_add(0x54f46d7c),
                s[3] ^ 0xac7958b7,
                s[0].wrapping_add(0x1c0638bd),
            ],
            Self::Mode9 => [
                s[1] ^ 0x49875325,
                s[2].wrapping_add(0x54f46d7d),
                s[3] ^ 0xad7948b7,
                s[0].wrapping_add(0x1d0638ad),
            ],
        };
        crypto::Keys {
            key: word(&header[48..]) ^ 0xae7d39bf,
            seed: 0,
            digest,
        }
    }

    fn table(self, mut key: u32, seed: u32) -> [u8; 256] {
        let multiplier = if self == Self::Mode9 {
            0x1a740235
        } else {
            0x1a743125
        };
        let mut table = std::array::from_fn(|i| i as u8);
        for _ in 0..256 {
            table.swap(((key >> 16) & 255) as usize, (key & 255) as usize);
            table.swap(((key >> 8) & 255) as usize, (key >> 24) as usize);
            key = key
                .rotate_right(2)
                .wrapping_mul(multiplier)
                .wrapping_add(seed);
        }
        table
    }

    fn outer_index(self, bytes: &mut [u8], key: u32) {
        let key = key ^ 0x3795b39a;
        let table: [u32; 24] =
            std::array::from_fn(|i| word(&self.table_bytes()[i * 4..]).wrapping_sub(key));
        let shift = ((key ^ key >> 8 ^ key >> 16 ^ key >> 24 ^ 0xfffffffb) & 15) + 7;
        let mut flag = 5;
        let (chunks, tail) = bytes.as_chunks_mut::<4>();
        for chunk in chunks {
            let value = (word(chunk) ^ table[flag])
                .wrapping_add(0x784c5962)
                .rotate_right(shift)
                .wrapping_add(0x01010101);
            chunk.copy_from_slice(&value.to_le_bytes());
            flag = (flag + 1) % 24;
        }
        let sub = if self == Self::Mode9 { 0x73 } else { 0x79 };
        let length = tail.len();
        for (i, byte) in tail.iter_mut().enumerate() {
            *byte = (*byte ^ (table[flag] >> (4 * (length - i))) as u8).wrapping_sub(sub);
            flag = (flag + 1) % 24;
        }
    }

    fn substitute(self, bytes: &mut [u8], keys: &crypto::Keys, seed: u32, xor: u8) {
        let table = self.table(keys.key, seed);
        for byte in bytes {
            *byte = table[(*byte ^ xor) as usize];
        }
    }

    fn directory_index(self, bytes: &mut [u8], keys: &crypto::Keys) {
        self.substitute(bytes, keys, keys.digest[1], 0x3a);
        let key = keys.directory_key();
        let mut seed = 0x76548aefu32;
        let mut flag = 0;
        let (chunks, tail) = bytes.as_chunks_mut::<4>();
        for chunk in chunks {
            let value = (word(chunk) ^ key[flag])
                .wrapping_sub(0x4a91c262)
                .rotate_left(3)
                .wrapping_sub(seed);
            chunk.copy_from_slice(&value.to_le_bytes());
            seed = seed.wrapping_add(0x10fb562a);
            flag = (flag + 1) & 3;
        }
        for byte in tail {
            *byte = (*byte ^ (key[flag] >> 6) as u8).wrapping_add(0x37);
            flag = (flag + 1) & 3;
        }
    }

    fn file_index(self, bytes: &mut [u8], directory: u32, keys: &crypto::Keys) {
        self.substitute(bytes, keys, keys.digest[2], 0x7e);
        let key = [
            directory ^ keys.digest[0],
            directory.wrapping_add(0x00112233) ^ keys.digest[1],
            directory ^ keys.digest[2],
            directory.wrapping_add(0x34258765) ^ keys.digest[3],
        ];
        let mut seed = 0x2a65cb4eu32;
        let mut flag = 0;
        let (chunks, tail) = bytes.as_chunks_mut::<4>();
        for chunk in chunks {
            let value = (word(chunk) ^ key[flag])
                .wrapping_sub(seed)
                .rotate_left(2)
                .wrapping_add(0x37a19e8b);
            chunk.copy_from_slice(&value.to_le_bytes());
            seed = seed.wrapping_sub(0x0139fa9b);
            flag = (flag + 1) & 3;
        }
        for byte in tail {
            *byte = (*byte ^ (key[flag] >> 4) as u8).wrapping_add(5);
            flag = (flag + 1) & 3;
        }
    }

    fn resource(self, bytes: &mut [u8], keys: &crypto::Keys, seed: u32) {
        let table = self.table(keys.digest[3], keys.key);
        let key_bytes: [u8; 96] = std::array::from_fn(|i| {
            (keys.digest[1] >> 2) as u8 ^ table[self.table_bytes()[i] as usize]
        });
        let decrypt: [u32; 24] = std::array::from_fn(|i| word(&key_bytes[i * 4..]) ^ seed);
        let mut key = if self == Self::Mode9 {
            0x2547b39eu32
        } else {
            0x2547a39e
        };
        let mut flag = 9;
        let (chunks, tail) = bytes.as_chunks_mut::<4>();
        for chunk in chunks {
            let value = ((decrypt[flag] >> 1) ^ decrypt[((key >> 6) & 15) as usize] ^ word(chunk))
                .wrapping_sub(seed)
                ^ keys.digest[(key & 3) as usize];
            chunk.copy_from_slice(&value.to_le_bytes());
            key = key.wrapping_add(seed).wrapping_add(value);
            flag = (flag + 1) & 15;
        }
        let xor = if self == Self::Mode9 { 0xcb } else { 0xbc };
        for byte in tail {
            *byte = table[(*byte ^ xor) as usize];
        }
    }
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

const LATE_TABLE: [u8; 96] = [
    0x89, 0xf0, 0x90, 0xcd, 0x82, 0xb7, 0x82, 0xe9, 0x88, 0xab, 0x82, 0xa2, 0x8e, 0x71, 0x82, 0xcd,
    0x83, 0x8a, 0x83, 0x52, 0x82, 0xaa, 0x82, 0xa8, 0x8e, 0x64, 0x92, 0x75, 0x82, 0xab, 0x82, 0xb5,
    0x82, 0xbf, 0x82, 0xe1, 0x82, 0xa2, 0x82, 0xdc, 0x82, 0xb7, 0x81, 0x42, 0x83, 0x81, 0x83, 0x62,
    0x81, 0x49, 0x82, 0xa9, 0x82, 0xed, 0x82, 0xa2, 0x82, 0xa2, 0x82, 0xc6, 0x82, 0xa9, 0x8c, 0xbe,
    0x82, 0xc1, 0x82, 0xc4, 0x82, 0xe0, 0x8b, 0x96, 0x82, 0xb5, 0x82, 0xc4, 0x82, 0xa0, 0x82, 0xb0,
    0x82, 0xc8, 0x82, 0xa2, 0x82, 0xf1, 0x82, 0xbe, 0x82, 0xa9, 0x82, 0xe7, 0x82, 0x9f, 0x81, 0x49,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz5Layout {
    pub directory_count: u32,
    pub directory_index_bytes: u32,
    pub file_index_bytes: u32,
    pub encryption_mode: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

/// Parsed index and derived cipher keys. Game resources remain in the caller's reader.
#[derive(Debug)]
pub struct Cpz5Index {
    pub layout: Cpz5Layout,
    pub mode: Cpz5Mode,
    entries: Vec<Entry>,
    entry_keys: Vec<(u32, u32)>,
    keys: crypto::Keys,
    file_bytes: u64,
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let end = offset
        .checked_add(4)
        .ok_or(FormatError("CPZ5 field offset overflow"))?;
    let b = bytes
        .get(offset..end)
        .ok_or(FormatError("truncated CPZ5 field"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn record(bytes: &[u8], offset: usize, minimum: usize) -> Result<&[u8], FormatError> {
    let len = read_u32(bytes, offset)? as usize;
    if len < minimum {
        return Err(FormatError("CPZ5 record is too short"));
    }
    let end = offset
        .checked_add(len)
        .ok_or(FormatError("CPZ5 record length overflow"))?;
    bytes
        .get(offset..end)
        .ok_or(FormatError("CPZ5 record exceeds its index block"))
}

fn name(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or(FormatError("unterminated CPZ5 filename"))?;
    if end > 4096 {
        return Err(FormatError("CPZ5 filename exceeds research limit"));
    }
    Ok(bytes[..end].to_vec())
}

struct Directory {
    name: Arc<[u8]>,
    count: u32,
    offset: usize,
    key: u32,
}

impl Cpz5Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        // The header does not carry the packing-mode setting. Without explicit
        // startup settings, require a unique fully bounded index interpretation.
        let mut found = None;
        let mut first_error = None;
        for mode in [
            Cpz5Mode::Mode9,
            Cpz5Mode::Mode8,
            Cpz5Mode::Mode7,
            Cpz5Mode::Mode6,
            Cpz5Mode::Mode5,
        ] {
            match Self::read_with_mode(reader, mode) {
                Ok(index) if index.entries.is_empty() => return Ok(index),
                Ok(index) => {
                    if found.is_some() {
                        return Err(FormatError(
                            "ambiguous CPZ5 packing mode; specify startup settings",
                        )
                        .into());
                    }
                    found = Some(index);
                }
                Err(error @ ArchiveError::Io(_)) => return Err(error),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        found.ok_or_else(|| first_error.expect("all packing modes returned format errors"))
    }

    pub fn read_with_mode<R: Read + Seek>(
        reader: &mut R,
        mode: Cpz5Mode,
    ) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ5_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz5Layout::parse(&header, file_bytes)?;
        if layout.encryption_mode > 1 {
            return Err(FormatError("unsupported CPZ5 encryption mode").into());
        }
        let index_len = layout.data_offset - CPZ5_HEADER_SIZE as u64;
        if index_len > crate::MAX_PAYLOAD as u64 {
            return Err(FormatError("CPZ5 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; index_len as usize];
        reader.read_exact(&mut bytes)?;
        if crypto::md5(&bytes) != header[16..32] {
            return Err(FormatError("CPZ5 encrypted index digest mismatch").into());
        }
        let keys = mode.keys(&header);
        mode.outer_index(&mut bytes, keys.key);
        let (dirs, files) = bytes.split_at_mut(layout.directory_index_bytes as usize);
        mode.directory_index(dirs, &keys);
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
            return Err(FormatError("CPZ5 directory count/length mismatch").into());
        }
        if directories.first().is_some_and(|d| d.offset != 0)
            || (directories.is_empty() && !files.is_empty())
        {
            return Err(FormatError("CPZ5 file index has an unassigned prefix").into());
        }
        let mut entries = Vec::new();
        let mut entry_keys = Vec::new();
        for (i, dir) in directories.iter().enumerate() {
            let end = directories.get(i + 1).map_or(files.len(), |d| d.offset);
            let block = files
                .get_mut(dir.offset..end)
                .ok_or(FormatError("invalid CPZ5 directory file-index offsets"))?;
            if u64::from(dir.count) * 25 > block.len() as u64
                || entries.len() as u64 + u64::from(dir.count) > 1_000_000
            {
                return Err(FormatError("CPZ5 file count exceeds index or research limit").into());
            }
            mode.file_index(block, dir.key, &keys);
            let mut offset = 0;
            for _ in 0..dir.count {
                let rec = record(block, offset, 25)?;
                let name = name(&rec[24..])?;
                if name.is_empty() {
                    return Err(FormatError("empty CPZ5 filename").into());
                }
                let relative = u64::from(read_u32(rec, 4)?);
                let size = read_u32(rec, 12)?;
                let absolute = layout
                    .data_offset
                    .checked_add(relative)
                    .ok_or(FormatError("CPZ5 data offset overflow"))?;
                if absolute
                    .checked_add(u64::from(size))
                    .is_none_or(|end| end > file_bytes)
                {
                    return Err(FormatError("CPZ5 resource exceeds archive length").into());
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
                return Err(FormatError("CPZ5 file count/length mismatch").into());
            }
        }
        Ok(Self {
            layout,
            mode,
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
            .ok_or(FormatError("CPZ5 filename not found"))?
            .0;
        if matches.next().is_some() {
            return Err(FormatError("ambiguous CPZ5 basename"));
        }
        Ok(first)
    }

    /// Fold the query and return the first matching basename in archive order.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        if name.len() >= 64 || name.contains(&0) {
            return Err(FormatError("invalid CPZ5 lookup name"));
        }
        let key = super::lookup_key(name)?;
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

    /// Verify the encrypted resource checksum before returning plaintext.
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        let (entry, mut bytes) = self.read_raw(reader, index)?;
        if data_checksum(&bytes) != entry.checksum_raw {
            return Err(FormatError("CPZ5 resource checksum mismatch").into());
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
            .ok_or(FormatError("CPZ5 entry index out of bounds"))?;
        if entry.size as usize > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ5 resource exceeds research allocation limit").into());
        }
        if entry
            .offset
            .checked_add(u64::from(entry.size))
            .is_none_or(|end| end > self.file_bytes)
        {
            return Err(FormatError("CPZ5 resource exceeds archive length").into());
        }
        reader.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0; entry.size as usize];
        reader.read_exact(&mut bytes)?;
        Ok((entry, bytes))
    }

    fn decrypt(&self, index: usize, bytes: &mut [u8]) {
        let (directory_key, file_key) = self.entry_keys[index];
        let seed = (self.keys.key ^ directory_key.wrapping_add(file_key))
            .wrapping_add(self.layout.directory_count)
            .wrapping_add(0xa3d61785);
        if self.layout.encryption_mode == 1 {
            self.mode.resource(bytes, &self.keys, seed);
        }
    }
}

impl Cpz5Layout {
    /// `file_bytes` is the actual archive length, not the amount read into memory.
    /// Checks the header checksum and layout bounds without reading the index.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ5_HEADER_SIZE || &header[..4] != b"CPZ5" {
            return Err(FormatError("expected 64-byte CPZ5 header"));
        }
        let field = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let checksum = data_checksum(&header[..60])
            .wrapping_sub(0x5a902b7c)
            .wrapping_sub(0x6dc5a9b4);
        if checksum != field(60) {
            return Err(FormatError("CPZ5 header checksum mismatch"));
        }
        let directory_count = field(4) ^ 0xfe3a53d9;
        let directory_index_bytes = field(8) ^ 0x37f298e7;
        let file_index_bytes = field(12) ^ 0x7a6f3a2c;
        let encryption_mode = field(52) ^ 0xfb73a955;
        let data_offset = CPZ5_HEADER_SIZE as u64
            + u64::from(directory_index_bytes)
            + u64::from(file_index_bytes);
        if data_offset > file_bytes {
            return Err(FormatError("CPZ5 indexes exceed archive length"));
        }
        // Each directory record has 16 fixed bytes and at least a NUL filename.
        if u64::from(directory_count) * 17 > u64::from(directory_index_bytes) {
            return Err(FormatError(
                "CPZ5 directory records cannot fit in declared index",
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

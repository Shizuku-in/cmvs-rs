//! Bounded CPZ7 layout, index and resource decoding.
//! Header checksum and encrypted index/key MD5 digests are always checked.
//! Resource checksums are checked through `read_entry_verified`.
//! Key derivation currently supports the AOITORI variant, not all CPZ7 games.
//!
//! Reference: Darkness-TX
use super::{ArchiveError, Entry, crypto, data_checksum};
use crate::FormatError;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub const CPZ7_HEADER_SIZE: usize = 72;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz7Layout {
    pub directory_count: u32,
    pub directory_index_bytes: u32,
    pub file_index_bytes: u32,
    pub encryption_mode: u32,
    pub index_key_bytes: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

/// Parsed index and derived cipher keys. Game resources remain in the caller's reader.
#[derive(Debug)]
pub struct Cpz7Index {
    pub layout: Cpz7Layout,
    entries: Vec<Entry>,
    entry_keys: Vec<(u32, u32)>,
    keys: crypto::Keys,
    file_bytes: u64,
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let end = offset
        .checked_add(4)
        .ok_or(FormatError("CPZ7 field offset overflow"))?;
    let b = bytes
        .get(offset..end)
        .ok_or(FormatError("truncated CPZ7 field"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn record(bytes: &[u8], offset: usize, minimum: usize) -> Result<&[u8], FormatError> {
    let len = read_u32(bytes, offset)? as usize;
    if len < minimum {
        return Err(FormatError("CPZ7 record is too short"));
    }
    let end = offset
        .checked_add(len)
        .ok_or(FormatError("CPZ7 record length overflow"))?;
    bytes
        .get(offset..end)
        .ok_or(FormatError("CPZ7 record exceeds its index block"))
}

fn name(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or(FormatError("unterminated CPZ7 filename"))?;
    if end > 4096 {
        return Err(FormatError("CPZ7 filename exceeds research limit"));
    }
    Ok(bytes[..end].to_vec())
}

struct Directory {
    name: Arc<[u8]>,
    count: u32,
    offset: usize,
    key: u32,
}

impl Cpz7Index {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        let file_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0; CPZ7_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        let layout = Cpz7Layout::parse(&header, file_bytes)?;
        if layout.encryption_mode > 1 {
            return Err(FormatError("unsupported CPZ7 encryption mode").into());
        }
        let index_len = layout.data_offset - CPZ7_HEADER_SIZE as u64;
        if index_len > crate::MAX_PAYLOAD as u64 {
            return Err(FormatError("CPZ7 index exceeds research allocation limit").into());
        }
        let mut bytes = vec![0; index_len as usize];
        reader.read_exact(&mut bytes)?;
        let index_bytes = layout.directory_index_bytes as usize + layout.file_index_bytes as usize;
        if crypto::md5(&bytes) != header[16..32] {
            return Err(FormatError("CPZ7 encrypted index digest mismatch").into());
        }
        let key_block = &bytes[index_bytes..];
        if crypto::md5(&key_block[16..]) != key_block[..16] {
            return Err(FormatError("CPZ7 index-key digest mismatch").into());
        }
        let index_key = unpack_index_key(key_block)?;
        bytes.truncate(index_bytes);
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte ^= index_key[(i + 3) % 0x3ff];
        }
        let keys = crypto::Keys::from_header_v7(&header);
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
            return Err(FormatError("CPZ7 directory count/length mismatch").into());
        }
        if directories.first().is_some_and(|d| d.offset != 0)
            || (directories.is_empty() && !files.is_empty())
        {
            return Err(FormatError("CPZ7 file index has an unassigned prefix").into());
        }
        let mut entries = Vec::new();
        let mut entry_keys = Vec::new();
        for (i, dir) in directories.iter().enumerate() {
            let end = directories.get(i + 1).map_or(files.len(), |d| d.offset);
            let block = files
                .get_mut(dir.offset..end)
                .ok_or(FormatError("invalid CPZ7 directory file-index offsets"))?;
            if u64::from(dir.count) * 29 > block.len() as u64
                || entries.len() as u64 + u64::from(dir.count) > 1_000_000
            {
                return Err(FormatError("CPZ7 file count exceeds index or research limit").into());
            }
            crypto::file_index(block, dir.key, &keys);
            let mut offset = 0;
            for _ in 0..dir.count {
                let rec = record(block, offset, 29)?;
                let name = name(&rec[28..])?;
                if name.is_empty() {
                    return Err(FormatError("empty CPZ7 filename").into());
                }
                let relative = u64::from(read_u32(rec, 4)?) | (u64::from(read_u32(rec, 8)?) << 32);
                let size = read_u32(rec, 12)?;
                if read_u32(rec, 16)? != 0 {
                    return Err(FormatError("CPZ7 resource exceeds supported size").into());
                }
                let absolute = layout
                    .data_offset
                    .checked_add(relative)
                    .ok_or(FormatError("CPZ7 data offset overflow"))?;
                if absolute
                    .checked_add(u64::from(size))
                    .is_none_or(|end| end > file_bytes)
                {
                    return Err(FormatError("CPZ7 resource exceeds archive length").into());
                }
                entries.push(Entry {
                    directory: dir.name.clone(),
                    name,
                    size,
                    offset: absolute,
                    checksum_raw: read_u32(rec, 20)?,
                });
                entry_keys.push((dir.key, read_u32(rec, 24)?));
                offset += rec.len();
            }
            if offset != block.len() {
                return Err(FormatError("CPZ7 file count/length mismatch").into());
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
            .ok_or(FormatError("CPZ7 filename not found"))?
            .0;
        if matches.next().is_some() {
            return Err(FormatError("ambiguous CPZ7 basename"));
        }
        Ok(first)
    }

    /// First entry in archive order matching ASCII-folded query bytes.
    /// Unlike CPZ6, this reader does not impose a 64-byte engine buffer limit.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        if name.len() > 4096 || name.contains(&0) {
            return Err(FormatError("invalid CPZ7 lookup name"));
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

    /// The stored checksum covers encrypted bytes, as confirmed against AOITORI assets.
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        index: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        let (entry, mut bytes) = self.read_raw(reader, index)?;
        if data_checksum(&bytes) != entry.checksum_raw {
            return Err(FormatError("CPZ7 resource checksum mismatch").into());
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
            .ok_or(FormatError("CPZ7 entry index out of bounds"))?;
        if entry.size as usize > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ7 resource exceeds research allocation limit").into());
        }
        if entry
            .offset
            .checked_add(u64::from(entry.size))
            .is_none_or(|end| end > self.file_bytes)
        {
            return Err(FormatError("CPZ7 resource exceeds archive length").into());
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
                .wrapping_add(0xa3c61785);
        if self.layout.encryption_mode == 1 {
            crypto::resource(bytes, &self.keys, seed);
        }
    }
}

impl Cpz7Layout {
    /// `file_bytes` is the actual archive length, not the amount read into memory.
    /// Checks the header checksum and layout bounds, without reading the index.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ7_HEADER_SIZE || &header[..4] != b"CPZ7" {
            return Err(FormatError("expected 72-byte CPZ7 header"));
        }
        let field = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let initial = field(64).wrapping_sub(0x6dc5a9b4);
        let checksum = data_checksum(&header[..64])
            .wrapping_sub(0x5a902b7c)
            .wrapping_add(initial);
        if checksum != field(68) {
            return Err(FormatError("CPZ7 header checksum mismatch"));
        }
        let index_key_bytes = field(64) ^ 0x65ef99f3;
        if index_key_bytes < 24 {
            return Err(FormatError("CPZ7 index-key block is too short"));
        }
        let directory_count = field(4) ^ 0xfe3a53da;
        let directory_index_bytes = field(8) ^ 0x37f298e8;
        let file_index_bytes = field(12) ^ 0x7a6f3a2d;
        let encryption_mode = field(52) ^ 0xfb73a956;
        let data_offset = CPZ7_HEADER_SIZE as u64
            + u64::from(directory_index_bytes)
            + u64::from(file_index_bytes)
            + u64::from(index_key_bytes);
        if data_offset > file_bytes {
            return Err(FormatError("CPZ7 indexes exceed archive length"));
        }
        // Each directory record has 16 fixed bytes and at least a NUL filename.
        if u64::from(directory_count) * 17 > u64::from(directory_index_bytes) {
            return Err(FormatError(
                "CPZ7 directory records cannot fit in declared index",
            ));
        }
        Ok(Self {
            directory_count,
            directory_index_bytes,
            file_index_bytes,
            encryption_mode,
            index_key_bytes,
            data_offset,
            data_bytes: file_bytes - data_offset,
        })
    }
}

// Actual reference streams decode to 1024 bytes; this is a research allocation bound.
const MAX_KEY_BYTES: usize = 64 * 1024;

struct HuffmanBits<'a> {
    bytes: &'a [u8],
    bit: usize,
    xor: [u8; 4],
}

impl HuffmanBits<'_> {
    fn get(&mut self, count: usize) -> Result<u16, FormatError> {
        let mut value = 0;
        for _ in 0..count {
            let at = self.bit / 8;
            let byte = self
                .bytes
                .get(at)
                .ok_or(FormatError("truncated CPZ7 Huffman stream"))?;
            value = (value << 1) | u16::from(((byte ^ self.xor[at & 3]) >> (self.bit & 7)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
}

fn huffman_tree(bits: &mut HuffmanBits<'_>, nodes: &mut Vec<[u16; 2]>) -> Result<u16, FormatError> {
    if bits.get(1)? == 0 {
        return bits.get(8);
    }
    if nodes.len() >= 255 {
        return Err(FormatError("CPZ7 Huffman tree exceeds 255 internal nodes"));
    }
    let at = nodes.len();
    nodes.push([0; 2]);
    let left = huffman_tree(bits, nodes)?;
    let right = huffman_tree(bits, nodes)?;
    nodes[at] = [left, right];
    Ok(at as u16 + 256)
}

fn unpack_index_key(block: &[u8]) -> Result<Vec<u8>, FormatError> {
    if block.len() < 24 {
        return Err(FormatError("truncated CPZ7 key block"));
    }
    let length = u32::from_le_bytes(block[16..20].try_into().unwrap()) as usize;
    if !(0x3ff..=MAX_KEY_BYTES).contains(&length) {
        return Err(FormatError(
            "CPZ7 unpacked key length outside research bounds",
        ));
    }
    let mut bits = HuffmanBits {
        bytes: &block[24..],
        bit: 0,
        xor: block[20..24].try_into().unwrap(),
    };
    let mut nodes = Vec::new();
    let root = huffman_tree(&mut bits, &mut nodes)?;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let mut symbol = root;
        while symbol >= 256 {
            symbol = nodes[usize::from(symbol - 256)][usize::from(bits.get(1)?)];
        }
        output.push(symbol as u8);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_PAYLOAD;
    use std::io::Cursor;

    fn fixture() -> Vec<u8> {
        include_str!("../../../../tests/fixtures/cpz7-0.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn header_checksum(bytes: &mut [u8]) {
        let initial = read_u32(bytes, 64).unwrap().wrapping_sub(0x6dc5a9b4);
        let sum = data_checksum(&bytes[..64])
            .wrapping_sub(0x5a902b7c)
            .wrapping_add(initial);
        bytes[68..72].copy_from_slice(&sum.to_le_bytes());
    }

    fn index_digest(bytes: &mut [u8], end: usize) {
        let hash = crypto::md5(&bytes[72..end]);
        bytes[16..32].copy_from_slice(&hash);
        header_checksum(bytes);
    }

    #[test]
    fn authenticated_key_block_damage_and_bad_modes_are_explicit_errors() {
        let original = fixture();
        let layout = Cpz7Layout::parse(&original[..72], original.len() as u64).unwrap();
        let key_at = 72 + layout.directory_index_bytes as usize + layout.file_index_bytes as usize;
        let end = layout.data_offset as usize;
        let mut bytes = original.clone();
        bytes[key_at + 16] ^= 1;
        index_digest(&mut bytes, end);
        assert!(
            Cpz7Index::read(&mut Cursor::new(bytes))
                .unwrap_err()
                .to_string()
                .contains("index-key digest mismatch")
        );

        let mut bytes = original.clone();
        bytes[key_at + 16..key_at + 20].copy_from_slice(&1022u32.to_le_bytes());
        let hash = crypto::md5(&bytes[key_at + 16..end]);
        bytes[key_at..key_at + 16].copy_from_slice(&hash);
        index_digest(&mut bytes, end);
        assert!(
            Cpz7Index::read(&mut Cursor::new(bytes))
                .unwrap_err()
                .to_string()
                .contains("unpacked key length")
        );

        let mut bytes = original;
        bytes[52..56].copy_from_slice(&(2u32 ^ 0xfb73a956).to_le_bytes());
        header_checksum(&mut bytes);
        assert!(
            Cpz7Index::read(&mut Cursor::new(bytes))
                .unwrap_err()
                .to_string()
                .contains("encryption mode")
        );
    }

    #[test]
    fn allocation_and_directory_bounds_fail_before_index_allocation() {
        let mut bytes = fixture();
        bytes[8..12].copy_from_slice(&((MAX_PAYLOAD as u32 + 1) ^ 0x37f298e8).to_le_bytes());
        header_checksum(&mut bytes);
        let length = MAX_PAYLOAD as u64 + bytes.len() as u64 + 100;
        let layout = Cpz7Layout::parse(&bytes[..72], length).unwrap();
        // Sparse cursor reports an artificial archive size without allocating it.
        struct SizedReader {
            inner: Cursor<Vec<u8>>,
            length: u64,
        }
        impl Read for SizedReader {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.inner.read(out)
            }
        }
        impl Seek for SizedReader {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                if pos == SeekFrom::End(0) {
                    Ok(self.length)
                } else {
                    self.inner.seek(pos)
                }
            }
        }
        let error = Cpz7Index::read(&mut SizedReader {
            inner: Cursor::new(bytes.clone()),
            length,
        })
        .unwrap_err();
        assert!(error.to_string().contains("allocation limit"));
        assert!(layout.data_offset > MAX_PAYLOAD as u64);
        bytes[4..8].copy_from_slice(&(u32::MAX ^ 0xfe3a53da).to_le_bytes());
        header_checksum(&mut bytes);
        assert!(Cpz7Layout::parse(&bytes[..72], length).is_err());
    }

    fn huffman_block(bits: &[u8], length: u32) -> Vec<u8> {
        let mut block = vec![0; 16];
        block.extend(length.to_le_bytes());
        block.extend([0; 4]);
        block.extend(bits);
        block
    }

    #[test]
    fn constant_leaf_and_truncated_trees_obey_declared_bounds() {
        // Tree: leaf 0xa5; the nine bits are read LSB-first from 0x4a, 0x01.
        assert_eq!(
            unpack_index_key(&huffman_block(&[0x4a, 1], 1023)).unwrap(),
            vec![0xa5; 1023]
        );
        for length in [0, 1022, MAX_KEY_BYTES as u32 + 1] {
            assert!(unpack_index_key(&huffman_block(&[0x4a, 1], length)).is_err());
        }
        assert!(unpack_index_key(&huffman_block(&[0x4a], 1023)).is_err());
        assert!(
            unpack_index_key(&huffman_block(&[0xff; 40], 1023))
                .unwrap_err()
                .to_string()
                .contains("255 internal nodes")
        );
        // Branch with two leaves, then too few bits to decode the requested key.
        assert!(unpack_index_key(&huffman_block(&[1, 0, 0], 1023)).is_err());
    }
}

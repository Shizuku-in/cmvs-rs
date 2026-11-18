//! The flat variable-length file record CPZ1 and CPZ2 share (CPZ3 inherited it):
//! length, size, offset, offset high, checksum, key, NUL-terminated name.
use super::{Entry, LookupTable, fold_into, fold_vec};
use crate::FormatError;

pub(super) fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("bounded CPZ field"))
}

/// Bounds shared by both headers once the count and index size are unmasked.
pub(super) fn data_offset(
    header_size: usize,
    file_count: u32,
    index_bytes: u32,
    file_bytes: u64,
) -> Result<u64, FormatError> {
    let data_offset = header_size as u64 + u64::from(index_bytes);
    if data_offset > file_bytes {
        return Err(FormatError("CPZ index exceeds archive length"));
    }
    if u64::from(file_count) * 25 > u64::from(index_bytes) {
        return Err(FormatError("CPZ file records cannot fit in declared index"));
    }
    Ok(data_offset)
}

/// Parse `file_count` records from a decrypted index. Returns the entries and the
/// raw key word of each record. Real indexes carry padding after the last record;
/// like the original, parsing stops at `file_count`.
pub(super) fn records(
    bytes: &[u8],
    file_count: u32,
    data_offset: u64,
    file_bytes: u64,
) -> Result<(Vec<Entry>, Vec<u32>), FormatError> {
    let mut entries = Vec::new();
    let mut keys = Vec::new();
    let directory: std::sync::Arc<[u8]> = [].into();
    let mut offset = 0usize;
    for _ in 0..file_count {
        let remaining = &bytes[offset..];
        if remaining.len() < 25 {
            return Err(FormatError("truncated CPZ file record"));
        }
        let length = word(remaining) as usize;
        let rec = remaining
            .get(..length)
            .filter(|b| b.len() >= 25)
            .ok_or(FormatError("CPZ record exceeds index or is too short"))?;
        let name_bytes = &rec[24..];
        let end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or(FormatError("unterminated CPZ filename"))?;
        if end == 0 {
            return Err(FormatError("empty CPZ filename"));
        }
        let size = word(&rec[4..]);
        // The x86 reader uses only the low offset word at +8.
        let absolute = data_offset + u64::from(word(&rec[8..]));
        if absolute + u64::from(size) > file_bytes {
            return Err(FormatError("CPZ resource exceeds archive length"));
        }
        entries.push(Entry {
            directory: directory.clone(),
            name: name_bytes[..end].to_vec(),
            size,
            offset: absolute,
            checksum_raw: word(&rec[16..]),
        });
        keys.push(word(&rec[20..]));
        offset += length;
    }
    Ok((entries, keys))
}

/// Precomputed lookup tables for flat CPZ1/CPZ2 indexes, supporting $O(1)$ exact
/// `find` and case-folded `lookup`, plus allocation-free stored-name `lookup_prefix`.
#[derive(Debug, Default)]
pub(super) struct FlatLookup {
    table: LookupTable,
    folded_names: Vec<Box<[u8]>>,
}

impl FlatLookup {
    pub(super) fn from_entries(entries: &[Entry]) -> Self {
        let folded_names = entries
            .iter()
            .map(|e| fold_vec(&e.name).into_boxed_slice())
            .collect();
        Self {
            table: LookupTable::from_entries(entries, fold_vec),
            folded_names,
        }
    }

    pub(super) fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        self.table
            .find(name, "CPZ filename not found", "ambiguous CPZ basename")
    }

    /// First entry whose name equals `name`, folding ASCII case on both sides.
    pub(super) fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        if name.contains(&0) {
            return Err(FormatError("invalid CPZ lookup name"));
        }
        Ok(self.table.lookup(name))
    }

    /// First entry whose name starts with `stem`, folding ASCII case on both sides.
    pub(super) fn lookup_prefix(&self, stem: &[u8]) -> Result<Option<usize>, FormatError> {
        if stem.contains(&0) {
            return Err(FormatError("invalid CPZ lookup name"));
        }
        if stem.len() <= 64 {
            let mut buf = [0u8; 64];
            let key = &mut buf[..stem.len()];
            fold_into(stem, key);
            Ok(self
                .folded_names
                .iter()
                .position(|stored| stored.starts_with(key)))
        } else {
            let key = fold_vec(stem);
            Ok(self
                .folded_names
                .iter()
                .position(|stored| stored.starts_with(&key)))
        }
    }
}

pub(super) fn read_raw<R: std::io::Read + std::io::Seek>(
    reader: &mut R,
    entries: &[Entry],
    index: usize,
) -> Result<Vec<u8>, super::ArchiveError> {
    let entry = entries
        .get(index)
        .ok_or(FormatError("CPZ entry index out of bounds"))?;
    if entry.size as u64 > crate::MAX_PAYLOAD as u64 {
        return Err(FormatError("CPZ resource exceeds research allocation limit").into());
    }
    reader.seek(std::io::SeekFrom::Start(entry.offset))?;
    let mut bytes = vec![0; entry.size as usize];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

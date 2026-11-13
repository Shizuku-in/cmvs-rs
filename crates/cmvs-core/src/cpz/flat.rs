//! The flat variable-length file record CPZ1 and CPZ2 share (CPZ3 inherited it):
//! length, size, offset, offset high, checksum, key, NUL-terminated name.
use super::Entry;
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

/// The original comparison: ASCII letters fold, Shift-JIS pairs compare exactly.
fn fold(name: &[u8]) -> Vec<u8> {
    let mut key = name.to_vec();
    let mut at = 0;
    while at < key.len() {
        if matches!(key[at], 0x81..=0x9f | 0xe0..=0xfc) {
            at += 2;
        } else {
            key[at] = key[at].to_ascii_uppercase();
            at += 1;
        }
    }
    key
}

pub(super) fn find(entries: &[Entry], name: &[u8]) -> Result<usize, FormatError> {
    let mut matches = entries.iter().enumerate().filter(|(_, e)| e.name == name);
    let first = matches
        .next()
        .ok_or(FormatError("CPZ filename not found"))?
        .0;
    if matches.next().is_some() {
        return Err(FormatError("ambiguous CPZ basename"));
    }
    Ok(first)
}

/// First entry whose name equals `name` (or, with `prefix`, starts with it),
/// folding ASCII case on both sides. Stored names keep their mixed case.
pub(super) fn lookup(
    entries: &[Entry],
    name: &[u8],
    prefix: bool,
) -> Result<Option<usize>, FormatError> {
    if name.contains(&0) {
        return Err(FormatError("invalid CPZ lookup name"));
    }
    let key = fold(name);
    Ok(entries.iter().position(|e| {
        let stored = fold(&e.name);
        if prefix {
            stored.starts_with(&key)
        } else {
            stored == key
        }
    }))
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

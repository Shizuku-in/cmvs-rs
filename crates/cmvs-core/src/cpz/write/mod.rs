//! Common replacement writing. Layout-specific ciphers stay behind ArchiveIndex.
mod cipher;
mod create;
use super::*;
use cipher::Cipher;

/// A file's raw name and plaintext content for a new directory-based archive.
#[derive(Debug, Clone)]
pub struct ArchiveFile {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

/// A directory in archive order. An empty name represents the root directory.
#[derive(Debug, Clone, Default)]
pub struct ArchiveDirectory {
    pub name: Vec<u8>,
    pub files: Vec<ArchiveFile>,
}

pub(super) fn put(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn finish_header(header: &mut [u8], index: &[u8]) {
    header[16..32].copy_from_slice(&crypto::md5(index));
    let (at, checksum) = if header.len() == CPZ7_HEADER_SIZE {
        (
            68,
            data_checksum(&header[..64])
                .wrapping_sub(0x5a902b7c)
                .wrapping_add(word(header, 64).wrapping_sub(0x6dc5a9b4)),
        )
    } else {
        // CPZ6 uses the same sum as CPZ5, confirmed on 25 original archives.
        (
            60,
            data_checksum(&header[..60])
                .wrapping_sub(0x5a902b7c)
                .wrapping_sub(0x6dc5a9b4),
        )
    };
    put(header, at, checksum);
}
pub(super) fn empty_output<W: Write + Seek>(output: &mut W) -> Result<(), ArchiveError> {
    if output.seek(SeekFrom::End(0))? != 0 {
        return Err(FormatError("CPZ output stream must be empty").into());
    }
    Ok(())
}
struct Written {
    offset: u64,
    size: u32,
    checksum: u32,
}
fn payload<R: Read + Seek, W: Write + Seek>(
    source: &mut R,
    output: &mut W,
    entry: &Entry,
    replacement: Option<&Vec<u8>>,
    checksum: fn(&[u8]) -> u32,
    encrypt: impl FnOnce(&mut [u8]),
) -> Result<Written, ArchiveError> {
    if entry.size as usize > crate::MAX_PAYLOAD {
        return Err(FormatError("CPZ resource exceeds allocation limit").into());
    }
    source.seek(SeekFrom::Start(entry.offset))?;
    let mut bytes = vec![0; entry.size as usize];
    source.read_exact(&mut bytes)?;
    if checksum(&bytes) != entry.checksum_raw {
        return Err(FormatError("CPZ resource checksum mismatch").into());
    }
    if let Some(replacement) = replacement {
        bytes.clone_from(replacement);
        encrypt(&mut bytes);
    }
    let result = Written {
        offset: output.stream_position()?,
        size: bytes.len() as u32,
        checksum: checksum(&bytes),
    };
    output.write_all(&bytes)?;
    Ok(result)
}
pub(super) fn rewrite<R: Read + Seek, W: Write + Seek>(
    index: &ArchiveIndex,
    source: &mut R,
    output: &mut W,
    replacements: &BTreeMap<usize, Vec<u8>>,
) -> Result<(), ArchiveError> {
    if replacements
        .iter()
        .any(|(i, bytes)| *i >= index.entries().len() || bytes.len() > crate::MAX_PAYLOAD)
    {
        return Err(FormatError("CPZ replacement index or size exceeds bounds").into());
    }
    // Revalidate the supplied source with the selected mode/per-archive parameters.
    let fresh = match index {
        ArchiveIndex::Cpz5(index) => ArchiveIndex::read_with_cpz5_mode(source, index.mode)?,
        ArchiveIndex::Cpz7(index) => {
            ArchiveIndex::read_with_cpz7_parameters(source, index.parameters)?
        }
        _ => ArchiveIndex::read(source)?,
    };
    if fresh.version() != index.version() || fresh.entries() != index.entries() {
        return Err(FormatError("CPZ source does not match the parsed index").into());
    }
    empty_output(output)?;
    if let ArchiveIndex::Cpz3(index) = &fresh {
        return rewrite_flat(index, source, output, replacements);
    }
    let (header_size, directory_count, directory_bytes, file_bytes, encrypted) = match &fresh {
        ArchiveIndex::Cpz5(index) => (
            CPZ5_HEADER_SIZE,
            index.layout.directory_count,
            index.layout.directory_index_bytes,
            index.layout.file_index_bytes,
            index.layout.encryption_mode == 1,
        ),
        ArchiveIndex::Cpz6(index) => (
            CPZ6_HEADER_SIZE,
            index.layout.directory_count,
            index.layout.directory_index_bytes,
            index.layout.file_index_bytes,
            true,
        ),
        ArchiveIndex::Cpz7(index) => (
            CPZ7_HEADER_SIZE,
            index.layout.directory_count,
            index.layout.directory_index_bytes,
            index.layout.file_index_bytes,
            index.layout.encryption_mode == 1,
        ),
        ArchiveIndex::Cpz3(_) => unreachable!(),
    };
    let data_offset = match &fresh {
        ArchiveIndex::Cpz5(index) => index.layout.data_offset,
        ArchiveIndex::Cpz6(index) => index.layout.data_offset,
        ArchiveIndex::Cpz7(index) => index.layout.data_offset,
        ArchiveIndex::Cpz3(_) => unreachable!(),
    };
    source.rewind()?;
    let mut header = vec![0; header_size];
    source.read_exact(&mut header)?;
    let mut bytes = vec![0; data_offset as usize - header_size];
    source.read_exact(&mut bytes)?;
    let index_bytes = directory_bytes as usize + file_bytes as usize;
    let key = if matches!(fresh, ArchiveIndex::Cpz7(_)) {
        let key = cpz7::unpack_index_key(&bytes[index_bytes..])?;
        for (i, byte) in bytes[..index_bytes].iter_mut().enumerate() {
            *byte ^= key[(i + 3) % 0x3ff];
        }
        Some(key)
    } else {
        None
    };
    let cipher = Cipher::new(&fresh, &header);
    cipher.decode_outer(&mut bytes[..index_bytes]);
    let (directories, _) = bytes.split_at_mut(directory_bytes as usize);
    cipher.decode_directories(directories);
    let mut dirs = Vec::new();
    let mut at = 0;
    for _ in 0..directory_count {
        dirs.push((
            word(directories, at + 4) as usize,
            word(directories, at + 8) as usize,
            word(directories, at + 12),
        ));
        at += word(directories, at) as usize;
    }
    // Reserve the original index extent. Its final ciphertext/digest is written last.
    output.write_all(&header)?;
    output.write_all(&bytes)?;
    // Restore the borrows after reserving bytes in the output.
    let (directories, remaining) = bytes.split_at_mut(directory_bytes as usize);
    let files = &mut remaining[..file_bytes as usize];
    let cpz7 = matches!(fresh, ArchiveIndex::Cpz7(_));
    let cpz5 = matches!(fresh, ArchiveIndex::Cpz5(_));
    let mut entry = 0;
    for (d, &(count, at, directory_key)) in dirs.iter().enumerate() {
        let end = dirs.get(d + 1).map_or(files.len(), |dir| dir.1);
        let block = &mut files[at..end];
        cipher.decode_files(block, directory_key);
        let mut at = 0;
        for _ in 0..count {
            let length = word(block, at) as usize;
            let rec = &mut block[at..at + length];
            let file_key = word(rec, if cpz7 { 24 } else { 20 });
            let written = payload(
                source,
                output,
                &fresh.entries()[entry],
                replacements.get(&entry),
                data_checksum,
                |bytes| {
                    if encrypted {
                        cipher.encode_resource(bytes, directory_key, file_key, directory_count);
                    }
                },
            )?;
            let offset = written.offset - data_offset;
            if cpz5 && offset > u64::from(u32::MAX) {
                return Err(FormatError("CPZ5 data offset exceeds 32 bits").into());
            }
            put(rec, 4, offset as u32);
            if !cpz5 {
                put(rec, 8, (offset >> 32) as u32);
            }
            put(rec, 12, written.size);
            put(rec, if cpz7 { 20 } else { 16 }, written.checksum);
            at += length;
            entry += 1;
        }
        cipher.encode_files(block, directory_key);
    }
    cipher.encode_directories(directories);
    cipher.encode_outer(&mut bytes[..index_bytes]);
    if let Some(key) = key {
        for (i, byte) in bytes[..index_bytes].iter_mut().enumerate() {
            *byte ^= key[(i + 3) % 0x3ff];
        }
    }
    finish_header(&mut header, &bytes);
    output.rewind()?;
    output.write_all(&header)?;
    output.write_all(&bytes)?;
    Ok(())
}
fn rewrite_flat<R: Read + Seek, W: Write + Seek>(
    index: &Cpz3Index,
    source: &mut R,
    output: &mut W,
    replacements: &BTreeMap<usize, Vec<u8>>,
) -> Result<(), ArchiveError> {
    source.rewind()?;
    let mut header = [0; CPZ3_HEADER_SIZE];
    source.read_exact(&mut header)?;
    let mut bytes = vec![0; index.layout.index_bytes as usize];
    source.read_exact(&mut bytes)?;
    let key = word(&header, 16) ^ 0xdddddddd ^ 0x7bf4a539;
    output.write_all(&header)?;
    output.write_all(&bytes)?;
    cpz3::crypt(&mut bytes, key, false);
    let mut at = 0;
    for (i, entry) in index.entries().iter().enumerate() {
        let length = word(&bytes, at) as usize;
        let rec = &mut bytes[at..at + length];
        let entry_key = word(rec, 20) ^ 0xc7f5da63;
        let written = payload(
            source,
            output,
            entry,
            replacements.get(&i),
            cpz3_checksum,
            |bytes| cpz3::crypt(bytes, entry_key, true),
        )?;
        let offset = u32::try_from(written.offset - index.layout.data_offset)
            .map_err(|_| FormatError("CPZ3 data offset exceeds 32 bits"))?;
        put(rec, 4, written.size);
        put(rec, 8, offset);
        put(rec, 16, written.checksum);
        at += length;
    }
    cpz3::crypt(&mut bytes, key, true);
    put(&mut header, 12, cpz3_checksum(&bytes));
    output.rewind()?;
    output.write_all(&header)?;
    output.write_all(&bytes)?;
    Ok(())
}

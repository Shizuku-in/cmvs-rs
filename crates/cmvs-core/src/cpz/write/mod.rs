//! Common replacement writing. Layout-specific ciphers stay behind ArchiveIndex.
mod cipher;
mod create;
pub use create::{DirectoryFormat, DirectoryNames};
mod flat;
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
/// Fixed-size reads keep checksum words and cipher feedback aligned even on short-reading sources.
fn stream(
    input: &mut impl Read,
    output: &mut impl Write,
    size: u32,
    mut cipher: Option<cipher::Resource>,
    encode: bool,
) -> Result<u32, ArchiveError> {
    let mut buffer = vec![0; (size as usize).min(64 * 1024)];
    let mut left = size as usize;
    let initial = data_checksum(&[]);
    let mut sum = initial;
    while left != 0 {
        let n = left.min(buffer.len());
        let chunk = &mut buffer[..n];
        input.read_exact(chunk)?;
        if !encode {
            sum = sum.wrapping_add(data_checksum(chunk).wrapping_sub(initial));
        }
        if let Some(c) = cipher.as_mut() {
            c.transform(chunk, encode);
        }
        if encode {
            sum = sum.wrapping_add(data_checksum(chunk).wrapping_sub(initial));
        }
        output.write_all(chunk)?;
        left -= n;
    }
    Ok(sum)
}
fn write_resource<R: Read>(
    size: u32,
    mut input: R,
    out: &mut impl Write,
    cipher: Option<cipher::Resource>,
) -> Result<u32, ArchiveError> {
    let checksum = stream(&mut input, out, size, cipher, true)?;
    if input.read(&mut [0])? != 0 {
        return Err(FormatError("resource grew while packing").into());
    }
    Ok(checksum)
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
    index.write_replacement_streams(source, output, |i| {
        Ok(replacements
            .get(&i)
            .map(|data| (data.len() as u32, data.as_slice())))
    })
}

impl ArchiveIndex {
    pub fn write_replacement_streams<R: Read>(
        &self,
        source: &mut (impl Read + Seek),
        output: &mut (impl Write + Seek),
        mut open: impl FnMut(usize) -> Result<Option<(u32, R)>, ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let index = self;
        if index.version().is_flat() {
            return index.write_flat_replacements(source, output, open);
        }
        // Revalidate the supplied source with the selected mode/per-archive parameters.
        let fresh = index.read_with_same_settings(source)?;
        if fresh.entries() != index.entries() {
            return Err(FormatError("CPZ source does not match the parsed index").into());
        }
        empty_output(output)?;
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
            ArchiveIndex::Cpz1(_) | ArchiveIndex::Cpz2(_) | ArchiveIndex::Cpz3(_) => unreachable!(),
        };
        let data_offset = match &fresh {
            ArchiveIndex::Cpz5(index) => index.layout.data_offset,
            ArchiveIndex::Cpz6(index) => index.layout.data_offset,
            ArchiveIndex::Cpz7(index) => index.layout.data_offset,
            ArchiveIndex::Cpz1(_) | ArchiveIndex::Cpz2(_) | ArchiveIndex::Cpz3(_) => unreachable!(),
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
                let before = &fresh.entries()[entry];
                source.seek(SeekFrom::Start(before.offset))?;
                let at_output = output.stream_position()?;
                let written = if let Some((size, plain)) = open(entry)? {
                    let original = stream(source, &mut std::io::sink(), before.size, None, false)?;
                    if original != before.checksum_raw {
                        return Err(FormatError("CPZ resource checksum mismatch").into());
                    }
                    let transform = encrypted
                        .then(|| cipher.resource(directory_key, file_key, directory_count));
                    Written {
                        offset: at_output,
                        size,
                        checksum: write_resource(size, plain, output, transform)?,
                    }
                } else {
                    let sum = stream(source, output, before.size, None, false)?;
                    if sum != before.checksum_raw {
                        return Err(FormatError("CPZ resource checksum mismatch").into());
                    }
                    Written {
                        offset: at_output,
                        size: before.size,
                        checksum: sum,
                    }
                };
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

    /// Copy plaintext with bounded memory, validating the ciphertext checksum when present.
    pub fn copy_entry_verified(
        &self,
        source: &mut (impl Read + Seek),
        i: usize,
        out: &mut impl Write,
    ) -> Result<(), ArchiveError> {
        if self.version().is_flat() {
            return self.copy_flat_entry_verified(source, i, out);
        }
        let entry = self
            .entries()
            .get(i)
            .ok_or(FormatError("CPZ entry out of bounds"))?;
        let (count, encrypted, (d, f), size) = match self {
            Self::Cpz5(index) => (
                index.layout.directory_count,
                index.layout.encryption_mode == 1,
                index.entry_keys[i],
                CPZ5_HEADER_SIZE,
            ),
            Self::Cpz6(index) => (
                index.layout.directory_count,
                true,
                index.entry_keys[i],
                CPZ6_HEADER_SIZE,
            ),
            Self::Cpz7(index) => (
                index.layout.directory_count,
                index.layout.encryption_mode == 1,
                index.entry_keys[i],
                CPZ7_HEADER_SIZE,
            ),
            _ => unreachable!(),
        };
        let cipher = if encrypted {
            source.rewind()?;
            let mut header = [0; CPZ7_HEADER_SIZE];
            source.read_exact(&mut header[..size])?;
            Some(Cipher::new(self, &header[..size]).resource(d, f, count))
        } else {
            None
        };
        source.seek(SeekFrom::Start(entry.offset))?;
        if stream(source, out, entry.size, cipher, false)? != entry.checksum_raw {
            return Err(FormatError("CPZ resource checksum mismatch").into());
        }
        Ok(())
    }

    /// Older read paths tolerate CPZ6 files with unchecked header fields. Verify both
    /// fields on publication, in addition to the format checks made when reopening.
    pub fn verify_written_index(
        &self,
        source: &mut (impl Read + Seek),
    ) -> Result<(), ArchiveError> {
        if let Self::Cpz6(index) = self {
            source.rewind()?;
            let mut h = [0; CPZ6_HEADER_SIZE];
            source.read_exact(&mut h)?;
            let mut bytes = vec![0; index.layout.data_offset as usize - CPZ6_HEADER_SIZE];
            source.read_exact(&mut bytes)?;
            let mut expected = h;
            finish_header(&mut expected, &bytes);
            if expected != h {
                return Err(FormatError("CPZ6 index digest or header checksum mismatch").into());
            }
        }
        Ok(())
    }
}

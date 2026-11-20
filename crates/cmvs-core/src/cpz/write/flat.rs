//! Stream flat CPZ1/2/3 entries. All three ciphers repeat their key every 64 bytes.
use super::*;

const CHUNK: usize = 64 * 1024;
#[derive(Clone, Copy)]
enum Cipher {
    V1,
    V2(u32),
    V3(u32),
}

impl Cipher {
    fn entry(index: &ArchiveIndex, i: usize) -> Result<Self, ArchiveError> {
        Ok(match index {
            ArchiveIndex::Cpz1(_) => Self::V1,
            ArchiveIndex::Cpz2(index) => Self::V2(index.entry_keys[i]),
            ArchiveIndex::Cpz3(index) => Self::V3(index.entry_keys[i]),
            _ => return Err(FormatError("expected CPZ1, CPZ2 or CPZ3").into()),
        })
    }

    fn crypt(self, bytes: &mut [u8], encrypt: bool) {
        match self {
            Self::V1 => cpz1::crypt(bytes, encrypt),
            Self::V2(key) => cpz2::crypt(bytes, key, encrypt),
            Self::V3(key) => cpz3::crypt(bytes, key, encrypt),
        }
    }

    fn has_checksum(self) -> bool {
        !matches!(self, Self::V1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transfer {
    Copy,
    Encrypt,
    Decrypt,
}

/// read_exact keeps all intermediate chunks aligned to the cipher and checksum periods,
/// even when individual reads are short. Only the final chunk processes tail bytes.
fn transfer(
    input: &mut impl Read,
    output: &mut impl Write,
    size: u32,
    cipher: Cipher,
    mode: Transfer,
    buffer: &mut [u8],
) -> Result<u32, ArchiveError> {
    let mut left = size as usize;
    let mut sum = cpz3_checksum(&[]);
    while left != 0 {
        let n = left.min(buffer.len());
        let chunk = &mut buffer[..n];
        input.read_exact(chunk)?;
        if mode == Transfer::Encrypt {
            cipher.crypt(chunk, true);
        }
        if cipher.has_checksum() {
            sum = cpz3::checksum_chunk(sum, chunk);
        }
        if mode == Transfer::Decrypt {
            cipher.crypt(chunk, false);
        }
        output.write_all(chunk)?;
        left -= n;
    }
    Ok(sum)
}

fn check_sum(cipher: Cipher, actual: u32, entry: &Entry) -> Result<(), ArchiveError> {
    if cipher.has_checksum() && actual != entry.checksum_raw {
        return Err(FormatError("CPZ resource checksum mismatch").into());
    }
    Ok(())
}

fn write_resource<R: Read>(
    size: u32,
    mut plain: R,
    out: &mut impl Write,
    cipher: Cipher,
    buffer: &mut [u8],
) -> Result<u32, ArchiveError> {
    let sum = transfer(&mut plain, out, size, cipher, Transfer::Encrypt, buffer)?;
    if plain.read(&mut [0])? != 0 {
        return Err(FormatError("resource grew while packing").into());
    }
    Ok(sum)
}

fn finish(
    output: &mut (impl Write + Seek),
    header: &mut [u8],
    index: &mut [u8],
    cipher: Cipher,
) -> Result<(), ArchiveError> {
    cipher.crypt(index, true);
    if cipher.has_checksum() {
        put(header, 12, cpz3_checksum(index));
    }
    output.rewind()?;
    output.write_all(header)?;
    output.write_all(index)?;
    Ok(())
}

fn data_offset(output: &mut impl Seek, start: u64) -> Result<u32, ArchiveError> {
    u32::try_from(output.stream_position()? - start)
        .map_err(|_| FormatError("flat archive data offset exceeds 32 bits").into())
}

impl ArchiveVersion {
    /// Create a flat archive with deterministic zero cipher keys. Names are stored in
    /// the supplied order, including duplicates. Opens one plaintext stream at a time.
    pub fn write_flat<R: Read>(
        self,
        output: &mut (impl Write + Seek),
        names: &[impl AsRef<[u8]>],
        mut open: impl FnMut(usize) -> Result<(u32, R), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let (magic, count_mask, size_mask, key_word, file_key, cipher) = match self {
            Self::Cpz1 => (b"CPZ1", 0, 0, None, 0, Cipher::V1),
            Self::Cpz2 => (
                b"CPZ2",
                0xe47c59f3,
                0x3f71de2a,
                Some(cpz2::INDEX_KEY_MASK),
                cpz2::FILE_KEY_MASK,
                Cipher::V2(0),
            ),
            Self::Cpz3 => (
                b"CPZ3",
                0x5e9c4f37,
                0xf32aed17,
                Some(cpz3::INDEX_KEY_MASK),
                cpz3::FILE_KEY_MASK,
                Cipher::V3(0),
            ),
            _ => return Err(FormatError("expected CPZ1, CPZ2 or CPZ3").into()),
        };
        if names.len() > 1_000_000 {
            return Err(FormatError("flat archive file count exceeds allocation limit").into());
        }
        let mut index = Vec::new();
        for name in names {
            let name = name.as_ref();
            if name.is_empty() || name.contains(&0) || (self == Self::Cpz3 && name.len() > 4096) {
                return Err(FormatError("invalid flat archive filename").into());
            }
            if name.len() + 25 > crate::MAX_PAYLOAD - index.len() {
                return Err(FormatError("flat archive index exceeds allocation limit").into());
            }
            let at = index.len();
            let length = 25 + name.len();
            index.resize(at + length, 0);
            put(&mut index, at, length as u32);
            put(&mut index, at + 20, file_key);
            index[at + 24..at + length - 1].copy_from_slice(name);
        }
        let mut header = magic.to_vec();
        for value in [
            names.len() as u32 ^ count_mask,
            index.len() as u32 ^ size_mask,
            0,
        ] {
            header.extend(value.to_le_bytes());
        }
        if let Some(key) = key_word {
            header.extend(key.to_le_bytes());
        }
        empty_output(output)?;
        let start = (header.len() + index.len()) as u64;
        output.seek(SeekFrom::Start(start))?;
        let mut buffer = vec![0; CHUNK];
        let mut at = 0;
        for (i, name) in names.iter().enumerate() {
            let offset = data_offset(output, start)?;
            let (size, plain) = open(i)?;
            let sum = write_resource(size, plain, output, cipher, &mut buffer)?;
            put(&mut index, at + 4, size);
            put(&mut index, at + 8, offset);
            if cipher.has_checksum() {
                put(&mut index, at + 16, sum);
            }
            at += 25 + name.as_ref().len();
        }
        finish(output, &mut header, &mut index, cipher)
    }
}

impl ArchiveIndex {
    /// Stream a flat archive entry to an output, checking its stored checksum when present.
    /// The output may be partial on error.
    pub fn copy_flat_entry_verified(
        &self,
        source: &mut (impl Read + Seek),
        i: usize,
        out: &mut impl Write,
    ) -> Result<(), ArchiveError> {
        let entry = self
            .entries()
            .get(i)
            .ok_or(FormatError("invalid CPZ entry number"))?;
        let cipher = Cipher::entry(self, i)?;
        source.seek(SeekFrom::Start(entry.offset))?;
        let sum = transfer(
            source,
            out,
            entry.size,
            cipher,
            Transfer::Decrypt,
            &mut vec![0; CHUNK],
        )?;
        check_sum(cipher, sum, entry)
    }

    /// Rebuild a CPZ1/2/3 archive in an empty stream, opening one replacement at a time.
    /// The callback supplies its exact length and reader, or None to retain the entry.
    pub fn write_flat_replacements<R: Read>(
        &self,
        source: &mut (impl Read + Seek),
        output: &mut (impl Write + Seek),
        mut open: impl FnMut(usize) -> Result<Option<(u32, R)>, ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let header_size = match self {
            Self::Cpz1(_) => CPZ1_HEADER_SIZE,
            Self::Cpz2(_) => CPZ2_HEADER_SIZE,
            Self::Cpz3(_) => CPZ3_HEADER_SIZE,
            _ => return Err(FormatError("expected CPZ1, CPZ2 or CPZ3").into()),
        };
        empty_output(output)?;
        let file_bytes = source.seek(SeekFrom::End(0))?;
        source.rewind()?;
        let mut header = vec![0; header_size];
        source.read_exact(&mut header)?;
        let mismatch = FormatError("CPZ source does not match the parsed index");
        let (index_cipher, index_bytes, key_mask, keys) = match self {
            Self::Cpz1(i) if Cpz1Layout::parse(&header, file_bytes)? == i.layout => {
                (Cipher::V1, i.layout.index_bytes, 0, None)
            }
            Self::Cpz2(i) if Cpz2Layout::parse(&header, file_bytes)? == i.layout => (
                Cipher::V2(word(&header, 16) ^ cpz2::INDEX_KEY_MASK),
                i.layout.index_bytes,
                cpz2::FILE_KEY_MASK,
                Some(&i.entry_keys),
            ),
            Self::Cpz3(i) if Cpz3Layout::parse(&header, file_bytes)? == i.layout => (
                Cipher::V3(word(&header, 16) ^ cpz3::INDEX_KEY_MASK),
                i.layout.index_bytes,
                cpz3::FILE_KEY_MASK,
                Some(&i.entry_keys),
            ),
            _ => return Err(mismatch.into()),
        };
        let mut index = vec![0; index_bytes as usize];
        source.read_exact(&mut index)?;
        if index_cipher.has_checksum() && cpz3_checksum(&index) != word(&header, 12) {
            return Err(FormatError("CPZ index checksum mismatch").into());
        }
        index_cipher.crypt(&mut index, false);
        let data_start = header_size as u64 + u64::from(index_bytes);
        {
            let (entries, raw_keys) = crate::cpz::flat::records(
                &index,
                self.entries().len() as u32,
                data_start,
                file_bytes,
            )?;
            if entries != self.entries()
                || keys.is_some_and(|keys| {
                    !raw_keys
                        .iter()
                        .map(|k| k ^ key_mask)
                        .eq(keys.iter().copied())
                })
            {
                return Err(mismatch.into());
            }
        }
        output.seek(SeekFrom::Start(data_start))?;
        let mut buffer = vec![0; CHUNK];
        let mut at = 0;
        for (i, entry) in self.entries().iter().enumerate() {
            let cipher = Cipher::entry(self, i)?;
            let offset = data_offset(output, data_start)?;
            source.seek(SeekFrom::Start(entry.offset))?;
            if let Some((size, replacement)) = open(i)? {
                if cipher.has_checksum() {
                    let sum = transfer(
                        source,
                        &mut std::io::sink(),
                        entry.size,
                        cipher,
                        Transfer::Copy,
                        &mut buffer,
                    )?;
                    check_sum(cipher, sum, entry)?;
                }
                let sum = write_resource(size, replacement, output, cipher, &mut buffer)?;
                put(&mut index, at + 4, size);
                if cipher.has_checksum() {
                    put(&mut index, at + 16, sum);
                }
            } else {
                let sum = transfer(
                    source,
                    output,
                    entry.size,
                    cipher,
                    Transfer::Copy,
                    &mut buffer,
                )?;
                check_sum(cipher, sum, entry)?;
            }
            put(&mut index, at + 8, offset);
            at += word(&index, at) as usize;
        }
        finish(output, &mut header, &mut index, index_cipher)
    }
}

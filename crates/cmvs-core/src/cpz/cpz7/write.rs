//! Rewrite existing CPZ7 entries while preserving directory records and cipher settings.
use super::*;
use std::{collections::BTreeMap, io::Write};

fn put(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn inverse_substitute(bytes: &mut [u8], key: u32, seed: u32, xor: u8) {
    let table = crypto::table(key, seed);
    let mut inverse = [0; 256];
    for (i, byte) in table.into_iter().enumerate() {
        inverse[byte as usize] = i as u8;
    }
    for byte in bytes {
        *byte = inverse[*byte as usize] ^ xor;
    }
}
fn encrypt_directory(bytes: &mut [u8], keys: &crypto::Keys, step: u32) {
    let key = keys.directory_key();
    let mut seed = 0x76548aefu32;
    let mut flag = 0;
    let (words, tail) = bytes.as_chunks_mut::<4>();
    for word in words {
        let value = u32::from_le_bytes(*word)
            .wrapping_add(seed)
            .rotate_right(3)
            .wrapping_add(0x4a91c262)
            ^ key[flag];
        *word = value.to_le_bytes();
        seed = seed.wrapping_add(step);
        flag = (flag + 1) & 3;
    }
    for byte in tail {
        *byte = byte.wrapping_sub(0x37) ^ (key[flag] >> 6) as u8;
        flag = (flag + 1) & 3;
    }
    inverse_substitute(bytes, keys.key, keys.digest[1], 0x3a);
}
fn encrypt_files(bytes: &mut [u8], directory: u32, keys: &crypto::Keys, step: u32) {
    let key = keys.file_key(directory);
    let mut seed = 0x2a65cb4fu32;
    let mut flag = 0;
    let (words, tail) = bytes.as_chunks_mut::<4>();
    for word in words {
        let value = u32::from_le_bytes(*word)
            .wrapping_sub(0x37a19e8b)
            .rotate_right(2)
            .wrapping_add(seed)
            ^ key[flag];
        *word = value.to_le_bytes();
        seed = seed.wrapping_sub(step);
        flag = (flag + 1) & 3;
    }
    for byte in tail {
        *byte = byte.wrapping_sub(3) ^ (key[flag] >> 4) as u8;
        flag = (flag + 1) & 3;
    }
    inverse_substitute(bytes, keys.key, keys.digest[2], 0x7e);
}
fn encrypt_outer(bytes: &mut [u8], key: u32) {
    let key = key ^ 0x3795b39a;
    let table: [u32; 24] = std::array::from_fn(|i| {
        u32::from_le_bytes(crypto::TABLE_BYTES[i * 4..i * 4 + 4].try_into().unwrap())
            .wrapping_sub(key)
    });
    let shift = ((key ^ (key >> 8) ^ (key >> 16) ^ (key >> 24) ^ 0xfffffffb) & 15) + 7;
    let mut flag = 5;
    let (words, tail) = bytes.as_chunks_mut::<4>();
    for word in words {
        *word = (u32::from_le_bytes(*word)
            .wrapping_sub(0x01010101)
            .rotate_left(shift)
            .wrapping_sub(0x784c5062)
            ^ table[flag])
            .to_le_bytes();
        flag = (flag + 1) % 24;
    }
    let length = tail.len();
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = byte.wrapping_add(0x7d) ^ (table[flag] >> (4 * (length - i))) as u8;
        flag = (flag + 1) % 24;
    }
}
fn encrypt_resource(bytes: &mut [u8], keys: &crypto::Keys, seed: u32) {
    let table = crypto::table(keys.digest[3], keys.key);
    let key_bytes: [u8; 96] = std::array::from_fn(|i| {
        (keys.digest[1] >> 2) as u8 ^ table[crypto::TABLE_BYTES[i] as usize]
    });
    let decrypt: [u32; 24] = std::array::from_fn(|i| {
        u32::from_le_bytes(key_bytes[i * 4..i * 4 + 4].try_into().unwrap()) ^ seed
    });
    let mut key = 0x2748c39eu32;
    let mut flag = 10;
    let (words, tail) = bytes.as_chunks_mut::<4>();
    for word in words {
        let value = u32::from_le_bytes(*word);
        *word = ((value ^ keys.digest[(key & 3) as usize]).wrapping_add(seed)
            ^ (decrypt[flag] >> 1)
            ^ decrypt[((key >> 6) & 15) as usize])
            .to_le_bytes();
        key = key.wrapping_add(seed).wrapping_add(value);
        flag = (flag + 1) & 15;
    }
    let mut inverse = [0; 256];
    for (i, byte) in table.into_iter().enumerate() {
        inverse[byte as usize] = i as u8;
    }
    for byte in tail {
        *byte = inverse[*byte as usize] ^ 0xae;
    }
}

impl Cpz7Index {
    /// Replace entries by index in a new, empty output stream. Names, directory/file
    /// keys, index-key block and encryption mode are retained; other payloads are
    /// copied encrypted after checksum verification. No entries are added or removed.
    pub fn write_replacements<R: Read + Seek, W: Write + Seek>(
        source: &mut R,
        output: &mut W,
        parameters: Cpz7Parameters,
        replacements: &BTreeMap<usize, Vec<u8>>,
    ) -> Result<(), ArchiveError> {
        let index = Self::read_with_parameters(source, parameters)?;
        if replacements
            .iter()
            .any(|(i, bytes)| *i >= index.entries.len() || bytes.len() > crate::MAX_PAYLOAD)
        {
            return Err(FormatError("CPZ7 replacement index or size exceeds bounds").into());
        }
        if output.seek(SeekFrom::End(0))? != 0 {
            return Err(FormatError("CPZ7 output stream must be empty").into());
        }
        source.rewind()?;
        let mut header = [0; CPZ7_HEADER_SIZE];
        source.read_exact(&mut header)?;
        let mut bytes = vec![0; index.layout.data_offset as usize - CPZ7_HEADER_SIZE];
        source.read_exact(&mut bytes)?;
        // Reserve the unchanged index extent; its final digest is written last.
        output.write_all(&header)?;
        output.write_all(&bytes)?;
        let directory_bytes = index.layout.directory_index_bytes as usize;
        let index_bytes = directory_bytes + index.layout.file_index_bytes as usize;
        let index_key = unpack_index_key(&bytes[index_bytes..])?;
        for (i, byte) in bytes[..index_bytes].iter_mut().enumerate() {
            *byte ^= index_key[(i + 3) % 0x3ff];
        }
        crypto::outer_index_v7(&mut bytes[..index_bytes], index.keys.key);
        let (directories, remaining) = bytes.split_at_mut(directory_bytes);
        let files = &mut remaining[..index.layout.file_index_bytes as usize];
        let dir_step = parameters.directory_step_xor ^ 0x10fb562a;
        let file_step = parameters.file_step_xor ^ 0x0139fa9b;
        crypto::directory_index_with_step(directories, &index.keys, dir_step);
        let mut dirs = Vec::new();
        let mut at = 0;
        for _ in 0..index.layout.directory_count {
            let rec = record(directories, at, 17)?;
            dirs.push((
                read_u32(rec, 4)? as usize,
                read_u32(rec, 8)? as usize,
                read_u32(rec, 12)?,
            ));
            at += rec.len();
        }
        let mut entry = 0;
        for (d, &(count, at, directory_key)) in dirs.iter().enumerate() {
            let end = dirs.get(d + 1).map_or(files.len(), |dir| dir.1);
            let block = &mut files[at..end];
            crypto::file_index_with_step(block, directory_key, &index.keys, file_step);
            let mut at = 0;
            for _ in 0..count {
                let length = record(block, at, 29)?.len();
                let rec = &mut block[at..at + length];
                let payload = if let Some(replacement) = replacements.get(&entry) {
                    let mut payload = replacement.clone();
                    if index.layout.encryption_mode == 1 {
                        let (_, file_key) = index.entry_keys[entry];
                        let resource_key = (index.keys.key ^ directory_key.wrapping_add(file_key))
                            .wrapping_add(index.layout.directory_count)
                            ^ parameters.resource_seed_xor;
                        let seed = index.keys.seed ^ resource_key.wrapping_add(0xa3c61785);
                        encrypt_resource(&mut payload, &index.keys, seed);
                    }
                    payload
                } else {
                    let (metadata, payload) = index.read_raw(source, entry)?;
                    if data_checksum(&payload) != metadata.checksum_raw {
                        return Err(FormatError("CPZ7 resource checksum mismatch").into());
                    }
                    payload
                };
                let offset = output.stream_position()? - index.layout.data_offset;
                put(rec, 4, offset as u32);
                put(rec, 8, (offset >> 32) as u32);
                put(rec, 12, payload.len() as u32);
                put(rec, 16, 0);
                put(rec, 20, data_checksum(&payload));
                output.write_all(&payload)?;
                at += length;
                entry += 1;
            }
            encrypt_files(block, directory_key, &index.keys, file_step);
        }
        encrypt_directory(directories, &index.keys, dir_step);
        encrypt_outer(&mut bytes[..index_bytes], index.keys.key);
        for (i, byte) in bytes[..index_bytes].iter_mut().enumerate() {
            *byte ^= index_key[(i + 3) % 0x3ff];
        }
        header[16..32].copy_from_slice(&crypto::md5(&bytes));
        let initial = read_u32(&header, 64)?.wrapping_sub(0x6dc5a9b4);
        let checksum = data_checksum(&header[..64])
            .wrapping_sub(0x5a902b7c)
            .wrapping_add(initial);
        put(&mut header, 68, checksum);
        output.rewind()?;
        output.write_all(&header)?;
        output.write_all(&bytes)?;
        Ok(())
    }
}

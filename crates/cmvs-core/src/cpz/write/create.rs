//! New CPZ5/6 archives use deterministic header/directory/file keys and mode 1.
use super::*;

impl Cpz5Index {
    /// Create a new CPZ5 archive using the selected packing mode (5..=9) and
    /// resource encryption mode 1. Names retain their raw encoding and order;
    /// header/directory/file keys use deterministic defaults. Output must be empty.
    pub fn write<W: Write + Seek>(
        output: &mut W,
        mode: Cpz5Mode,
        directories: &[ArchiveDirectory],
    ) -> Result<(), ArchiveError> {
        create(output, directories, Some(mode))
    }
}
impl Cpz6Index {
    /// Create a new CPZ6 archive using resource encryption mode 1. Names retain
    /// their raw encoding and order; keys use deterministic defaults. Output must
    /// be empty. Failures may leave a partial output, as with replacement writing.
    pub fn write<W: Write + Seek>(
        output: &mut W,
        directories: &[ArchiveDirectory],
    ) -> Result<(), ArchiveError> {
        create(output, directories, None)
    }
}
fn valid_name(name: &[u8], allow_empty: bool) -> bool {
    (allow_empty || !name.is_empty()) && name.len() <= 4096 && !name.contains(&0)
}
fn create<W: Write + Seek>(
    output: &mut W,
    directories: &[ArchiveDirectory],
    mode: Option<Cpz5Mode>,
) -> Result<(), ArchiveError> {
    let mut directory_len = 0;
    let mut file_len = 0;
    let mut files = 0usize;
    let mut data_len = 0u64;
    for dir in directories {
        if !valid_name(&dir.name, true) {
            return Err(FormatError("invalid CPZ directory name").into());
        }
        directory_len += 17 + dir.name.len();
        if directory_len + file_len > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ index exceeds allocation limit").into());
        }
        for file in &dir.files {
            if !valid_name(&file.name, false) || file.data.len() > crate::MAX_PAYLOAD {
                return Err(FormatError("invalid CPZ filename or resource size").into());
            }
            file_len += 25 + file.name.len();
            files += 1;
            if directory_len + file_len > crate::MAX_PAYLOAD || files > 1_000_000 {
                return Err(FormatError("CPZ index exceeds allocation limit").into());
            }
            if mode.is_some() && data_len > u64::from(u32::MAX) {
                return Err(FormatError("CPZ5 data offset exceeds 32 bits").into());
            }
            data_len += file.data.len() as u64;
        }
    }
    empty_output(output)?;
    let early = mode.is_some();
    let mut header = [0; CPZ6_HEADER_SIZE];
    header[..4].copy_from_slice(if early { b"CPZ5" } else { b"CPZ6" });
    put(
        &mut header,
        4,
        directories.len() as u32 ^ if early { 0xfe3a53d9 } else { 0xfe3a53da },
    );
    put(
        &mut header,
        8,
        directory_len as u32 ^ if early { 0x37f298e7 } else { 0x37f298e8 },
    );
    put(
        &mut header,
        12,
        file_len as u32 ^ if early { 0x7a6f3a2c } else { 0x7a6f3a2d },
    );
    let masks = if early {
        [0x43de7c19, 0xcc65f415, 0xd016a93c, 0x97a3ba9a]
    } else {
        [0x43de7c1a, 0xcc65f416, 0xd016a93d, 0x97a3ba9b]
    };
    for (i, mask) in masks.into_iter().enumerate() {
        put(&mut header, 32 + i * 4, mask);
    }
    put(&mut header, 48, if early { 0xae7d39bf } else { 0xae7d39b7 });
    put(
        &mut header,
        52,
        1 ^ if early { 0xfb73a955 } else { 0xfb73a956 },
    );
    if !early {
        put(&mut header, 56, 0x37acf832);
    }
    let cipher = if let Some(mode) = mode {
        Cipher::Cpz5(mode, mode.keys(&header))
    } else {
        Cipher::Cpz6(crypto::Keys::from_header(&header))
    };
    let mut dirs = Vec::with_capacity(directory_len);
    let mut files = Vec::with_capacity(file_len);
    let mut offset = 0u64;
    for dir in directories {
        for field in [
            17 + dir.name.len() as u32,
            dir.files.len() as u32,
            files.len() as u32,
            0,
        ] {
            dirs.extend(field.to_le_bytes());
        }
        dirs.extend(&dir.name);
        dirs.push(0);
        for file in &dir.files {
            for field in [
                25 + file.name.len() as u32,
                offset as u32,
                (offset >> 32) as u32,
                file.data.len() as u32,
                0,
                0,
            ] {
                files.extend(field.to_le_bytes());
            }
            files.extend(&file.name);
            files.push(0);
            offset += file.data.len() as u64;
        }
    }
    output.write_all(&header)?;
    output.write_all(&dirs)?;
    output.write_all(&files)?;
    let mut at = 0;
    for dir in directories {
        let start = at;
        for file in &dir.files {
            let mut data = file.data.clone();
            cipher.encode_resource(&mut data, 0, 0, directories.len() as u32);
            put(&mut files, at + 16, data_checksum(&data));
            output.write_all(&data)?;
            at += 25 + file.name.len();
        }
        cipher.encode_files(&mut files[start..at], 0);
    }
    cipher.encode_directories(&mut dirs);
    dirs.extend(files);
    cipher.encode_outer(&mut dirs);
    finish_header(&mut header, &dirs);
    output.rewind()?;
    output.write_all(&header)?;
    output.write_all(&dirs)?;
    Ok(())
}

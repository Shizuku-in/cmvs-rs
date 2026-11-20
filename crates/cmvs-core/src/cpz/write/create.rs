//! Deterministic directory archive creation, with one input stream open at a time.
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
/// Directory and file names in on-disk order, including duplicate entries.
#[derive(Default)]
pub struct DirectoryNames {
    pub name: Vec<u8>,
    pub files: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryFormat {
    Cpz5 {
        mode: Cpz5Mode,
        encrypted: bool,
    },
    Cpz6,
    Cpz7 {
        parameters: Cpz7Parameters,
        encrypted: bool,
    },
}

fn create<W: Write + Seek>(
    out: &mut W,
    dirs: &[ArchiveDirectory],
    mode: Option<Cpz5Mode>,
) -> Result<(), ArchiveError> {
    let names: Vec<_> = dirs
        .iter()
        .map(|d| DirectoryNames {
            name: d.name.clone(),
            files: d.files.iter().map(|f| f.name.clone()).collect(),
        })
        .collect();
    let format = mode.map_or(DirectoryFormat::Cpz6, |mode| DirectoryFormat::Cpz5 {
        mode,
        encrypted: true,
    });
    let mut files = dirs.iter().flat_map(|d| &d.files);
    format.write(out, &names, |_| {
        let bytes = &files.next().expect("one input per name").data;
        if bytes.len() > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ resource exceeds allocation limit").into());
        }
        Ok((bytes.len() as u32, bytes.as_slice()))
    })
}
fn valid_name(name: &[u8], empty: bool) -> bool {
    (empty || !name.is_empty()) && name.len() <= 4096 && !name.contains(&0)
}

impl DirectoryFormat {
    pub fn read(&self, input: &mut (impl Read + Seek)) -> Result<ArchiveIndex, ArchiveError> {
        Ok(match self {
            Self::Cpz5 { mode, .. } => ArchiveIndex::Cpz5(Cpz5Index::read_with_mode(input, *mode)?),
            Self::Cpz6 => ArchiveIndex::Cpz6(Cpz6Index::read(input)?),
            Self::Cpz7 { parameters, .. } => {
                ArchiveIndex::Cpz7(Cpz7Index::read_with_parameters(input, *parameters)?)
            }
        })
    }
    pub fn write<R: Read>(
        self,
        output: &mut (impl Write + Seek),
        directories: &[DirectoryNames],
        mut open: impl FnMut(usize) -> Result<(u32, R), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let (magic, encrypted) = match self {
            Self::Cpz5 { encrypted, .. } => (b"CPZ5", encrypted),
            Self::Cpz6 => (b"CPZ6", true),
            Self::Cpz7 { encrypted, .. } => (b"CPZ7", encrypted),
        };
        let early = matches!(self, Self::Cpz5 { .. });
        let v7 = matches!(self, Self::Cpz7 { .. });
        let fixed = if v7 { 28 } else { 24 };
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut count = 0;
        for dir in directories {
            if !valid_name(&dir.name, true) {
                return Err(FormatError("invalid CPZ directory name").into());
            }
            for v in [17 + dir.name.len(), dir.files.len(), files.len(), 0] {
                dirs.extend((v as u32).to_le_bytes());
            }
            dirs.extend(&dir.name);
            dirs.push(0);
            for name in &dir.files {
                if !valid_name(name, false) || (!v7 && name.len() >= 64) {
                    return Err(FormatError("invalid CPZ filename").into());
                }
                count += 1;
                if count > 1_000_000
                    || dirs.len() + files.len() + fixed + 1 + name.len() > crate::MAX_PAYLOAD
                {
                    return Err(FormatError("CPZ index exceeds allocation limit").into());
                }
                let at = files.len();
                let len = fixed + name.len() + 1;
                files.resize(at + len, 0);
                put(&mut files, at, len as u32);
                files[at + fixed..at + fixed + name.len()].copy_from_slice(name);
            }
            if dirs.len() + files.len() > crate::MAX_PAYLOAD {
                return Err(FormatError("CPZ index exceeds allocation limit").into());
            }
        }
        // A one-symbol Huffman tree emits an all-zero index key. The last four bytes
        // contain its leaf bit and eight zero symbol bits, padded to a word.
        let mut key_block = Vec::new();
        if v7 {
            key_block.resize(28, 0);
            put(&mut key_block, 16, 1024);
            let digest = crypto::md5(&key_block[16..]);
            key_block[..16].copy_from_slice(&digest);
        }
        if dirs.len() + files.len() + key_block.len() > crate::MAX_PAYLOAD {
            return Err(FormatError("CPZ index exceeds allocation limit").into());
        }
        let mut header = vec![
            0;
            if v7 {
                CPZ7_HEADER_SIZE
            } else {
                CPZ6_HEADER_SIZE
            }
        ];
        header[..4].copy_from_slice(magic);
        put(
            &mut header,
            4,
            directories.len() as u32 ^ if early { 0xfe3a53d9 } else { 0xfe3a53da },
        );
        put(
            &mut header,
            8,
            dirs.len() as u32 ^ if early { 0x37f298e7 } else { 0x37f298e8 },
        );
        put(
            &mut header,
            12,
            files.len() as u32 ^ if early { 0x7a6f3a2c } else { 0x7a6f3a2d },
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
            u32::from(encrypted) ^ if early { 0xfb73a955 } else { 0xfb73a956 },
        );
        if !early {
            put(&mut header, 56, 0x37acf832);
        }

        let cipher = match self {
            Self::Cpz5 { mode, .. } => Cipher::Cpz5(mode, mode.keys(&header)),
            Self::Cpz6 => Cipher::Cpz6(crypto::Keys::from_header(&header)),
            Self::Cpz7 { parameters, .. } => {
                put(&mut header, 64, key_block.len() as u32 ^ 0x65ef99f3);
                let mut keys = crypto::Keys::from_header_v7(&header);
                keys.seed = keys.seed.wrapping_add(parameters.seed_add);
                Cipher::Cpz7(parameters, keys)
            }
        };
        empty_output(output)?;
        let data_start = (header.len() + dirs.len() + files.len() + key_block.len()) as u64;
        output.seek(SeekFrom::Start(data_start))?;
        let mut at = 0;
        let mut i = 0;
        for dir in directories {
            let start = at;
            for name in &dir.files {
                let offset = output.stream_position()? - data_start;
                if early && offset > u64::from(u32::MAX) {
                    return Err(FormatError("CPZ5 data offset exceeds 32 bits").into());
                }
                let (size, plain) = open(i)?;
                let sum = write_resource(
                    size,
                    plain,
                    output,
                    encrypted.then(|| cipher.resource(0, 0, directories.len() as u32)),
                )?;
                put(&mut files, at + 4, offset as u32);
                put(&mut files, at + 8, (offset >> 32) as u32);
                put(&mut files, at + 12, size);
                put(&mut files, at + if v7 { 20 } else { 16 }, sum);
                i += 1;
                at += fixed + 1 + name.len();
            }
            cipher.encode_files(&mut files[start..at], 0);
        }
        cipher.encode_directories(&mut dirs);
        dirs.extend(files);
        cipher.encode_outer(&mut dirs);
        dirs.extend(key_block);
        finish_header(&mut header, &dirs);
        output.rewind()?;
        output.write_all(&header)?;
        output.write_all(&dirs)?;
        Ok(())
    }
}

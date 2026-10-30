//! Face-name font resolution standing in for GDI's CreateFontA lookup. The explicit
//! `--font` file serves the built-in default face; other faces are found by family name
//! in the directories `cmvs_native::font_directories` lists for this platform (none yet
//! outside Windows, where only the default face resolves).
use ab_glyph::FontVec;
use cmvs_core::{FormatError, settings::DEFAULT_FONT_FACE};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FONT: usize = 32 * 1024 * 1024;
const MAX_FONT_FILES: usize = 8192;

pub struct FontLibrary {
    default: Arc<FontVec>,
    loaded: BTreeMap<Vec<u8>, Arc<FontVec>>,
    /// Lowercased family name -> font file and collection index; built on first use.
    index: Option<BTreeMap<String, (PathBuf, u32)>>,
    directories: Vec<PathBuf>,
}

impl FontLibrary {
    pub fn new(default: Arc<FontVec>) -> Self {
        Self {
            default,
            loaded: BTreeMap::new(),
            index: None,
            directories: cmvs_native::font_directories(),
        }
    }
    /// Research override of the searched directories.
    pub fn with_directories(default: Arc<FontVec>, directories: Vec<PathBuf>) -> Self {
        Self {
            directories,
            ..Self::new(default)
        }
    }
    /// The font for a face in the game encoding (Shift-JIS).
    pub fn face(&mut self, face: &[u8]) -> Result<Arc<FontVec>, FormatError> {
        if face == DEFAULT_FONT_FACE {
            return Ok(self.default.clone());
        }
        if let Some(font) = self.loaded.get(face) {
            return Ok(font.clone());
        }
        let (path, index) = self.locate(face)?.ok_or(FormatError(
            "font face not found in the system font directories",
        ))?;
        let mut bytes = Vec::new();
        File::open(&path)
            .and_then(|file| file.take(MAX_FONT as u64 + 1).read_to_end(&mut bytes))
            .map_err(|_| FormatError("located font file cannot be read"))?;
        if bytes.len() > MAX_FONT {
            return Err(FormatError("font exceeds 32 MiB research limit"));
        }
        let font = Arc::new(
            FontVec::try_from_vec_and_index(bytes, index)
                .map_err(|_| FormatError("located font is not a supported outline font"))?,
        );
        self.loaded.insert(face.to_vec(), font.clone());
        Ok(font)
    }
    /// Startup existence check (0x140007f80): whether a face can be resolved.
    pub fn contains(&mut self, face: &[u8]) -> bool {
        face == DEFAULT_FONT_FACE
            || self.loaded.contains_key(face)
            || matches!(self.locate(face), Ok(Some(_)))
    }
    fn locate(&mut self, face: &[u8]) -> Result<Option<(PathBuf, u32)>, FormatError> {
        let (name, _, invalid) = encoding_rs::SHIFT_JIS.decode(face);
        if invalid {
            return Err(FormatError("font face is not valid Shift-JIS"));
        }
        let key = name.to_lowercase();
        let index = match &self.index {
            Some(index) => index,
            None => self.index.insert(build_index(&self.directories)),
        };
        Ok(index.get(&key).cloned())
    }
}

/// First file wins for a name, scanning directories in order and files sorted by path.
fn build_index(directories: &[PathBuf]) -> BTreeMap<String, (PathBuf, u32)> {
    let mut files = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    ["ttf", "ttc", "otf", "otc"].contains(&&*e.to_ascii_lowercase())
                })
            })
            .collect();
        found.sort();
        files.extend(found);
    }
    let mut index = BTreeMap::new();
    for path in files.into_iter().take(MAX_FONT_FILES) {
        let Ok(faces) = family_names(&path) else {
            continue;
        };
        for (face, names) in faces {
            for name in names {
                index
                    .entry(name.to_lowercase())
                    .or_insert_with(|| (path.clone(), face));
            }
        }
    }
    index
}

fn read_at(file: &mut File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Family names (name IDs 1 and 16, Unicode and Windows platforms, every language) of
/// each face in an OpenType file or collection, read without loading the whole file.
pub fn family_names(path: &Path) -> std::io::Result<Vec<(u32, Vec<String>)>> {
    let invalid = || std::io::Error::other("unsupported font header");
    let mut file = File::open(path)?;
    let header = read_at(&mut file, 0, 12)?;
    let offsets = if &header[..4] == b"ttcf" {
        let count = u32_at(&header, 8).ok_or_else(invalid)? as usize;
        if count == 0 || count > 256 {
            return Err(invalid());
        }
        let table = read_at(&mut file, 12, 4 * count)?;
        (0..count)
            .map(|i| u32_at(&table, 4 * i).ok_or_else(invalid))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![0]
    };
    let mut faces = Vec::new();
    for (face, offset) in offsets.into_iter().enumerate() {
        let directory = read_at(&mut file, u64::from(offset), 12)?;
        let tables = usize::from(u16_at(&directory, 4).ok_or_else(invalid)?);
        if tables > 512 {
            return Err(invalid());
        }
        let records = read_at(&mut file, u64::from(offset) + 12, 16 * tables)?;
        let Some(name) = (0..tables).find(|i| &records[16 * i..16 * i + 4] == b"name") else {
            continue;
        };
        let at = u32_at(&records, 16 * name + 8).ok_or_else(invalid)?;
        let length = u32_at(&records, 16 * name + 12).ok_or_else(invalid)? as usize;
        if length > 1 << 20 {
            return Err(invalid());
        }
        faces.push((
            face as u32,
            name_records(&read_at(&mut file, u64::from(at), length)?),
        ));
    }
    Ok(faces)
}

fn name_records(table: &[u8]) -> Vec<String> {
    let (Some(count), Some(strings)) = (u16_at(table, 2), u16_at(table, 4)) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for i in 0..usize::from(count) {
        let at = 6 + 12 * i;
        let field = |n: usize| u16_at(table, at + 2 * n);
        let (Some(platform), Some(encoding), Some(id), Some(length), Some(offset)) =
            (field(0), field(1), field(3), field(4), field(5))
        else {
            break;
        };
        // UTF-16BE records: Unicode platform, Windows symbol/BMP/full repertoire.
        let utf16 = platform == 0 || (platform == 3 && matches!(encoding, 0 | 1 | 10));
        if !matches!(id, 1 | 16) || !utf16 {
            continue;
        }
        let start = usize::from(strings) + usize::from(offset);
        let Some(raw) = table.get(start..start + usize::from(length)) else {
            continue;
        };
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        if let Ok(name) = String::from_utf16(&units)
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sfnt face whose only table is `name`, holding the given (platform, encoding,
    /// name ID, UTF-16 string) records.
    fn face(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let strings: Vec<Vec<u8>> = records
            .iter()
            .map(|r| r.3.encode_utf16().flat_map(u16::to_be_bytes).collect())
            .collect();
        let mut name = Vec::new();
        let start = 6 + 12 * records.len();
        for v in [0, records.len() as u16, start as u16] {
            name.extend(v.to_be_bytes());
        }
        let mut offset = 0u16;
        for (record, string) in records.iter().zip(&strings) {
            for v in [
                record.0,
                record.1,
                0x411,
                record.2,
                string.len() as u16,
                offset,
            ] {
                name.extend(v.to_be_bytes());
            }
            offset += string.len() as u16;
        }
        name.extend(strings.concat());
        let mut font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        font.extend(b"name");
        font.extend(0u32.to_be_bytes());
        font.extend(28u32.to_be_bytes());
        font.extend((name.len() as u32).to_be_bytes());
        font.extend(name);
        font
    }
    /// Rebases each face's table offsets into a `ttcf` collection.
    fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"ttcf".to_vec();
        out.extend([0, 1, 0, 0]);
        out.extend((faces.len() as u32).to_be_bytes());
        let mut at = 12 + 4 * faces.len();
        let mut bodies = Vec::new();
        for face in faces {
            out.extend((at as u32).to_be_bytes());
            let mut face = face.clone();
            let table = u32::from_be_bytes(face[20..24].try_into().unwrap()) as usize + at;
            face[20..24].copy_from_slice(&(table as u32).to_be_bytes());
            at += face.len();
            bodies.push(face);
        }
        out.extend(bodies.concat());
        out
    }

    #[test]
    fn family_names_cover_utf16_records_and_collection_faces() {
        let dir = std::env::temp_dir().join(format!("cmvs-font-library-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let single = face(&[
            (3, 1, 1, "テスト"),
            (3, 1, 1, "Test Face"),
            (3, 1, 4, "Full"),
            (1, 0, 1, "Mac"),
        ]);
        std::fs::write(dir.join("a.ttf"), &single).unwrap();
        let pair = collection(&[
            face(&[(3, 1, 1, "First")]),
            face(&[(0, 3, 16, "Second"), (3, 2, 1, "Legacy")]),
        ]);
        std::fs::write(dir.join("b.ttc"), &pair).unwrap();
        std::fs::write(dir.join("c.txt"), b"not a font").unwrap();
        std::fs::write(dir.join("d.ttf"), b"bad").unwrap();
        assert_eq!(
            family_names(&dir.join("a.ttf")).unwrap(),
            [(0, vec!["テスト".to_owned(), "Test Face".to_owned()])]
        );
        assert_eq!(
            family_names(&dir.join("b.ttc")).unwrap(),
            [
                (0, vec!["First".to_owned()]),
                (1, vec!["Second".to_owned()])
            ]
        );
        let index = build_index(std::slice::from_ref(&dir));
        assert_eq!(index.get("test face"), Some(&(dir.join("a.ttf"), 0)));
        assert_eq!(index.get("テスト"), Some(&(dir.join("a.ttf"), 0)));
        assert_eq!(index.get("second"), Some(&(dir.join("b.ttc"), 1)));
        assert!(!index.contains_key("full") && !index.contains_key("legacy"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! Face-name font resolution standing in for GDI's CreateFontA lookup. The explicit
//! `--font` file serves the built-in default face; other faces are found by family name
//! in the directories `cmvs_native::font_directories` lists for this platform
//! (Windows/macOS system/user folders and Linux Fontconfig directories).
use ab_glyph::{Font, FontVec};
use cmvs_core::{
    FormatError, TextEncoding,
    settings::{DEFAULT_FONT_FACE, font_face_for_encoding},
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FONT: usize = 32 * 1024 * 1024;
const MAX_FONT_FILES: usize = 8192;

/// Encoding-free state shared by renderers: each caller passes the encoding of the
/// game's legacy font face fields (LOGFONT names), so renderers sharing a library
/// may use different encodings.
pub struct FontLibrary {
    default: Arc<FontVec>,
    /// Tried in order for characters neither the selected face nor the default covers.
    fallbacks: Vec<Arc<FontVec>>,
    /// Faces whose resolution does not depend on the encoding (handles, imported
    /// font names and the built-in default).
    loaded: BTreeMap<Vec<u8>, Arc<FontVec>>,
    /// System fonts found by decoded family name; the same bytes may name another
    /// family in another encoding.
    located: BTreeMap<(TextEncoding, Vec<u8>), Arc<FontVec>>,
    /// Lowercased family name -> font file and collection index; built on first use.
    index: Option<BTreeMap<String, (PathBuf, u32)>>,
    directories: Vec<PathBuf>,
    /// Unicode names live outside the game's fixed-width legacy font fields.
    unicode_names: BTreeMap<Vec<u8>, String>,
    unicode_directory: Option<PathBuf>,
}

impl FontLibrary {
    pub fn new(default: Arc<FontVec>) -> Self {
        Self {
            default,
            fallbacks: Vec::new(),
            loaded: BTreeMap::new(),
            located: BTreeMap::new(),
            index: None,
            directories: cmvs_native::font_directories(),
            unicode_names: BTreeMap::new(),
            unicode_directory: None,
        }
    }
    /// Research override of the searched directories.
    pub fn with_directories(default: Arc<FontVec>, directories: Vec<PathBuf>) -> Self {
        Self {
            directories,
            ..Self::new(default)
        }
    }
    pub fn set_user_directory(&mut self, directory: &Path) {
        self.unicode_directory = Some(directory.join("font-faces"));
    }
    /// A stable ASCII handle fits the VM/CSV2 LOGFONT field. The actual family
    /// name stays Unicode, including across restarts, in an immutable UTF-8 file.
    pub fn register_unicode(&mut self, name: &str) -> Result<Vec<u8>, FormatError> {
        if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
            return Err(FormatError("invalid Unicode font family name"));
        }
        let key = format!(
            "CMVS-U-{}",
            &cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(name.as_bytes()))[..24]
        )
        .into_bytes();
        if let Some(directory) = &self.unicode_directory {
            std::fs::create_dir_all(directory)
                .map_err(|_| FormatError("cannot create Unicode font directory"))?;
            let path = directory.join(std::str::from_utf8(&key).expect("ASCII handle"));
            // Existing mappings are immutable: another session cannot silently
            // change which font an older gameplay save refers to.
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(name.as_bytes())
                        .and_then(|()| file.sync_all())
                        .map_err(|_| FormatError("cannot save Unicode font name"))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::fs::read(&path).ok().as_deref() != Some(name.as_bytes()) {
                        return Err(FormatError(
                            "Unicode font handle conflicts with stored name",
                        ));
                    }
                }
                Err(_) => return Err(FormatError("cannot save Unicode font name")),
            }
        }
        self.unicode_names.insert(key.clone(), name.to_owned());
        Ok(key)
    }
    pub fn family_name(
        &mut self,
        face: &[u8],
        encoding: TextEncoding,
    ) -> Result<String, FormatError> {
        if let Some(name) = self.unicode_names.get(face) {
            return Ok(name.clone());
        }
        if face.starts_with(b"CMVS-U-") {
            if face.len() != 31 || !face[7..].iter().all(u8::is_ascii_hexdigit) {
                return Err(FormatError("invalid Unicode font handle"));
            }
            let directory = self
                .unicode_directory
                .as_ref()
                .ok_or(FormatError("Unicode font mapping is unavailable"))?;
            let mut bytes = Vec::new();
            File::open(directory.join(std::str::from_utf8(face).expect("ASCII handle")))
                .and_then(|file| file.take(513).read_to_end(&mut bytes))
                .map_err(|_| FormatError("cannot read Unicode font name"))?;
            let name = String::from_utf8(bytes)
                .map_err(|_| FormatError("font family name is not valid UTF-8"))?;
            if name.is_empty()
                || name.len() > 512
                || name.chars().any(char::is_control)
                || format!(
                    "CMVS-U-{}",
                    &cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(name.as_bytes()))[..24]
                )
                .as_bytes()
                    != face
            {
                return Err(FormatError("invalid stored Unicode font name"));
            }
            self.unicode_names.insert(face.to_vec(), name.clone());
            return Ok(name);
        }
        legacy_family_name(encoding, face)
    }
    /// The font for a face whose legacy name is in `encoding`.
    pub fn face(
        &mut self,
        face: &[u8],
        encoding: TextEncoding,
    ) -> Result<Arc<FontVec>, FormatError> {
        if let Some(font) = self.loaded.get(face) {
            return Ok(font.clone());
        }
        let located = (encoding, face.to_vec());
        if let Some(font) = self.located.get(&located) {
            return Ok(font.clone());
        }
        if face.starts_with(b"CMVS-F-") {
            if face.len() != 31 || !face[7..].iter().all(u8::is_ascii_hexdigit) {
                return Err(FormatError("invalid imported font handle"));
            }
            let directory = self
                .unicode_directory
                .as_ref()
                .ok_or(FormatError("font directory unavailable"))?;
            let mut bytes = Vec::new();
            File::open(directory.join(std::str::from_utf8(face).expect("ASCII handle")))
                .and_then(|file| file.take((MAX_FONT + 5) as u64).read_to_end(&mut bytes))
                .map_err(|_| FormatError("cannot read imported font"))?;
            if bytes.len() < 4 || bytes.len() > MAX_FONT + 4 || imported_key(&bytes) != face {
                return Err(FormatError("invalid imported font data"));
            }
            let index = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
            let font = Arc::new(
                FontVec::try_from_vec_and_index(bytes[4..].to_vec(), index)
                    .map_err(|_| FormatError("invalid imported font face"))?,
            );
            self.loaded.insert(face.to_vec(), font.clone());
            return Ok(font);
        }
        if let Some(directory) = &self.unicode_directory {
            let mapping = directory.join(named_key(face));
            match File::open(mapping) {
                Ok(file) => {
                    let mut reference = Vec::new();
                    file.take(32)
                        .read_to_end(&mut reference)
                        .map_err(|_| FormatError("cannot read imported font name"))?;
                    if reference.len() != 31 || !reference.starts_with(b"CMVS-F-") {
                        return Err(FormatError("invalid imported font reference"));
                    }
                    let font = self.face(&reference, encoding)?;
                    self.loaded.insert(face.to_vec(), font.clone());
                    return Ok(font);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FormatError("cannot read imported font name")),
            }
        }
        if face == DEFAULT_FONT_FACE {
            self.loaded.insert(face.to_vec(), self.default.clone());
            return Ok(self.default.clone());
        }
        if face == font_face_for_encoding(DEFAULT_FONT_FACE, encoding) {
            self.located.insert(located, self.default.clone());
            return Ok(self.default.clone());
        }
        let (path, index) = self.locate(face, encoding)?.ok_or(FormatError(
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
        self.located.insert(located, font.clone());
        Ok(font)
    }
    /// Keep an immutable font copy so game saves can resolve the same face later.
    pub fn import_file(&mut self, bytes: Vec<u8>, index: u32) -> Result<Vec<u8>, FormatError> {
        if bytes.len() > MAX_FONT {
            return Err(FormatError("font exceeds 32 MiB research limit"));
        }
        let font = Arc::new(
            FontVec::try_from_vec_and_index(bytes.clone(), index)
                .map_err(|_| FormatError("invalid OpenType font or collection face index"))?,
        );
        let stored: Vec<_> = index.to_le_bytes().into_iter().chain(bytes).collect();
        let key = imported_key(&stored);
        let directory = self
            .unicode_directory
            .as_ref()
            .ok_or(FormatError("font directory unavailable"))?;
        std::fs::create_dir_all(directory)
            .map_err(|_| FormatError("cannot create font directory"))?;
        let path = directory.join(std::str::from_utf8(&key).expect("ASCII handle"));
        if path.exists() {
            let mut existing = Vec::new();
            File::open(&path)
                .and_then(|file| file.take((MAX_FONT + 5) as u64).read_to_end(&mut existing))
                .map_err(|_| FormatError("cannot check imported font"))?;
            if existing != stored {
                return Err(FormatError("stored font handle conflict"));
            }
        } else {
            use std::io::Write;
            let temporary = path.with_extension("tmp");
            let mut file =
                File::create(&temporary).map_err(|_| FormatError("cannot create imported font"))?;
            file.write_all(&stored)
                .and_then(|()| file.sync_all())
                .map_err(|_| FormatError("cannot write imported font"))?;
            #[cfg_attr(target_arch = "wasm32", allow(clippy::drop_non_drop))]
            drop(file);
            std::fs::rename(temporary, path)
                .map_err(|_| FormatError("cannot publish imported font"))?;
        }
        self.loaded.insert(key.clone(), font);
        Ok(key)
    }
    /// Use the real family name in the game's font field when its encoding permits.
    pub fn name_imported_font(&mut self, name: &[u8], key: &[u8]) -> Result<(), FormatError> {
        // Imported keys are ASCII handles; their resolution ignores the encoding.
        let font = self.face(key, TextEncoding::default())?;
        let directory = self
            .unicode_directory
            .as_ref()
            .ok_or(FormatError("font directory unavailable"))?;
        let path = directory.join(named_key(name));
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, key)
            .map_err(|_| FormatError("cannot save imported font name"))?;
        std::fs::rename(temporary, path)
            .map_err(|_| FormatError("cannot publish imported font name"))?;
        self.loaded.insert(name.to_vec(), font);
        Ok(())
    }
    /// Runtime existence check (0x140007f80): whether a face can be resolved.
    pub fn contains(&mut self, face: &[u8], encoding: TextEncoding) -> bool {
        self.face(face, encoding).is_ok()
    }
    /// Adds a font for characters that neither the selected face nor the default covers.
    pub fn add_fallback(&mut self, font: Arc<FontVec>) {
        self.fallbacks.push(font);
    }
    /// Resolve each character identically for layout and rasterization. Missing or
    /// unusable selected faces fall back to the explicit default supplied by --font,
    /// then to the added fallbacks; a character none covers uses the default.
    pub fn for_character(&mut self, face: &[u8], encoding: TextEncoding, ch: char) -> Arc<FontVec> {
        let covers = |font: &FontVec| {
            let id = font.glyph_id(ch);
            id.0 != 0 && (ch.is_whitespace() || font.outline(id).is_some())
        };
        let font = self
            .face(face, encoding)
            .unwrap_or_else(|_| self.default.clone());
        if covers(&font) {
            return font;
        }
        std::iter::once(&self.default)
            .chain(&self.fallbacks)
            .find(|font| covers(font))
            .unwrap_or(&self.default)
            .clone()
    }
    fn locate(
        &mut self,
        face: &[u8],
        encoding: TextEncoding,
    ) -> Result<Option<(PathBuf, u32)>, FormatError> {
        let name = self.family_name(face, encoding)?;
        let key = name.to_lowercase();
        let index = match &self.index {
            Some(index) => index,
            None => self.index.insert(build_index(&self.directories)),
        };
        Ok(index.get(&key).cloned())
    }
}

/// Decode a legacy face field. The engine's built-in default face is a fixed Shift-JIS
/// constant. Both it and the name returned to script text in the game encoding identify
/// the same default font.
fn legacy_family_name(encoding: TextEncoding, face: &[u8]) -> Result<String, FormatError> {
    if face == DEFAULT_FONT_FACE || face == font_face_for_encoding(DEFAULT_FONT_FACE, encoding) {
        return Ok("ＭＳ ゴシック".to_owned());
    }
    crate::font::codec(encoding)?
        .decode_without_bom_handling_and_without_replacement(face)
        .map(|name| name.into_owned())
        .ok_or(FormatError(match encoding {
            TextEncoding::ShiftJis => "font face is not valid Shift-JIS",
            TextEncoding::Gbk => "font face is not valid GBK",
            TextEncoding::Utf8 => "font face is not valid UTF-8",
            _ => "font face is not valid in the game encoding",
        }))
}

fn imported_key(bytes: &[u8]) -> Vec<u8> {
    format!(
        "CMVS-F-{}",
        &cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(bytes))[..24]
    )
    .into_bytes()
}

fn named_key(face: &[u8]) -> String {
    format!(
        "file-{}",
        &cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(face))[..24]
    )
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
            .as_chunks::<2>()
            .0
            .iter()
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

    #[test]
    #[ignore = "requires CMVS_FONT_PATH (Japanese) and CMVS_PARTIAL_FONT_PATH (Latin-only)"]
    fn missing_characters_fall_back_without_replacing_supported_characters() {
        let load = |key| {
            Arc::new(
                FontVec::try_from_vec(std::fs::read(std::env::var_os(key).unwrap()).unwrap())
                    .unwrap(),
            )
        };
        let default = load("CMVS_FONT_PATH");
        let partial = load("CMVS_PARTIAL_FONT_PATH");
        assert_ne!(default.glyph_id('あ').0, 0);
        assert_eq!(partial.glyph_id('あ').0, 0);
        assert_ne!(partial.glyph_id('A').0, 0);
        let mut library = FontLibrary::with_directories(default.clone(), Vec::new());
        library.loaded.insert(b"partial".to_vec(), partial.clone());
        assert!(Arc::ptr_eq(
            &library.for_character(b"partial", TextEncoding::ShiftJis, 'あ'),
            &default
        ));
        assert!(Arc::ptr_eq(
            &library.for_character(b"partial", TextEncoding::ShiftJis, 'A'),
            &partial
        ));
        assert!(Arc::ptr_eq(
            &library.for_character(b"missing", TextEncoding::ShiftJis, 'あ'),
            &default
        ));
        // A Latin-only default reaches the fallback only for what it lacks.
        let mut chained = FontLibrary::with_directories(partial.clone(), Vec::new());
        chained.add_fallback(default.clone());
        assert!(Arc::ptr_eq(
            &chained.for_character(b"missing", TextEncoding::ShiftJis, 'あ'),
            &default
        ));
        assert!(Arc::ptr_eq(
            &chained.for_character(b"missing", TextEncoding::ShiftJis, 'A'),
            &partial
        ));
        assert!(Arc::ptr_eq(
            &chained.for_character(b"missing", TextEncoding::ShiftJis, '\u{e000}'),
            &partial
        ));
        let directory = std::env::temp_dir().join(format!(
            "cmvs-unicode-font-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        library.set_user_directory(&directory);
        let handle = library.register_unicode("萝莉体").unwrap();
        assert_eq!(handle.len(), 31);
        cmvs_core::settings::SettingsHost::default()
            .set_font_face(&handle)
            .unwrap();
        let mut restarted = FontLibrary::with_directories(default.clone(), Vec::new());
        restarted.set_user_directory(&directory);
        assert_eq!(
            restarted
                .family_name(&handle, TextEncoding::ShiftJis)
                .unwrap(),
            "萝莉体"
        );
        assert_eq!(restarted.register_unicode("萝莉体").unwrap(), handle);
        assert!(
            restarted
                .family_name(b"CMVS-U-../../../../../../../../..", TextEncoding::ShiftJis)
                .is_err()
        );
        std::fs::write(
            directory
                .join("font-faces")
                .join(std::str::from_utf8(&handle).unwrap()),
            "wrong name",
        )
        .unwrap();
        let mut corrupt = FontLibrary::with_directories(default.clone(), Vec::new());
        corrupt.set_user_directory(&directory);
        assert!(
            corrupt
                .family_name(&handle, TextEncoding::ShiftJis)
                .is_err()
        );
        assert!(Arc::ptr_eq(
            &corrupt.for_character(&handle, TextEncoding::ShiftJis, 'あ'),
            &default
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }

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
    fn legacy_face_names_follow_the_game_encoding() {
        let sjis = encoding_rs::SHIFT_JIS.encode("メイリオ").0;
        let gbk = encoding_rs::GBK.encode("黑体").0;
        assert_eq!(
            legacy_family_name(TextEncoding::ShiftJis, &sjis).unwrap(),
            "メイリオ"
        );
        assert_eq!(legacy_family_name(TextEncoding::Gbk, &gbk).unwrap(), "黑体");
        assert_eq!(
            legacy_family_name(TextEncoding::Gbk, b"SimHei").unwrap(),
            "SimHei"
        );
        // 0xff passes the engine's GBK lead-byte test but is not valid GBK.
        assert!(legacy_family_name(TextEncoding::Gbk, b"\xff\xff").is_err());
        assert_eq!(
            legacy_family_name(TextEncoding::Utf8, "微软雅黑".as_bytes()).unwrap(),
            "微软雅黑"
        );
        assert!(legacy_family_name(TextEncoding::Utf8, &sjis).is_err());
        // The fixed default face keeps its Shift-JIS name in every encoding.
        for encoding in [
            TextEncoding::ShiftJis,
            TextEncoding::Gbk,
            TextEncoding::Utf8,
        ] {
            assert_eq!(
                legacy_family_name(encoding, DEFAULT_FONT_FACE).unwrap(),
                "ＭＳ ゴシック"
            );
            let displayed = font_face_for_encoding(DEFAULT_FONT_FACE, encoding);
            assert_eq!(
                legacy_family_name(encoding, displayed).unwrap(),
                "ＭＳ ゴシック"
            );
            assert_eq!(
                crate::font::codec(encoding).unwrap().decode(displayed).0,
                "ＭＳ ゴシック"
            );
        }
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

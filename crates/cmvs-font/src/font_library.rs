//! Face-name font resolution standing in for GDI's CreateFontA lookup. The explicit
//! `--font` file serves the built-in default face; other faces are found by family name
//! in the directories `cmvs_native::font_directories` lists for this platform
//! (Windows/macOS system/user folders and Linux Fontconfig directories).
use ab_glyph::{Font, FontVec};
use cmvs_core::{
    FormatError, TextEncoding,
    format::digest,
    host::settings::{DEFAULT_FONT_FACE, font_face_for_encoding},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FONT: usize = 32 * 1024 * 1024;
const MAX_FONT_FILES: usize = 8192;
const MAX_MISSING_FACES: usize = 1024;
const FACE_NOT_FOUND: FormatError =
    FormatError("font face not found in the system font directories");
const UNREADABLE_NAME: FormatError = FormatError("cannot read Unicode font name");

fn remember_missing(cache: &mut HashSet<Vec<u8>>, face: &[u8]) {
    if cache.len() >= MAX_MISSING_FACES {
        cache.clear();
    }
    cache.insert(face.to_vec());
}

/// Encoding-free state shared by renderers: each caller passes the encoding of the
/// game's legacy font face fields (LOGFONT names), so renderers sharing a library
/// may use different encodings.
pub struct FontLibrary {
    default: Arc<FontVec>,
    /// Tried in order for characters neither the selected face nor the default covers.
    fallbacks: Vec<Arc<FontVec>>,
    /// Faces whose resolution does not depend on the encoding (handles, imported
    /// font names and the built-in default).
    loaded: HashMap<Vec<u8>, Arc<FontVec>>,
    /// References supplied by the host, including those whose source is unavailable.
    referenced: HashMap<Vec<u8>, ReferencedFace>,
    /// System fonts found by decoded family name; the same bytes may name another
    /// family in another encoding.
    located: HashMap<TextEncoding, HashMap<Vec<u8>, Arc<FontVec>>>,
    /// Known-missing face keys per encoding, avoiding repeated SHA-256 hashing and
    /// `ENOENT` probes on every glyph measurement and rasterization.
    missing: HashMap<TextEncoding, HashSet<Vec<u8>>>,
    /// Lowercased family name -> font file and collection index; built on first use.
    index: Option<BTreeMap<String, (PathBuf, u32)>>,
    directories: Vec<PathBuf>,
    /// Unicode names live outside the game's fixed-width legacy font fields.
    unicode_names: HashMap<Vec<u8>, String>,
    /// Known-unresolvable `CMVS-U-...` handles queried through `family_name`.
    missing_unicode: HashSet<Vec<u8>>,
    unicode_directory: Option<PathBuf>,
}

struct ReferencedFace {
    name: String,
    source: Option<(PathBuf, u32)>,
}

impl FontLibrary {
    pub fn new(default: Arc<FontVec>) -> Self {
        Self {
            default,
            fallbacks: Vec::new(),
            loaded: HashMap::new(),
            referenced: HashMap::new(),
            located: HashMap::new(),
            missing: HashMap::new(),
            index: None,
            directories: cmvs_native::font_directories(),
            unicode_names: HashMap::new(),
            missing_unicode: HashSet::new(),
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
    fn invalidate_missing(&mut self) {
        self.missing.clear();
        self.missing_unicode.clear();
    }
    pub(crate) fn bind_face_reference(
        &mut self,
        key: Vec<u8>,
        name: String,
        source: Option<(PathBuf, u32)>,
    ) {
        self.loaded.remove(&key);
        self.referenced.insert(key, ReferencedFace { name, source });
    }
    pub fn set_user_directory(&mut self, directory: &Path) {
        let directory = directory.join("font-faces");
        if self.unicode_directory.as_ref() != Some(&directory) {
            // User aliases take precedence over both imported and system faces.
            self.loaded.clear();
            self.located.clear();
            if self.unicode_directory.is_some() {
                self.unicode_names.clear();
            }
            self.unicode_directory = Some(directory);
        }
        self.invalidate_missing();
    }
    /// A stable ASCII handle fits the VM/CSV2 LOGFONT field. The actual family
    /// name stays Unicode, including across restarts, in an immutable UTF-8 file.
    pub fn register_unicode(&mut self, name: &str) -> Result<Vec<u8>, FormatError> {
        if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
            return Err(FormatError("invalid Unicode font family name"));
        }
        let key = format!(
            "CMVS-U-{}",
            &digest::to_hex(&digest::sha256(name.as_bytes()))[..24]
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
        self.invalidate_missing();
        Ok(key)
    }
    pub fn family_name(
        &mut self,
        face: &[u8],
        encoding: TextEncoding,
    ) -> Result<String, FormatError> {
        if let Some(reference) = self.referenced.get(face) {
            return Ok(reference.name.clone());
        }
        if let Some(name) = self.unicode_names.get(face) {
            return Ok(name.clone());
        }
        if face.starts_with(b"CMVS-U-") {
            if self.missing_unicode.contains(face) {
                return Err(UNREADABLE_NAME);
            }
            return self.resolve_unicode_name(face);
        }
        legacy_family_name(encoding, face)
    }
    fn resolve_unicode_name(&mut self, face: &[u8]) -> Result<String, FormatError> {
        if face.len() != 31 || !face[7..].iter().all(u8::is_ascii_hexdigit) {
            return Err(FormatError("invalid Unicode font handle"));
        }
        let directory = self
            .unicode_directory
            .as_ref()
            .ok_or(FormatError("Unicode font mapping is unavailable"))?;
        let mut bytes = Vec::new();
        let file = File::open(directory.join(std::str::from_utf8(face).expect("ASCII handle")))
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    remember_missing(&mut self.missing_unicode, face);
                }
                UNREADABLE_NAME
            })?;
        file.take(513)
            .read_to_end(&mut bytes)
            .map_err(|_| UNREADABLE_NAME)?;
        let name = String::from_utf8(bytes)
            .map_err(|_| FormatError("font family name is not valid UTF-8"))?;
        if name.is_empty()
            || name.len() > 512
            || name.chars().any(char::is_control)
            || format!(
                "CMVS-U-{}",
                &digest::to_hex(&digest::sha256(name.as_bytes()))[..24]
            )
            .as_bytes()
                != face
        {
            return Err(FormatError("invalid stored Unicode font name"));
        }
        self.unicode_names.insert(face.to_vec(), name.clone());
        Ok(name)
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
        if let Some(reference) = self.referenced.get(face) {
            let font = reference
                .source
                .as_ref()
                .and_then(|(path, index)| read_font(path, *index).ok())
                .unwrap_or_else(|| self.default.clone());
            self.loaded.insert(face.to_vec(), font.clone());
            return Ok(font);
        }
        if let Some(font) = self.located.get(&encoding).and_then(|m| m.get(face)) {
            return Ok(font.clone());
        }
        if self
            .missing
            .get(&encoding)
            .is_some_and(|m| m.contains(face))
        {
            return Err(FACE_NOT_FOUND);
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
            self.located
                .entry(encoding)
                .or_default()
                .insert(face.to_vec(), self.default.clone());
            return Ok(self.default.clone());
        }
        let Some((path, index)) = self.locate(face, encoding)? else {
            remember_missing(self.missing.entry(encoding).or_default(), face);
            return Err(FACE_NOT_FOUND);
        };
        let font = read_font(&path, index)?;
        self.located
            .entry(encoding)
            .or_default()
            .insert(face.to_vec(), font.clone());
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
        self.invalidate_missing();
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
        for by_encoding in self.located.values_mut() {
            by_encoding.remove(name);
        }
        self.loaded.insert(name.to_vec(), font);
        self.invalidate_missing();
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
        self.resolve_character(face, encoding, ch).0
    }
    /// Returns the glyph face, the primary face for baseline placement, and cacheability.
    pub(crate) fn resolve_character(
        &mut self,
        face: &[u8],
        encoding: TextEncoding,
        ch: char,
    ) -> (Arc<FontVec>, Arc<FontVec>, bool) {
        let covers = |font: &FontVec| {
            let id = font.glyph_id(ch);
            id.0 != 0 && (ch.is_whitespace() || font.outline(id).is_some())
        };
        let (font, cacheable) = match self.face(face, encoding) {
            Ok(font) => (font, true),
            Err(err) => (
                self.default.clone(),
                err == FACE_NOT_FOUND
                    || (err == UNREADABLE_NAME && self.missing_unicode.contains(face)),
            ),
        };
        if covers(&font) {
            return (font.clone(), font, cacheable);
        }
        let fallback = std::iter::once(&self.default)
            .chain(&self.fallbacks)
            .find(|font| covers(font))
            .unwrap_or(&self.default);
        (fallback.clone(), font, cacheable)
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

fn read_font(path: &Path, index: u32) -> Result<Arc<FontVec>, FormatError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_FONT as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| FormatError("located font file cannot be read"))?;
    if bytes.len() > MAX_FONT {
        return Err(FormatError("font exceeds 32 MiB research limit"));
    }
    FontVec::try_from_vec_and_index(bytes, index)
        .map(Arc::new)
        .map_err(|_| FormatError("located font is not a supported outline font"))
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
    format!("CMVS-F-{}", &digest::to_hex(&digest::sha256(bytes))[..24]).into_bytes()
}

fn named_key(face: &[u8]) -> String {
    format!("file-{}", &digest::to_hex(&digest::sha256(face))[..24])
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
pub(crate) mod tests {
    use super::*;

    #[test]
    fn unavailable_font_reference_uses_default_and_retains_its_name() {
        let default = Arc::new(FontVec::try_from_vec(minimal_ttf("Default")).unwrap());
        let mut library = FontLibrary::with_directories(default.clone(), Vec::new());
        let key = b"CMVS-R-012345678901234567890123";
        library.bind_face_reference(key.to_vec(), "Saved face".into(), None);
        let encoding = TextEncoding::default();
        assert!(library.contains(key, encoding));
        assert!(Arc::ptr_eq(
            &library.for_character(key, encoding, 'A'),
            &default
        ));
        assert_eq!(library.family_name(key, encoding).unwrap(), "Saved face");
    }

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
        cmvs_core::host::settings::SettingsHost::default()
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

    fn name_table(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
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
        name
    }

    /// One sfnt face whose only table is `name`, holding the given (platform, encoding,
    /// name ID, UTF-16 string) records.
    fn face(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let name = name_table(records);
        let mut font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        font.extend(b"name");
        font.extend(0u32.to_be_bytes());
        font.extend(28u32.to_be_bytes());
        font.extend((name.len() as u32).to_be_bytes());
        font.extend(name);
        font
    }

    /// Minimal self-contained TrueType `sfnt` accepted by `ab_glyph::FontVec`, mapping
    /// `ch` to a rectangular outline glyph `(0, 0)..(xmax, ymax)` so tests run offline.
    pub(crate) fn minimal_ttf_with_box(family: &str, ch: u16, xmax: i16, ymax: i16) -> Vec<u8> {
        minimal_ttf_with_metrics(family, ch, xmax, ymax, [800, -200])
    }

    pub(crate) fn minimal_ttf_with_metrics(
        family: &str,
        ch: u16,
        xmax: i16,
        ymax: i16,
        [ascent, descent]: [i16; 2],
    ) -> Vec<u8> {
        minimal_ttf_with_layout(family, ch, [0, 0, xmax, ymax], [ascent, descent], None)
    }

    pub(crate) fn minimal_ttf_with_layout(
        family: &str,
        ch: u16,
        [xmin, ymin, xmax, ymax]: [i16; 4],
        [ascent, descent]: [i16; 2],
        os2_typo: Option<[i16; 2]>,
    ) -> Vec<u8> {
        let mut cmap = Vec::new();
        for v in [0u16, 1] {
            cmap.extend(v.to_be_bytes());
        }
        cmap.extend(3u16.to_be_bytes());
        cmap.extend(1u16.to_be_bytes());
        cmap.extend(12u32.to_be_bytes());
        let id_delta = (1i32 - i32::from(ch)) as i16 as u16;
        for v in [
            4u16, 32, 0, 4, 4, 1, 0, ch, 0xffff, 0, ch, 0xffff, id_delta, 1, 0, 0,
        ] {
            cmap.extend(v.to_be_bytes());
        }

        let mut glyf = Vec::new();
        for v in [1i16, xmin, ymin, xmax, ymax] {
            glyf.extend(v.to_be_bytes());
        }
        glyf.extend(3u16.to_be_bytes());
        glyf.extend(0u16.to_be_bytes());
        glyf.extend([0x01u8; 4]);
        let dx = xmax - xmin;
        let dy = ymax - ymin;
        for v in [xmin, dx, 0, -dx, ymin, 0, dy, 0] {
            glyf.extend(v.to_be_bytes());
        }

        let mut head = vec![0u8; 54];
        head[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[38..40].copy_from_slice(&1000i16.to_be_bytes());
        head[40..42].copy_from_slice(&1000i16.to_be_bytes());
        head[46..48].copy_from_slice(&8u16.to_be_bytes());
        head[48..50].copy_from_slice(&2i16.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&ascent.to_be_bytes());
        hhea[6..8].copy_from_slice(&descent.to_be_bytes());
        hhea[10..12].copy_from_slice(&1000u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());

        let mut hmtx = Vec::new();
        for (adv, lsb) in [(500u16, 0i16), (600u16, 0i16)] {
            hmtx.extend(adv.to_be_bytes());
            hmtx.extend(lsb.to_be_bytes());
        }

        let mut loca = Vec::new();
        for off in [0u16, 0, (glyf.len() / 2) as u16] {
            loca.extend(off.to_be_bytes());
        }

        let mut maxp = vec![0u8; 32];
        maxp[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
        maxp[6..8].copy_from_slice(&4u16.to_be_bytes());
        maxp[8..10].copy_from_slice(&1u16.to_be_bytes());

        let name = name_table(&[(3, 1, 1, family)]);
        let mut tables: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
        if let Some([typo_asc, typo_desc]) = os2_typo {
            let mut os2 = vec![0u8; 78];
            os2[68..70].copy_from_slice(&typo_asc.to_be_bytes());
            os2[70..72].copy_from_slice(&typo_desc.to_be_bytes());
            tables.push((b"OS/2", os2));
        }
        tables.extend([
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"loca", loca),
            (b"maxp", maxp),
            (b"name", name),
        ]);

        let num_tables = tables.len() as u16;
        let entry_selector = num_tables.ilog2() as u16;
        let search_range = (1u16 << entry_selector) * 16;
        let range_shift = num_tables * 16 - search_range;

        let mut sfnt = Vec::new();
        sfnt.extend(0x0001_0000u32.to_be_bytes());
        sfnt.extend(num_tables.to_be_bytes());
        sfnt.extend(search_range.to_be_bytes());
        sfnt.extend(entry_selector.to_be_bytes());
        sfnt.extend(range_shift.to_be_bytes());

        let mut offset = 12 + 16 * tables.len();
        let mut payload = Vec::new();
        for (tag, data) in &tables {
            sfnt.extend_from_slice(*tag);
            sfnt.extend(0u32.to_be_bytes());
            sfnt.extend((offset as u32).to_be_bytes());
            sfnt.extend((data.len() as u32).to_be_bytes());
            payload.extend_from_slice(data);
            let pad = (4 - data.len() % 4) % 4;
            payload.resize(payload.len() + pad, 0);
            offset += data.len() + pad;
        }
        sfnt.extend(payload);
        sfnt
    }

    pub(crate) fn minimal_ttf(family: &str) -> Vec<u8> {
        minimal_ttf_with_box(family, 0x0041, 500, 700)
    }

    /// Rebases each face's table offsets into a `ttcf` collection.
    pub(crate) fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"ttcf".to_vec();
        out.extend([0, 1, 0, 0]);
        out.extend((faces.len() as u32).to_be_bytes());
        let mut at = 12 + 4 * faces.len();
        let mut bodies = Vec::new();
        for face in faces {
            out.extend((at as u32).to_be_bytes());
            let mut face = face.clone();
            let tables = usize::from(u16::from_be_bytes([face[4], face[5]]));
            for i in 0..tables {
                let off_at = 12 + 16 * i + 8;
                let table =
                    u32::from_be_bytes(face[off_at..off_at + 4].try_into().unwrap()) as usize + at;
                face[off_at..off_at + 4].copy_from_slice(&(table as u32).to_be_bytes());
            }
            at += face.len();
            bodies.push(face);
        }
        out.extend(bodies.concat());
        out
    }

    #[test]
    fn missing_face_hits_negative_cache_and_clears_when_later_added() {
        let root = std::env::temp_dir().join(format!(
            "cmvs-font-neg-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sys_dir = root.join("system");
        let user_dir = root.join("user");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::create_dir_all(user_dir.join("font-faces")).unwrap();

        let default_font = Arc::new(FontVec::try_from_vec(minimal_ttf("DefaultTest")).unwrap());
        let unicode_bytes = minimal_ttf("UnicodeTarget");
        std::fs::write(sys_dir.join("unicode.ttf"), &unicode_bytes).unwrap();

        let mut library = FontLibrary::with_directories(default_font.clone(), vec![sys_dir]);
        library.set_user_directory(&user_dir);

        let missing_sjis = encoding_rs::SHIFT_JIS.encode("ＭＳ 明朝").0.into_owned();
        assert!(library.face(&missing_sjis, TextEncoding::ShiftJis).is_err());
        assert!(
            library
                .missing
                .get(&TextEncoding::ShiftJis)
                .is_some_and(|m| m.contains(missing_sjis.as_slice()))
        );

        // Plant a corrupt mapping file directly on disk without going through
        // `FontLibrary`: a negative-cache hit must return immediately from memory
        // without calling `File::open` on `font-faces/file-...`.
        let raw_mapping = user_dir.join("font-faces").join(named_key(&missing_sjis));
        std::fs::write(&raw_mapping, b"corrupt-on-disk").unwrap();
        assert_eq!(
            library
                .face(&missing_sjis, TextEncoding::ShiftJis)
                .unwrap_err()
                .0,
            "font face not found in the system font directories",
            "second lookup must hit negative cache rather than re-reading disk"
        );
        assert!(Arc::ptr_eq(
            &library.for_character(&missing_sjis, TextEncoding::ShiftJis, 'A'),
            &default_font
        ));
        std::fs::remove_file(&raw_mapping).unwrap();

        // Importing and naming that face clears the negative cache so subsequent
        // lookups resolve to the newly added font.
        let added_bytes = minimal_ttf("ＭＳ 明朝");
        let handle = library.import_file(added_bytes, 0).unwrap();
        library.name_imported_font(&missing_sjis, &handle).unwrap();
        assert!(
            !library
                .missing
                .get(&TextEncoding::ShiftJis)
                .is_some_and(|m| m.contains(missing_sjis.as_slice()))
        );
        let resolved = library.face(&missing_sjis, TextEncoding::ShiftJis).unwrap();
        assert!(!Arc::ptr_eq(&resolved, &default_font));
        assert!(library.contains(&missing_sjis, TextEncoding::ShiftJis));
        assert!(Arc::ptr_eq(
            &library.for_character(&missing_sjis, TextEncoding::ShiftJis, 'A'),
            &resolved
        ));

        // Also verify negative caching and invalidation when `register_unicode` adds
        // a previously missing `CMVS-U-...` handle.
        let unicode_handle = format!(
            "CMVS-U-{}",
            &digest::to_hex(&digest::sha256("UnicodeTarget".as_bytes()))[..24]
        )
        .into_bytes();
        assert!(
            library
                .face(&unicode_handle, TextEncoding::ShiftJis)
                .is_err()
        );
        assert!(library.missing_unicode.contains(unicode_handle.as_slice()));
        assert_eq!(
            library.register_unicode("UnicodeTarget").unwrap(),
            unicode_handle
        );
        assert!(library.missing_unicode.is_empty());
        assert!(
            library
                .face(&unicode_handle, TextEncoding::ShiftJis)
                .is_ok()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_font_and_name_reads_can_be_retried() {
        let root = std::env::temp_dir().join(format!("cmvs-font-retry-{}", std::process::id()));
        let default = Arc::new(FontVec::try_from_vec(minimal_ttf("Default")).unwrap());
        let mut writer = FontLibrary::with_directories(default.clone(), Vec::new());
        writer.set_user_directory(&root);
        let name = writer.register_unicode("Test Face").unwrap();
        let font = writer.import_file(minimal_ttf("Test Face"), 0).unwrap();
        writer.name_imported_font(&name, &font).unwrap();

        let mut reader = FontLibrary::with_directories(default, Vec::new());
        reader.set_user_directory(&root);
        let encoding = TextEncoding::ShiftJis;
        // A directory at the expected path fails to read even for privileged test runners.
        for handle in [&name, &font] {
            let path = root
                .join("font-faces")
                .join(std::str::from_utf8(handle).unwrap());
            let backup = path.with_extension("saved");
            std::fs::rename(&path, &backup).unwrap();
            std::fs::create_dir(&path).unwrap();
            if handle == &name {
                assert!(reader.family_name(&name, encoding).is_err());
            } else {
                assert!(reader.face(&name, encoding).is_err());
            }
            std::fs::remove_dir(&path).unwrap();
            std::fs::rename(backup, path).unwrap();
        }
        assert_eq!(reader.family_name(&name, encoding).unwrap(), "Test Face");
        assert!(reader.face(&name, encoding).is_ok());
        std::fs::remove_dir_all(root).unwrap();
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

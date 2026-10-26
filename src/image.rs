//! Read-only image lookup and decoding for 0x2030, following the image manager (+4760):
//! 0x140036820 tries the ten 0x2011 entries in slot order and takes the first hit.
use crate::{
    FormatError, MAX_PAYLOAD, cpz::Cpz6Index, pb3, platform::resolve_under, raster::RgbaImage,
};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// 0x2011 / 0x1400717c0 accepts slots below 10 (image manager +4232, stride 2056).
pub const IMAGE_ENTRIES: u32 = 10;

pub trait ImageSource {
    /// Raw bytes from the first entry holding `name`, or Ok(None) when no entry does
    /// (the original then shows a MessageBox).
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
}

struct OpenArchive {
    path: Vec<u8>,
    file: File,
    index: Cpz6Index,
}

/// Archives open lazily per entry slot. A slot registered again with another path is
/// reopened, as 0x140035b00 destroys the old archive object.
pub struct ImageArchives {
    root: PathBuf,
    open: BTreeMap<u32, OpenArchive>,
}

impl ImageArchives {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err("image root is not a directory".into());
        }
        Ok(Self {
            root,
            open: BTreeMap::new(),
        })
    }

    /// Binds the 0x2011 registrations recorded by `Bootstrap::paths`.
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Registered<'a> {
        Registered {
            archives: self,
            paths,
            command: 0x2011,
            entries: IMAGE_ENTRIES,
        }
    }
    /// Shared bounded directory/CPZ lookup, also used by the four-slot audio roots.
    #[cfg(feature = "audio-output")]
    pub(crate) fn with_entries<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
        command: u16,
        entries: u32,
    ) -> Registered<'a> {
        Registered {
            archives: self,
            paths,
            command,
            entries,
        }
    }

    /// Directory entry: 0x1400087c0 reads root + entry path + name; a missing file is a miss.
    fn read_file(&self, path: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let Some(path) = resolve_present(&self.root, path)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_PAYLOAD as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PAYLOAD {
            return Err("image file exceeds research limit".into());
        }
        Ok(Some(bytes))
    }

    /// Archive entry: an archive that cannot be opened is a miss, a malformed one an error
    /// (the original exits). Resources are checksum-verified as with PACK_DATA_CHECK=1.
    fn read_archive(
        &mut self,
        slot: u32,
        path: &[u8],
        name: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        if self.open.get(&slot).is_none_or(|open| open.path != path) {
            self.open.remove(&slot);
            let Some(resolved) = resolve_present(&self.root, path)? else {
                return Ok(None);
            };
            let mut file = File::open(resolved)?;
            let index = Cpz6Index::read(&mut file)?;
            self.open.insert(
                slot,
                OpenArchive {
                    path: path.to_vec(),
                    file,
                    index,
                },
            );
        }
        let open = self.open.get_mut(&slot).expect("opened above");
        let Some(entry) = open.index.lookup(name)? else {
            return Ok(None);
        };
        Ok(Some(open.index.read_entry_verified(&mut open.file, entry)?))
    }
}

fn resolve_present(root: &Path, name: &[u8]) -> Result<Option<PathBuf>, Box<dyn Error>> {
    match resolve_under(root, name) {
        Ok(path) => Ok(Some(path)),
        Err(e)
            if e.downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub struct Registered<'a> {
    archives: &'a mut ImageArchives,
    paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    command: u16,
    entries: u32,
}

impl ImageSource for Registered<'_> {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        for slot in 0..self.entries {
            let Some(path) = self
                .paths
                .get(&(self.command, slot))
                .filter(|p| !p.is_empty())
            else {
                continue;
            };
            // A trailing backslash marks a directory; anything else is an archive.
            let hit = if path.ends_with(b"\\") {
                self.archives.read_file(&[path.as_slice(), name].concat())?
            } else {
                self.archives.read_archive(slot, path, name)?
            };
            if hit.is_some() {
                return Ok(hit);
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub pixels: RgbaImage,
    /// Image info +24 (object image +1152): the source has 32 bits per pixel.
    pub alpha: bool,
}

/// 0x140036700 picks the codec by magic; only PB3B type 1 decodes here, other known codecs
/// are explicit errors. Ok(None) is the original quiet failure for an unknown magic or a
/// PB3 type outside 1..8: the load returns 0 without a MessageBox.
pub fn decode(bytes: &[u8]) -> Result<Option<DecodedImage>, FormatError> {
    let unsupported = FormatError("image codec not implemented; only PB3 type 1 is supported");
    let magic = bytes
        .get(..4)
        .ok_or(FormatError("image file shorter than its 4-byte magic"))?;
    match magic {
        b"PB3B" => {
            let header = pb3::Header::parse(bytes)?;
            match header.kind {
                1 => Ok(Some(DecodedImage {
                    pixels: pb3::decode(bytes)?,
                    alpha: header.bpp == 32,
                })),
                2..=8 => Err(unsupported),
                _ => Ok(None),
            }
        }
        [0x89, b'P', b'N', b'G'] | b"MSK0" | b"PB2A" | [b'B', b'M', ..] => Err(unsupported),
        _ => Ok(None),
    }
}

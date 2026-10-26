//! Bounded read-only directory/CPZ lookup shared by image and audio backends.
//! Slot order, case folding, checksums and path confinement follow the existing reader.
use crate::{MAX_PAYLOAD, cpz::Cpz6Index, platform::resolve_under};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Verified engine registration families. Images have ten slots; SE/BGM have four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Image,
    SoundEffect,
    Music,
}
impl ResourceKind {
    pub const fn command(self) -> u16 {
        match self {
            Self::Image => 0x2011,
            Self::SoundEffect => 0x2012,
            Self::Music => 0x2013,
        }
    }
    pub const fn slot_count(self) -> u32 {
        match self {
            Self::Image => 10,
            Self::SoundEffect | Self::Music => 4,
        }
    }
}
pub trait ResourceSource {
    /// Return the first hit, None for missing resources, or an explicit format/I/O error.
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
}

struct OpenArchive {
    path: Vec<u8>,
    file: File,
    index: Cpz6Index,
}

/// Archives open lazily per entry slot. A slot registered again with another path is
/// reopened, as 0x140035b00 destroys the old archive object.
pub struct ResourceArchives {
    root: PathBuf,
    open: BTreeMap<(u16, u32), OpenArchive>,
}

impl ResourceArchives {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err("resource root is not a directory".into());
        }
        Ok(Self {
            root,
            open: BTreeMap::new(),
        })
    }

    /// Bind the explicit registrations for one engine resource family.
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
        kind: ResourceKind,
    ) -> RegisteredResources<'a> {
        RegisteredResources {
            archives: self,
            paths,
            command: kind.command(),
            entries: kind.slot_count(),
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
            return Err("resource file exceeds research limit".into());
        }
        Ok(Some(bytes))
    }

    /// Archive entry: an archive that cannot be opened is a miss, a malformed one an error
    /// (the original exits). Resources are checksum-verified as with PACK_DATA_CHECK=1.
    fn read_archive(
        &mut self,
        command: u16,
        slot: u32,
        path: &[u8],
        name: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        if self
            .open
            .get(&(command, slot))
            .is_none_or(|open| open.path != path)
        {
            self.open.remove(&(command, slot));
            let Some(resolved) = resolve_present(&self.root, path)? else {
                return Ok(None);
            };
            let mut file = File::open(resolved)?;
            let index = Cpz6Index::read(&mut file)?;
            self.open.insert(
                (command, slot),
                OpenArchive {
                    path: path.to_vec(),
                    file,
                    index,
                },
            );
        }
        let open = self.open.get_mut(&(command, slot)).expect("opened above");
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

pub struct RegisteredResources<'a> {
    archives: &'a mut ResourceArchives,
    paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    command: u16,
    entries: u32,
}

impl ResourceSource for RegisteredResources<'_> {
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
                self.archives.read_archive(self.command, slot, path, name)?
            };
            if hit.is_some() {
                return Ok(hit);
            }
        }
        Ok(None)
    }
}

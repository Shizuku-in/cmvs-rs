//! Shared resource lookup across registered directories and CPZ archives.
//!
//! Create [`ResourceArchives`] for a game root, then use [`ResourceArchives::with_paths`]
//! to bind one [`ResourceKind`] to its registered slots. The returned
//! [`RegisteredResources`] implements [`ResourceSource`]. Slots are searched in order:
//! the first hit wins, a missing file is `Ok(None)`, and malformed data or I/O failures
//! are errors. Paths must stay inside the game root.
use crate::filesystem as fs;
use crate::filesystem::File;
use crate::{
    MAX_PAYLOAD,
    cpz::{ArchiveIndex, Cpz7Parameters},
    platform::resolve_under,
};
use std::{
    collections::BTreeMap,
    error::Error,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Resource families with independent registration slots and archive caches.
///
/// Images have ten slots; each other family has four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// Scene and interface images.
    Image,
    /// Short sound effects.
    SoundEffect,
    /// Background music.
    Music,
    // Engine +4808, registered by 0x2014 (0x140071b30); 0x2820-byte list class.
    /// Spoken dialogue audio.
    Voice,
    // Engine +4824, 0x201a / 0x140071c40.
    /// Movies bound to scene objects.
    SceneMovie,
}
impl ResourceKind {
    /// System command that registers paths for this family.
    pub const fn command(self) -> u16 {
        match self {
            Self::Image => 0x2011,
            Self::SoundEffect => 0x2012,
            Self::Music => 0x2013,
            Self::Voice => 0x2014,
            Self::SceneMovie => 0x201a,
        }
    }
    /// Further tables searched after the primary one. cmvs-20070119 keeps stand images in
    /// a separate 0x2017 table; only its hash-bound variant registers that table.
    const fn extra_tables(self) -> &'static [(u16, u32)] {
        match self {
            Self::Image => &[(0x2017, 10)],
            _ => &[],
        }
    }
    /// Number of slots in the family's primary registration table.
    pub const fn slot_count(self) -> u32 {
        match self {
            Self::Image => 10,
            Self::SoundEffect | Self::Music | Self::Voice | Self::SceneMovie => 4,
        }
    }
}
/// Read raw resource bytes by their encoded resource name.
pub trait ResourceSource {
    /// Return the first hit, None for missing resources, or an explicit format/I/O error.
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
}

struct OpenArchive {
    path: Vec<u8>,
    file: File,
    index: ArchiveIndex,
}

/// Read-only resource access rooted in one game directory.
///
/// Archives open lazily and are cached by family and slot. Changing a registered
/// path or its CPZ7 parameters invalidates the cached archive for that slot.
// 0x140035b00 destroys the old archive object when a slot path changes.
pub struct ResourceArchives {
    root: PathBuf,
    open: BTreeMap<(u16, u32), OpenArchive>,
    parameters: BTreeMap<(u16, u32), Cpz7Parameters>,
}

impl ResourceArchives {
    /// Canonicalize an existing game directory without opening any archives.
    ///
    /// Returns an error if the root cannot be resolved or is not a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let root = fs::canonicalize(root.as_ref())?;
        if !fs::is_dir(&root) {
            return Err("resource root is not a directory".into());
        }
        Ok(Self {
            root,
            open: BTreeMap::new(),
            parameters: BTreeMap::new(),
        })
    }

    /// Reopen a slot when its keys change, even if its path stays the same.
    pub fn configure_parameters(&mut self, parameters: &BTreeMap<(u16, u32), Cpz7Parameters>) {
        self.open
            .retain(|slot, _| self.parameters.get(slot) == parameters.get(slot));
        self.parameters.clone_from(parameters);
    }

    /// Bind one resource family to a view of the current path registrations.
    ///
    /// `paths` maps `(registration command, slot)` to a game-relative path. The
    /// view borrows both the cache and the table; use [`ResourceSource::read`]
    /// to search it. Binding alone performs no I/O.
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
            extra: kind.extra_tables(),
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
            let parameters = self
                .parameters
                .get(&(command, slot))
                .copied()
                .unwrap_or_default();
            let index = ArchiveIndex::read_with_cpz7_parameters(&mut file, parameters)?;
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

/// A borrowed resource-family view returned by [`ResourceArchives::with_paths`].
pub struct RegisteredResources<'a> {
    archives: &'a mut ResourceArchives,
    paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    command: u16,
    entries: u32,
    extra: &'static [(u16, u32)],
}

impl ResourceSource for RegisteredResources<'_> {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let tables =
            std::iter::once((self.command, self.entries)).chain(self.extra.iter().copied());
        for (command, entries) in tables {
            for slot in 0..entries {
                let Some(path) = self.paths.get(&(command, slot)).filter(|p| !p.is_empty()) else {
                    continue;
                };
                // A trailing backslash marks a directory; anything else is an archive.
                let hit = if path.ends_with(b"\\") {
                    self.archives.read_file(&[path.as_slice(), name].concat())?
                } else {
                    self.archives.read_archive(command, slot, path, name)?
                };
                if hit.is_some() {
                    return Ok(hit);
                }
            }
        }
        Ok(None)
    }
}

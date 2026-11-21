//! Explicit user-directory storage policy, independent of original CMVS file replacement.
use super::{Css1Save, FileSystemSave, SystemSaveSink, SystemSaveSource};
use crate::filesystem::{File, HostFs, OpenOptions};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn regular_file(fs: &dyn HostFs, path: &Path) -> io::Result<bool> {
    match fs.symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::other(
            "profile entries must be regular files, not links or directories",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn external_directory(fs: &dyn HostFs, path: &Path, game_root: &Path) -> io::Result<PathBuf> {
    let game_root = fs.canonicalize(game_root)?;
    let mut ancestor = path.to_path_buf();
    let mut missing = Vec::new();
    while !fs.try_exists(&ancestor)? {
        let name = ancestor
            .file_name()
            .ok_or_else(|| io::Error::other("profile path needs an existing ancestor"))?
            .to_owned();
        missing.push(name);
        ancestor = ancestor
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_owned();
    }
    let mut resolved = fs.canonicalize(&ancestor)?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    if resolved.starts_with(&game_root) {
        return Err(io::Error::other(
            "user data directory must be outside the read-only game root",
        ));
    }
    fs.create_dir_all(&resolved)?;
    let resolved = fs.canonicalize(&resolved)?;
    if resolved.starts_with(game_root) {
        return Err(io::Error::other(
            "user data directory resolves inside the read-only game root",
        ));
    }
    Ok(resolved)
}

struct StagedFile {
    fs: Arc<dyn HostFs>,
    path: PathBuf,
    committed: bool,
}
impl StagedFile {
    fn commit(mut self, target: &Path) -> io::Result<()> {
        self.fs.replace(&self.path, target)?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.fs.remove_file(&self.path);
        }
    }
}

/// One session owns a host directory lock handle; dropping releases it.
/// Hosts without competing sessions (single-app consoles, browser workers) may mark
/// the directory without OS locking.
pub struct ProfileSystemSave {
    fs: Arc<dyn HostFs>,
    directory: PathBuf,
    css0: Option<super::PersistentStarts>,
    _lock: File,
}
impl ProfileSystemSave {
    pub fn open_with_fs(
        directory: &Path,
        game_root: &Path,
        fs: Arc<dyn HostFs>,
    ) -> io::Result<Self> {
        let directory = external_directory(&*fs, directory, game_root)?;
        Self::open_directory(directory, fs, false)
    }

    #[cfg(feature = "std-fs")]
    pub fn open(directory: &Path, game_root: &Path) -> io::Result<Self> {
        Self::open_with_fs(directory, game_root, crate::filesystem::default_fs())
    }

    /// Explicitly opt into the game's own savedata directory using `fs`. Other game files
    /// remain read-only, and a symlink may not redirect writes into resources.
    pub fn open_game_savedata_with_fs(game_root: &Path, fs: Arc<dyn HostFs>) -> io::Result<Self> {
        Self::game_directory(game_root, fs, false)
    }

    #[cfg(feature = "std-fs")]
    pub fn open_for_restore(directory: &Path, game_root: &Path, local: bool) -> io::Result<Self> {
        let fs = crate::filesystem::default_fs();
        if local {
            match Self::game_directory(game_root, fs.clone(), true) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                    ) => {}
                result => return result,
            }
        }
        Self::open_directory(external_directory(&*fs, directory, game_root)?, fs, true)
    }

    fn game_directory(game_root: &Path, fs: Arc<dyn HostFs>, repair: bool) -> io::Result<Self> {
        let game_root = fs.canonicalize(game_root)?;
        let directory = game_root.join("savedata");
        match fs.create_dir_all(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs.symlink_metadata(&directory)?;
        if !fs.is_dir(&directory) || metadata.file_type().is_symlink() {
            return Err(io::Error::other("savedata must be a real directory"));
        }
        let directory = fs.canonicalize(&directory)?;
        if directory.parent() != Some(game_root.as_path()) {
            return Err(io::Error::other(
                "savedata resolves outside the game directory",
            ));
        }
        let profile = Self::open_directory(directory, fs, repair)?;
        // Opening an existing lock alone does not prove that saves can be
        // created or atomically replaced in this directory.
        drop(profile.stage(&[])?);
        Ok(profile)
    }

    /// Explicitly opt into the game's own savedata directory. Other game files
    /// remain read-only, and a symlink may not redirect writes into resources.
    #[cfg(feature = "std-fs")]
    pub fn open_game_savedata(game_root: &Path) -> io::Result<Self> {
        Self::open_game_savedata_with_fs(game_root, crate::filesystem::default_fs())
    }

    /// Lock an existing application-owned save directory for a read-only export.
    #[cfg(feature = "std-fs")]
    pub fn lock_for_export(directory: &Path) -> io::Result<File> {
        let fs = crate::filesystem::default_fs();
        if !fs.symlink_metadata(directory)?.is_dir() {
            return Err(io::Error::other("expected a save directory"));
        }
        Self::lock_directory(directory, &*fs)
    }

    fn lock_directory(directory: &Path, fs: &dyn HostFs) -> io::Result<File> {
        let lock_path = directory.join(".cmvs-system.lock");
        if regular_file(fs, &lock_path)? && fs.metadata(&lock_path)?.len() != 0 {
            return Err(io::Error::other("profile lock file must be empty"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open_with(fs, &lock_path)?;
        if lock.metadata()?.len() != 0 {
            return Err(io::Error::other("profile lock file must be empty"));
        }
        lock.try_lock().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("cannot lock user data directory: {error}"),
            )
        })?;
        Ok(lock)
    }

    fn open_directory(directory: PathBuf, fs: Arc<dyn HostFs>, repair: bool) -> io::Result<Self> {
        let lock = Self::lock_directory(&directory, &*fs)?;
        let profile = Self {
            fs,
            directory,
            css0: None,
            _lock: lock,
        };
        let primary = regular_file(&*profile.fs, &profile.path())?;
        let backup = regular_file(&*profile.fs, &profile.backup_path())?;
        if !repair && !primary && backup {
            return Err(io::Error::other(
                "profile system.dat is missing but system.bak exists; restore the backup explicitly",
            ));
        }
        Ok(profile)
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn fs(&self) -> &Arc<dyn HostFs> {
        &self.fs
    }
    pub fn is_css0(&self) -> bool {
        self.css0.is_some()
    }
    /// Selects CSS0 (`save000.dat` / `save000.bak`) validation when `starts` is `Some`.
    pub fn set_css0(&mut self, starts: Option<super::PersistentStarts>) -> io::Result<()> {
        let first_css0 = self.css0.is_none() && starts.is_some();
        self.css0 = starts;
        if first_css0 {
            let primary = regular_file(&*self.fs, &self.path())?;
            let backup = regular_file(&*self.fs, &self.backup_path())?;
            if !primary && backup {
                return Err(io::Error::other(
                    "profile save000.dat is missing but save000.bak exists; restore the backup explicitly",
                ));
            }
        }
        Ok(())
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join(if self.css0.is_some() {
            "save000.dat"
        } else {
            "system.dat"
        })
    }
    pub fn backup_path(&self) -> PathBuf {
        self.directory.join(if self.css0.is_some() {
            "save000.bak"
        } else {
            "system.bak"
        })
    }
    fn config_names(&self) -> (&'static str, &'static str) {
        if self.css0.is_some() {
            ("cvns.cfg", "cvns.cfg.bak")
        } else {
            ("cmvs.cfg", "cmvs.cfg.bak")
        }
    }
    pub fn read_configuration(&self) -> io::Result<Option<crate::settings::Configuration>> {
        let (name, bak) = self.config_names();
        let path = self.directory.join(name);
        let primary = regular_file(&*self.fs, &path)?;
        let backup = regular_file(&*self.fs, &self.directory.join(bak))?;
        if !primary && backup {
            return Err(io::Error::other(format!(
                "profile {name} is missing but its backup exists; restore explicitly"
            )));
        }
        if primary {
            crate::settings::Configuration::read_with_fs(&*self.fs, &path)
        } else {
            Ok(None)
        }
    }
    pub fn write_configuration(
        &mut self,
        configuration: &crate::settings::Configuration,
    ) -> io::Result<()> {
        let (name, bak) = self.config_names();
        let path = self.directory.join(name);
        let backup = self.directory.join(bak);
        regular_file(&*self.fs, &backup)?;
        let previous = self.read_configuration()?;
        let staged = self.stage(configuration.bytes())?;
        if let Some(previous) = previous {
            self.stage(previous.bytes())?.commit(&backup)?;
        }
        staged.commit(&path)
    }
    fn stage(&self, raw: &[u8]) -> io::Result<StagedFile> {
        for _ in 0..100 {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = self
                .directory
                .join(format!(".cmvs-system-{}-{id}.tmp", self.fs.session_id()));
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open_with(&*self.fs, &path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let staged = StagedFile {
                fs: Arc::clone(&self.fs),
                path,
                committed: false,
            };
            let result = file.write_all(raw).and_then(|()| file.sync_all());
            drop(file);
            result?;
            return Ok(staged);
        }
        Err(io::Error::other(
            "cannot allocate a new profile staging file",
        ))
    }
}
impl SystemSaveSink for ProfileSystemSave {
    fn write(&mut self, raw: &[u8]) -> io::Result<()> {
        let primary = self.path();
        let backup = self.backup_path();
        let previous = if let Some(starts) = self.css0 {
            super::Css0Save::decode(raw, starts).map_err(io::Error::other)?;
            if regular_file(&*self.fs, &primary)? {
                let raw = FileSystemSave::css0_with_fs(&primary, Arc::clone(&self.fs))
                    .read()?
                    .ok_or_else(|| io::Error::other("profile save000.dat disappeared"))?;
                super::Css0Save::decode(&raw, starts).map_err(io::Error::other)?;
                Some(raw)
            } else {
                None
            }
        } else {
            let incoming = Css1Save::decode(raw).map_err(io::Error::other)?;
            if regular_file(&*self.fs, &primary)? {
                let raw = FileSystemSave::with_fs(&primary, Arc::clone(&self.fs))
                    .read()?
                    .ok_or_else(|| io::Error::other("profile system.dat disappeared"))?;
                let previous = Css1Save::decode(&raw).map_err(io::Error::other)?;
                if previous.layout().game_id != 0
                    && previous.layout().game_id != incoming.layout().game_id
                {
                    return Err(io::Error::other("profile CSS1 game identifier mismatch"));
                }
                Some(raw)
            } else {
                None
            }
        };
        regular_file(&*self.fs, &backup)?;
        let staged = self.stage(raw)?;
        if let Some(previous) = previous {
            self.stage(&previous)?.commit(&backup)?;
        }
        // The only fallible step after backing up is the primary rename. Its success
        // commits the new state; no later I/O can turn that success into a VM retry.
        staged.commit(&primary)
    }
}

//! Explicit user-directory storage policy, independent of original CMVS file replacement.
use super::{Css1Save, FileSystemSave, SystemSaveSink, SystemSaveSource};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn regular_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::other(
            "profile entries must be regular files, not links or directories",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn external_directory(path: &Path, game_root: &Path) -> io::Result<PathBuf> {
    let game_root = game_root.canonicalize()?;
    let mut ancestor = path.to_path_buf();
    let mut missing = Vec::new();
    while !ancestor.try_exists()? {
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
    let mut resolved = ancestor.canonicalize()?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    if resolved.starts_with(&game_root) {
        return Err(io::Error::other(
            "user data directory must be outside the read-only game root",
        ));
    }
    fs::create_dir_all(&resolved)?;
    let resolved = resolved.canonicalize()?;
    if resolved.starts_with(game_root) {
        return Err(io::Error::other(
            "user data directory resolves inside the read-only game root",
        ));
    }
    Ok(resolved)
}

struct StagedFile {
    path: PathBuf,
    committed: bool,
}
impl StagedFile {
    fn commit(mut self, target: &Path) -> io::Result<()> {
        fs::rename(&self.path, target)?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// One session owns an OS file lock. Close/crash releases the lock automatically;
/// the empty lock file remains and must not be removed while a session owns it.
pub struct ProfileSystemSave {
    directory: PathBuf,
    _lock: File,
}
impl ProfileSystemSave {
    pub fn open(directory: &Path, game_root: &Path) -> io::Result<Self> {
        let directory = external_directory(directory, game_root)?;
        let lock_path = directory.join(".cmvs-system.lock");
        if regular_file(&lock_path)? && fs::metadata(&lock_path)?.len() != 0 {
            return Err(io::Error::other("profile lock file must be empty"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        if lock.metadata()?.len() != 0 {
            return Err(io::Error::other("profile lock file must be empty"));
        }
        lock.try_lock().map_err(|error| {
            io::Error::other(format!("cannot lock user data directory: {error}"))
        })?;
        let profile = Self {
            directory,
            _lock: lock,
        };
        let primary = regular_file(&profile.path())?;
        let backup = regular_file(&profile.backup_path())?;
        if !primary && backup {
            return Err(io::Error::other(
                "profile system.dat is missing but system.bak exists; restore the backup explicitly",
            ));
        }
        Ok(profile)
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join("system.dat")
    }
    pub fn backup_path(&self) -> PathBuf {
        self.directory.join("system.bak")
    }
    pub fn read_configuration(&self) -> io::Result<Option<crate::settings::Configuration>> {
        let path = self.directory.join("cmvs.cfg");
        let primary = regular_file(&path)?;
        let backup = regular_file(&self.directory.join("cmvs.cfg.bak"))?;
        if !primary && backup {
            return Err(io::Error::other(
                "profile cmvs.cfg is missing but its backup exists; restore explicitly",
            ));
        }
        if primary {
            crate::settings::Configuration::read(&path)
        } else {
            Ok(None)
        }
    }
    pub fn write_configuration(
        &mut self,
        configuration: &crate::settings::Configuration,
    ) -> io::Result<()> {
        let path = self.directory.join("cmvs.cfg");
        let backup = self.directory.join("cmvs.cfg.bak");
        regular_file(&backup)?;
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
                .join(format!(".cmvs-system-{}-{id}.tmp", std::process::id()));
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let staged = StagedFile {
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
        let incoming = Css1Save::decode(raw).map_err(io::Error::other)?;
        let primary = self.path();
        let backup = self.backup_path();
        let previous = if regular_file(&primary)? {
            let raw = FileSystemSave::new(&primary)
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
        };
        regular_file(&backup)?;
        let staged = self.stage(raw)?;
        if let Some(previous) = previous {
            self.stage(&previous)?.commit(&backup)?;
        }
        // The only fallible step after backing up is the primary rename. Its success
        // commits the new state; no later I/O can turn that success into a VM retry.
        staged.commit(&primary)
    }
}

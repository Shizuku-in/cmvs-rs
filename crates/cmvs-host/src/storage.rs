//! Opt-in game-local storage for embedding applications. Research/CLI players
//! retain their explicit external-profile policy.
use cmvs_core::{format::system_save::ProfileSystemSave, host::filesystem::HostFs};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(target_os = "android")]
mod android_fs {
    use cmvs_core::{
        host::filesystem::File, host::filesystem::HostFile, host::filesystem::HostFs,
        host::filesystem::Metadata, host::filesystem::OpenOptions, host::filesystem::StdFs,
        host::filesystem::metadata_from_std,
    };
    use std::{
        io::{self, Read, Seek, SeekFrom, Write},
        path::{Path, PathBuf},
    };

    struct AndroidFile(std::fs::File);

    impl Read for AndroidFile {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for AndroidFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    impl Seek for AndroidFile {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }

    impl HostFile for AndroidFile {
        fn metadata(&self) -> io::Result<Metadata> {
            Ok(metadata_from_std(&self.0.metadata()?))
        }
        fn sync_all(&self) -> io::Result<()> {
            self.0.sync_all()
        }
        fn try_lock(&self) -> io::Result<()> {
            rustix::fs::flock(
                &self.0,
                rustix::fs::FlockOperation::NonBlockingLockExclusive,
            )
            .map_err(io::Error::from)
        }
    }

    /// Android filesystem adapter using non-blocking OS `flock` for save-directory locks.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct AndroidHostFs;

    impl HostFs for AndroidHostFs {
        fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
            Ok(File::from_host(Box::new(AndroidFile(
                StdFs::open_std_file(path, options)?,
            ))))
        }
        fn metadata(&self, path: &Path) -> io::Result<Metadata> {
            StdFs.metadata(path)
        }
        fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata> {
            StdFs.symlink_metadata(path)
        }
        fn try_exists(&self, path: &Path) -> io::Result<bool> {
            StdFs.try_exists(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            StdFs.is_file(path)
        }
        fn is_dir(&self, path: &Path) -> bool {
            StdFs.is_dir(path)
        }
        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            StdFs.canonicalize(path)
        }
        fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
            StdFs.absolute(path)
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            StdFs.create_dir_all(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            StdFs.remove_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            StdFs.rename(from, to)
        }
        fn session_id(&self) -> u32 {
            StdFs.session_id()
        }
        fn read_dir_names(&self, dir: &Path) -> io::Result<Option<Vec<std::ffi::OsString>>> {
            StdFs.read_dir_names(dir)
        }
    }
}

#[cfg(target_os = "android")]
pub use android_fs::AndroidHostFs;

/// Default standard-library [`HostFs`] for this OS (`AndroidHostFs` on Android, `StdFs` elsewhere).
pub fn default_host_fs() -> Arc<dyn HostFs> {
    #[cfg(target_os = "android")]
    {
        Arc::new(AndroidHostFs)
    }
    #[cfg(not(target_os = "android"))]
    {
        cmvs_core::host::filesystem::default_fs()
    }
}

const IMPORTED: &str = ".cmvs-legacy-imported";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Opens `<root>/savedata`, importing legacy external saves once without overwriting local files,
/// or falls back to `legacy` on read-only game media.
pub fn game_savedata(root: &Path, legacy: &Path) -> io::Result<ProfileSystemSave> {
    let fs = default_host_fs();
    let profile = match ProfileSystemSave::open_game_savedata_with_fs(root, fs.clone()) {
        Ok(profile) => profile,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return ProfileSystemSave::open_with_fs(legacy, root, fs);
        }
        Err(error) => return Err(error),
    };
    let target = profile.directory();
    if regular_file(&target.join(IMPORTED))? || !legacy.try_exists()? {
        return Ok(profile);
    }
    // Hold both locks throughout migration. An active legacy session must not
    // race copying, nor may a second local player scan a partial import.
    let source = ProfileSystemSave::open_with_fs(legacy, root, fs)?;
    import_entries(
        fs::read_dir(source.directory())?.collect::<io::Result<Vec<_>>>()?,
        target,
    )?;
    // A deleted local slot must not be resurrected from the old profile at the
    // next launch. Partial failed imports are retried without replacing files.
    let mut staged = Staged::new(target)?;
    staged.file.write_all(b"1\n")?;
    staged.commit(&target.join(IMPORTED))?;
    Ok(profile)
}

fn import_entries(mut entries: Vec<fs::DirEntry>, target: &Path) -> io::Result<()> {
    // Import backups last so an interrupted import cannot leave an orphaned
    // backup that prevents opening the local profile to retry the migration.
    entries.sort_by_key(|entry| {
        let name = entry.file_name();
        name == "system.bak" || name == "save000.bak" || name == "cmvs.cfg.bak"
    });
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if save_file(&name) {
            copy_missing(&entry.path(), &target.join(name.as_ref()))?;
        } else if name == "font-faces" {
            real_directory(&entry.path())?;
            let fonts = target.join("font-faces");
            fs::create_dir_all(&fonts)?;
            real_directory(&fonts)?;
            for font in fs::read_dir(entry.path())? {
                let font = font?;
                let name = font.file_name();
                let text = name.to_string_lossy();
                let hash = if text.len() == 31
                    && (text.starts_with("CMVS-U-") || text.starts_with("CMVS-F-"))
                {
                    Some(&text.as_bytes()[7..])
                } else if text.len() == 29 && text.starts_with("file-") {
                    Some(&text.as_bytes()[5..])
                } else {
                    None
                };
                if hash.is_some_and(|hash| hash.iter().all(u8::is_ascii_hexdigit)) {
                    copy_missing(&font.path(), &fonts.join(name))?;
                }
            }
        }
    }
    Ok(())
}

fn save_file(name: &str) -> bool {
    matches!(
        name,
        "system.dat"
            | "system.bak"
            | "save000.bak"
            | "cmvs.cfg"
            | "cmvs.cfg.bak"
            | "cvns.cfg"
            | "cvns.cfg.bak"
    ) || (name.len() == 11
        && name.starts_with("save")
        && name.ends_with(".dat")
        && name.as_bytes()[4..7].iter().all(u8::is_ascii_digit))
}

fn regular_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::other("save import requires regular files")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn real_directory(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(io::Error::other("save import requires real directories"));
    }
    Ok(())
}

fn copy_missing(source: &Path, target: &Path) -> io::Result<()> {
    if regular_file(target)? {
        return Ok(());
    }
    if !regular_file(source)? {
        return Err(io::Error::other("legacy save disappeared during import"));
    }
    let mut staged = Staged::new(target.parent().expect("save directory"))?;
    io::copy(&mut File::open(source)?, &mut staged.file)?;
    staged.commit(target)
}

struct Staged {
    path: PathBuf,
    file: File,
}
impl Staged {
    fn new(directory: &Path) -> io::Result<Self> {
        for _ in 0..100 {
            let path = directory.join(format!(
                ".cmvs-import-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("cannot stage a legacy save"))
    }
    fn commit(self, target: &Path) -> io::Result<()> {
        self.file.sync_all()?;
        fs::rename(&self.path, target)
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let base = std::env::var_os("CMVS_TEST_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = base.join(format!(
                "cmvs-host-storage-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("game")).unwrap();
            Self(root)
        }
        fn game(&self) -> PathBuf {
            self.0.join("game")
        }
        fn legacy(&self) -> PathBuf {
            self.0.join("legacy")
        }
        fn savedata(&self) -> PathBuf {
            self.game().join("savedata")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn game_savedata_imports_once_preserves_local_files_and_keeps_legacy() {
        let root = Fixture::new();
        fs::create_dir(root.legacy()).unwrap();
        fs::create_dir(root.savedata()).unwrap();
        fs::write(root.legacy().join("save001.dat"), b"old slot one").unwrap();
        fs::write(root.legacy().join("save002.dat"), b"slot two").unwrap();
        fs::write(root.legacy().join("cmvs.cfg"), b"[S]\nX=1\n").unwrap();
        fs::write(root.legacy().join("unrelated"), b"not a save").unwrap();
        fs::write(root.savedata().join("save001.dat"), b"new local slot").unwrap();
        let fonts = root.legacy().join("font-faces");
        fs::create_dir(&fonts).unwrap();
        for prefix in ["CMVS-U-", "CMVS-F-", "file-"] {
            fs::write(
                fonts.join(format!("{prefix}0123456789abcdef01234567")),
                b"font mapping",
            )
            .unwrap();
        }
        let profile = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert_eq!(
            profile.directory(),
            fs::canonicalize(root.savedata()).unwrap()
        );
        assert_eq!(
            fs::read(root.savedata().join("save001.dat")).unwrap(),
            b"new local slot"
        );
        assert_eq!(
            fs::read(root.savedata().join("save002.dat")).unwrap(),
            b"slot two"
        );
        assert_eq!(
            fs::read(root.legacy().join("save001.dat")).unwrap(),
            b"old slot one"
        );
        assert!(profile.read_configuration().unwrap().is_some());
        assert!(!root.savedata().join("unrelated").exists());
        assert_eq!(
            fs::read_dir(root.savedata().join("font-faces"))
                .unwrap()
                .count(),
            3
        );
        drop(profile);
        fs::remove_file(root.savedata().join("save002.dat")).unwrap();
        let _reopened = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert!(!root.savedata().join("save002.dat").exists());
    }

    #[test]
    fn local_lock_contention_and_bad_saves_do_not_fall_back_to_empty_profile() {
        let root = Fixture::new();
        let profile = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert!(game_savedata(&root.game(), &root.legacy()).is_err());
        assert!(!root.legacy().exists());
        drop(profile);
        fs::write(root.savedata().join("system.bak"), b"orphaned backup").unwrap();
        assert!(game_savedata(&root.game(), &root.legacy()).is_err());
        assert!(!root.legacy().exists());
    }

    #[test]
    fn interrupted_import_with_backups_first_can_reopen_and_retry() {
        let root = Fixture::new();
        let source = ProfileSystemSave::open(&root.legacy(), &root.game()).unwrap();
        let local = ProfileSystemSave::open_game_savedata(&root.game()).unwrap();
        let originals = [
            ("system.dat", b"system primary".as_slice()),
            ("system.bak", b"system backup".as_slice()),
            ("cmvs.cfg", b"[S]\nX=1\n".as_slice()),
            ("cmvs.cfg.bak", b"[S]\nX=0\n".as_slice()),
        ];
        for (name, bytes) in originals {
            fs::write(root.legacy().join(name), bytes).unwrap();
        }
        let bad_slot = root.legacy().join("save003.dat");
        #[cfg(unix)]
        std::os::unix::fs::symlink("missing", &bad_slot).unwrap();
        #[cfg(not(unix))]
        fs::create_dir(&bad_slot).unwrap();
        let mut entries = fs::read_dir(source.directory())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), entry)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        // Force the review's failure order independently of filesystem ordering.
        let order = [
            "system.bak",
            "cmvs.cfg.bak",
            "save003.dat",
            "system.dat",
            "cmvs.cfg",
        ];
        let entries = order
            .into_iter()
            .map(|name| entries.remove(std::ffi::OsStr::new(name)).unwrap())
            .collect();
        assert!(import_entries(entries, local.directory()).is_err());
        assert!(!root.savedata().join(IMPORTED).exists());
        assert!(!root.savedata().join("system.bak").exists());
        assert!(!root.savedata().join("cmvs.cfg.bak").exists());
        drop(local);
        let reopened = ProfileSystemSave::open_game_savedata(&root.game()).unwrap();
        assert!(reopened.read_configuration().unwrap().is_none());
        drop(reopened);
        drop(source);
        #[cfg(unix)]
        fs::remove_file(&bad_slot).unwrap();
        #[cfg(not(unix))]
        fs::remove_dir(&bad_slot).unwrap();
        fs::write(&bad_slot, b"slot three").unwrap();
        let imported = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert!(imported.read_configuration().unwrap().is_some());
        assert!(root.savedata().join(IMPORTED).is_file());
        assert_eq!(
            fs::read(root.savedata().join("save003.dat")).unwrap(),
            b"slot three"
        );
        for (name, bytes) in originals {
            assert_eq!(fs::read(root.savedata().join(name)).unwrap(), bytes);
            assert_eq!(fs::read(root.legacy().join(name)).unwrap(), bytes);
        }
    }

    #[test]
    fn migration_waits_for_legacy_session_and_retries_without_overwriting() {
        let root = Fixture::new();
        let legacy = ProfileSystemSave::open(&root.legacy(), &root.game()).unwrap();
        fs::write(root.legacy().join("save001.dat"), b"old save").unwrap();
        assert!(game_savedata(&root.game(), &root.legacy()).is_err());
        assert!(!root.savedata().join(IMPORTED).exists());
        assert!(!root.savedata().join("save001.dat").exists());
        drop(legacy);
        let _profile = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert_eq!(
            fs::read(root.savedata().join("save001.dat")).unwrap(),
            b"old save"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_game_falls_back_to_the_same_external_profile() {
        use std::os::unix::fs::PermissionsExt;
        let root = Fixture::new();
        fs::set_permissions(root.game(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = game_savedata(&root.game(), &root.legacy());
        fs::set_permissions(root.game(), fs::Permissions::from_mode(0o755)).unwrap();
        let profile = result.unwrap();
        // Privileged test runners can bypass POSIX write permissions.
        if root.savedata().exists() {
            return;
        }
        assert_eq!(
            profile.directory(),
            fs::canonicalize(root.legacy()).unwrap()
        );
        drop(profile);
        let migrated = game_savedata(&root.game(), &root.legacy()).unwrap();
        assert_eq!(
            migrated.directory(),
            fs::canonicalize(root.savedata()).unwrap()
        );
    }
}

//! Opt-in game-local storage for embedding applications. Research/CLI players
//! retain their explicit external-profile policy.
use cmvs_core::system_save::ProfileSystemSave;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const IMPORTED: &str = ".cmvs-legacy-imported";
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) fn game_savedata(root: &Path, legacy: &Path) -> io::Result<ProfileSystemSave> {
    let profile = match ProfileSystemSave::open_game_savedata(root) {
        Ok(profile) => profile,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return ProfileSystemSave::open(legacy, root);
        }
        Err(error) => return Err(error),
    };
    let target = profile.directory();
    if regular_file(&target.join(IMPORTED))? || !legacy.try_exists()? {
        return Ok(profile);
    }
    // Hold both locks throughout migration. An active legacy session must not
    // race copying, nor may a second local player scan a partial import.
    let source = ProfileSystemSave::open(legacy, root)?;
    for entry in fs::read_dir(source.directory())? {
        let entry = entry?;
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
    // A deleted local slot must not be resurrected from the old profile at the
    // next launch. Partial failed imports are retried without replacing files.
    let mut staged = Staged::new(target)?;
    staged.file.write_all(b"1\n")?;
    staged.commit(&target.join(IMPORTED))?;
    Ok(profile)
}

fn save_file(name: &str) -> bool {
    matches!(
        name,
        "system.dat" | "system.bak" | "cmvs.cfg" | "cmvs.cfg.bak"
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

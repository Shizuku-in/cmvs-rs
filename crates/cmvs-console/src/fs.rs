//! Filesystem adapter for single-application game consoles (Switch, PlayStation Vita).
use cmvs_core::{
    host::filesystem::File, host::filesystem::HostFile, host::filesystem::HostFs,
    host::filesystem::Metadata, host::filesystem::OpenOptions, host::filesystem::StdFs,
    host::filesystem::metadata_from_std,
};
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

struct ConsoleFile(std::fs::File);

impl Read for ConsoleFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for ConsoleFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for ConsoleFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

impl HostFile for ConsoleFile {
    fn metadata(&self) -> io::Result<Metadata> {
        Ok(metadata_from_std(&self.0.metadata()?))
    }

    fn sync_all(&self) -> io::Result<()> {
        self.0.sync_all()
    }

    // Horizon and the Vita run one application at a time and their std has no
    // file locks; the open lock file alone marks the profile.
    fn try_lock(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Filesystem adapter for Switch/Horizon and Vita save directories.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleFs;

impl HostFs for ConsoleFs {
    fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
        Ok(File::from_host(Box::new(ConsoleFile(
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

    /// The Horizon and Vita filesystems reject an occupied target, so the old file is
    /// removed first: unlike elsewhere this is not atomic, and an interruption between
    /// the steps leaves only `from`.
    fn replace(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self.rename(from, to) {
            Err(error) if self.is_file(from) && self.is_file(to) => {
                self.remove_file(to).map_err(|_| error)?;
                self.rename(from, to)
            }
            result => result,
        }
    }

    fn session_id(&self) -> u32 {
        StdFs.session_id()
    }

    fn read_dir_names(&self, dir: &Path) -> io::Result<Option<Vec<std::ffi::OsString>>> {
        StdFs.read_dir_names(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_fs_replaces_existing_file_and_allows_lock() {
        let dir = std::env::temp_dir().join(format!("cmvs-console-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("from.tmp");
        let to = dir.join("to.dat");
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"old").unwrap();

        ConsoleFs.replace(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());

        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open_with(&ConsoleFs, dir.join(".lock"))
            .unwrap();
        lock.try_lock().unwrap();
        drop(lock);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

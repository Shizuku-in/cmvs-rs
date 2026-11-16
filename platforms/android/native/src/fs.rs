//! Android filesystem adapter using non-blocking `flock` for save-directory locks.
use cmvs_core::filesystem::{
    File, HostFile, HostFs, Metadata, OpenOptions, StdFs, metadata_from_std,
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

    // std::fs::File locking is unsupported on Android; use nonblocking OS flock
    // while keeping the file descriptor open until drop.
    fn try_lock(&self) -> io::Result<()> {
        rustix::fs::flock(
            &self.0,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(io::Error::from)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidFs;

impl HostFs for AndroidFs {
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

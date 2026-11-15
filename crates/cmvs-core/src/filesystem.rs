//! Host filesystem capability and the optional `std::fs` default backend.
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[cfg(feature = "std-fs")]
use std::sync::Arc;

/// Opened file handle supplied by a [`HostFs`] implementation.
pub trait HostFile: Read + Write + Seek + Send + Sync {
    /// Query metadata for the opened file.
    fn metadata(&self) -> io::Result<Metadata>;
    /// Flush file contents and metadata to durable storage.
    fn sync_all(&self) -> io::Result<()>;
    /// Acquire a non-blocking exclusive advisory lock on this handle.
    fn try_lock(&self) -> io::Result<()>;
}

/// Type-erased file handle returned by [`HostFs`].
pub struct File(Box<dyn HostFile>);

impl File {
    /// Wrap a host-specific file handle.
    pub fn from_host(file: Box<dyn HostFile>) -> Self {
        Self(file)
    }

    /// Open an existing file for reading with [`StdFs`].
    #[cfg(feature = "std-fs")]
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        StdFs.open_read(path.as_ref())
    }

    /// Query metadata for the opened file.
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.0.metadata()
    }

    /// Flush file contents and metadata to durable storage.
    pub fn sync_all(&self) -> io::Result<()> {
        self.0.sync_all()
    }

    /// Acquire a non-blocking exclusive advisory lock on this file handle.
    pub fn try_lock(&self) -> io::Result<()> {
        self.0.try_lock()
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

/// Subset of filesystem entry type flags used by core validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileType {
    /// Whether the inspected path itself is a symbolic link.
    pub is_symlink: bool,
}

impl FileType {
    /// Return whether the entry is a symbolic link.
    pub const fn is_symlink(self) -> bool {
        self.is_symlink
    }
}

/// File metadata queried through [`HostFs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// Byte length of the file, or 0 for non-files.
    pub len: u64,
    /// Whether the entry is a regular file.
    pub is_file: bool,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// Whether the entry itself is a symbolic link.
    pub is_symlink: bool,
}

#[allow(clippy::len_without_is_empty)]
impl Metadata {
    /// Byte length of the file.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the entry is a regular file.
    pub const fn is_file(&self) -> bool {
        self.is_file
    }

    /// Whether the entry is a directory.
    pub const fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// Type flags for the entry.
    pub const fn file_type(&self) -> FileType {
        FileType {
            is_symlink: self.is_symlink,
        }
    }
}

/// Options used to open or create a file through [`HostFs`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub create_new: bool,
    pub truncate: bool,
}

impl OpenOptions {
    /// Create a blank set of open flags.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, value: bool) -> &mut Self {
        self.read = value;
        self
    }

    pub fn write(&mut self, value: bool) -> &mut Self {
        self.write = value;
        self
    }

    pub fn create(&mut self, value: bool) -> &mut Self {
        self.create = value;
        self
    }

    pub fn create_new(&mut self, value: bool) -> &mut Self {
        self.create_new = value;
        self
    }

    pub fn truncate(&mut self, value: bool) -> &mut Self {
        self.truncate = value;
        self
    }

    /// Open `path` using the given [`HostFs`].
    pub fn open_with(&self, fs: &dyn HostFs, path: impl AsRef<Path>) -> io::Result<File> {
        fs.open(path.as_ref(), self)
    }

    /// Open `path` using [`StdFs`].
    #[cfg(feature = "std-fs")]
    pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        self.open_with(&StdFs, path)
    }
}

/// Host-supplied filesystem and directory-locking capability for [`crate::runtime::Runtime`].
pub trait HostFs: Send + Sync {
    /// Open or create a file with `options`.
    fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File>;
    /// Query metadata following symbolic links.
    fn metadata(&self, path: &Path) -> io::Result<Metadata>;
    /// Query metadata without following symbolic links.
    fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata>;
    /// Check whether `path` exists, propagating I/O or permission errors.
    fn try_exists(&self, path: &Path) -> io::Result<bool>;
    /// Return whether `path` refers to a regular file.
    fn is_file(&self, path: &Path) -> bool {
        self.metadata(path).is_ok_and(|meta| meta.is_file())
    }
    /// Return whether `path` refers to a directory.
    fn is_dir(&self, path: &Path) -> bool {
        self.metadata(path).is_ok_and(|meta| meta.is_dir())
    }
    /// Resolve `path` to its canonical, symlink-free form.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// Make `path` absolute without requiring it to exist.
    fn absolute(&self, path: &Path) -> io::Result<PathBuf>;
    /// Create `path` and all missing parent directories.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Remove a regular file.
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Rename `from` to `to`.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Move `from` over `to`, replacing an existing file.
    fn replace(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.rename(from, to)
    }
    /// Disambiguate temporary filenames across concurrent sessions on the same host.
    fn session_id(&self) -> u32 {
        0
    }
    /// Open an existing file for reading.
    fn open_read(&self, path: &Path) -> io::Result<File> {
        self.open(
            path,
            &OpenOptions {
                read: true,
                ..Default::default()
            },
        )
    }
}

/// Convert [`std::fs::Metadata`] to [`Metadata`] for hosts wrapping standard files.
#[cfg(feature = "std-fs")]
pub fn metadata_from_std(meta: &std::fs::Metadata) -> Metadata {
    Metadata {
        len: meta.len(),
        is_file: meta.is_file(),
        is_dir: meta.is_dir(),
        is_symlink: meta.file_type().is_symlink(),
    }
}

#[cfg(feature = "std-fs")]
struct StdFile(std::fs::File);

#[cfg(feature = "std-fs")]
impl Read for StdFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

#[cfg(feature = "std-fs")]
impl Write for StdFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(feature = "std-fs")]
impl Seek for StdFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

#[cfg(feature = "std-fs")]
impl HostFile for StdFile {
    fn metadata(&self) -> io::Result<Metadata> {
        Ok(metadata_from_std(&self.0.metadata()?))
    }

    fn sync_all(&self) -> io::Result<()> {
        self.0.sync_all()
    }

    fn try_lock(&self) -> io::Result<()> {
        self.0.try_lock().map_err(io::Error::from)
    }
}

/// Standard-library filesystem backend for hosts using `std::fs`.
#[cfg(feature = "std-fs")]
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFs;

#[cfg(feature = "std-fs")]
impl StdFs {
    /// Open `path` directly with `std::fs::OpenOptions`.
    pub fn open_std_file(path: &Path, options: &OpenOptions) -> io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .read(options.read)
            .write(options.write)
            .create(options.create)
            .create_new(options.create_new)
            .truncate(options.truncate)
            .open(path)
    }
}

#[cfg(feature = "std-fs")]
impl HostFs for StdFs {
    fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
        Ok(File::from_host(Box::new(StdFile(Self::open_std_file(
            path, options,
        )?))))
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        Ok(metadata_from_std(&std::fs::metadata(path)?))
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata> {
        Ok(metadata_from_std(&std::fs::symlink_metadata(path)?))
    }

    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn session_id(&self) -> u32 {
        std::process::id()
    }
}

/// Shared [`StdFs`] instance for callers using default host storage.
#[cfg(feature = "std-fs")]
pub fn default_fs() -> Arc<dyn HostFs> {
    Arc::new(StdFs)
}

#[cfg(all(test, feature = "std-fs"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingFs {
        locks: Arc<AtomicUsize>,
        replaces: Arc<AtomicUsize>,
    }

    struct RecordingFile(StdFile, Arc<AtomicUsize>);

    impl Read for RecordingFile {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for RecordingFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    impl Seek for RecordingFile {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }

    impl HostFile for RecordingFile {
        fn metadata(&self) -> io::Result<Metadata> {
            self.0.metadata()
        }

        fn sync_all(&self) -> io::Result<()> {
            self.0.sync_all()
        }

        fn try_lock(&self) -> io::Result<()> {
            self.1.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    impl HostFs for RecordingFs {
        fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
            Ok(File::from_host(Box::new(RecordingFile(
                StdFile(StdFs::open_std_file(path, options)?),
                Arc::clone(&self.locks),
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

        fn replace(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.replaces.fetch_add(1, Ordering::Relaxed);
            StdFs.replace(from, to)
        }

        fn session_id(&self) -> u32 {
            42
        }
    }

    #[test]
    fn custom_host_fs_handles_profile_lock_and_replace() {
        let root = std::env::temp_dir().join(format!("cmvs-host-fs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("game");
        let user = root.join("user");
        std::fs::create_dir_all(&game).unwrap();

        let locks = Arc::new(AtomicUsize::new(0));
        let replaces = Arc::new(AtomicUsize::new(0));
        let fs: Arc<dyn HostFs> = Arc::new(RecordingFs {
            locks: Arc::clone(&locks),
            replaces: Arc::clone(&replaces),
        });

        let mut profile = crate::system_save::ProfileSystemSave::open_with_fs(
            &user,
            &game,
            Arc::clone(&fs),
        )
        .unwrap();
        assert_eq!(locks.load(Ordering::Relaxed), 1);

        let cfg = crate::settings::Configuration::new(b"[SYSTEM_INFO]\r\n".to_vec()).unwrap();
        profile.write_configuration(&cfg).unwrap();
        assert_eq!(replaces.load(Ordering::Relaxed), 1);

        drop(profile);
        std::fs::remove_dir_all(&root).unwrap();
    }
}

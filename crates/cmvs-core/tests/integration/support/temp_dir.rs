use std::{
    ffi::OsStr,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn unique_name(tag: &str) -> String {
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("cmvs-test-{tag}-{}-{seq}", std::process::id())
}

/// RAII guard for a unique temporary path that is not pre-created on construction,
/// removing any created file or directory tree when dropped.
pub struct TempPath(PathBuf);

impl TempPath {
    pub fn new(tag: &str) -> Self {
        Self::in_dir(std::env::temp_dir(), tag)
    }

    pub fn in_target_tmp(tag: &str) -> Self {
        Self::in_dir(PathBuf::from(env!("CARGO_TARGET_TMPDIR")), tag)
    }

    pub fn in_dir(parent: impl Into<PathBuf>, tag: &str) -> Self {
        let path = parent.into().join(unique_name(tag));
        let _ = fs::remove_dir_all(&path);
        let _ = fs::remove_file(&path);
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Deref for TempPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TempPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for TempPath {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

/// RAII guard for an eagerly created temporary directory.
pub struct TempDir(TempPath);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        Self::in_dir(std::env::temp_dir(), tag)
    }

    pub fn in_target_tmp(tag: &str) -> Self {
        Self::in_dir(PathBuf::from(env!("CARGO_TARGET_TMPDIR")), tag)
    }

    pub fn in_dir(parent: impl Into<PathBuf>, tag: &str) -> Self {
        let inner = TempPath::in_dir(parent, tag);
        fs::create_dir_all(&inner).unwrap();
        Self(inner)
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.0.path()
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<OsStr> for TempDir {
    fn as_ref(&self) -> &OsStr {
        self.0.as_ref()
    }
}

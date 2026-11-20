//! Common CLI, file and script-table formats for the offline tools.
pub mod cli;
pub mod project;
pub mod script;
pub mod table;

use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    path::Path,
};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(format!("{} exceeds the {} byte input limit", path.display(), limit).into());
    }
    Ok(bytes)
}

/// Publish a complete, verified file without replacing an existing destination.
pub fn write_new(path: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    if path.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output already exists: {}", path.display()),
        )
        .into());
    }
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    write(temporary.as_file_mut())?;
    temporary.persist_noclobber(path)?;
    Ok(())
}

/// Stage a complete multi-file project beside its destination and publish it once.
pub fn write_new_dir(path: &Path, build: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    if !cfg!(any(
        windows,
        target_os = "linux",
        target_vendor = "apple",
        target_os = "android",
        target_os = "redox"
    )) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exclusive directory publication is unavailable on this platform",
        )
        .into());
    }
    if path.try_exists()? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "output directory already exists",
        )
        .into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = tempfile::tempdir_in(parent)?;
    build(tmp.path())?;
    // TempDir handles recursive cleanup if the exclusive rename fails.
    tempfile::TempPath::try_from_path(tmp.path())?
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    let _ = tmp.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_directory_created_during_staging() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        let result = write_new_dir(&out, |stage| {
            std::fs::write(stage.join("staged"), b"content")?;
            Ok(std::fs::create_dir(&out)?)
        });
        assert!(result.is_err());
        if out.exists() {
            assert_eq!(std::fs::read_dir(out).unwrap().count(), 0);
        } else {
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<io::Error>()
                    .unwrap()
                    .kind(),
                io::ErrorKind::Unsupported
            );
        }
    }
}

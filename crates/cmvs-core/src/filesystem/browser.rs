//! Synchronous imports are supplied only inside the dedicated game worker.
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["globalThis", "cmvsFs"], js_name = size)]
    fn host_size(path: &str) -> f64;
    #[wasm_bindgen(catch, js_namespace = ["globalThis", "cmvsFs"], js_name = read)]
    fn host_read(path: &str, offset: f64, length: u32) -> Result<Vec<u8>, JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["globalThis", "cmvsFs"], js_name = write)]
    fn host_write(path: &str, offset: f64, bytes: &[u8]) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["globalThis", "cmvsFs"], js_name = operation)]
    fn host_operation(op: &str, path: &str, target: &str) -> Result<(), JsValue>;
}
fn host_error(error: JsValue) -> io::Error {
    io::Error::other(format!("browser filesystem: {error:?}"))
}
fn missing() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}
fn normalized(path: impl AsRef<Path>) -> io::Result<String> {
    let mut parts = Vec::new();
    for part in path.as_ref().components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => parts.push(
                name.to_str()
                    .ok_or_else(|| io::Error::other("non-UTF8 browser path"))?,
            ),
            _ => return Err(io::Error::other("invalid browser path")),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let name = normalized(path)?;
    if host_size(&name) == -1.0 {
        return Err(missing());
    }
    Ok(name.into())
}
pub fn absolute(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    Ok(normalized(path)?.into())
}
pub fn try_exists(path: impl AsRef<Path>) -> io::Result<bool> {
    Ok(host_size(&normalized(path)?) != -1.0)
}
pub fn is_file(path: impl AsRef<Path>) -> bool {
    normalized(path).is_ok_and(|p| host_size(&p) >= 0.0)
}
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    normalized(path).is_ok_and(|p| host_size(&p) == -2.0)
}
pub struct Metadata {
    size: f64,
}
#[allow(clippy::len_without_is_empty)] // Match std::fs::Metadata, including native callers.
impl Metadata {
    pub fn len(&self) -> u64 {
        self.size.max(0.0) as u64
    }
    pub fn is_file(&self) -> bool {
        self.size >= 0.0
    }
    pub fn file_type(&self) -> FileType {
        FileType
    }
}
pub struct FileType;
impl FileType {
    pub fn is_symlink(&self) -> bool {
        false
    }
}
pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let size = host_size(&normalized(path)?);
    if size == -1.0 {
        return Err(missing());
    }
    Ok(Metadata { size })
}
pub use metadata as symlink_metadata;
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    host_operation("mkdir", &normalized(path)?, "").map_err(host_error)
}
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = normalized(path)?;
    if host_size(&path) == -1.0 {
        return Err(missing());
    }
    host_operation("remove", &path, "").map_err(host_error)
}
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = normalized(from)?;
    if host_size(&from) == -1.0 {
        return Err(missing());
    }
    host_operation("rename", &from, &normalized(to)?).map_err(host_error)
}
pub struct File {
    path: String,
    offset: u64,
    readable: bool,
    writable: bool,
}
impl File {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().read(true).open(path)
    }
    pub fn metadata(&self) -> io::Result<Metadata> {
        metadata(&self.path)
    }
    pub fn sync_all(&self) -> io::Result<()> {
        Ok(())
    }
    // Each worker owns its private save overlay. There are no shared file handles.
    pub fn try_lock(&self) -> io::Result<()> {
        Ok(())
    }
}
impl Read for File {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.readable {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let length = buffer.len().min(1024 * 1024) as u32;
        let bytes = host_read(&self.path, self.offset as f64, length).map_err(host_error)?;
        if bytes.len() > length as usize {
            return Err(io::Error::other("oversized browser read"));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        self.offset += bytes.len() as u64;
        Ok(bytes.len())
    }
}
impl Seek for File {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let offset = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(value) => i128::from(self.offset) + i128::from(value),
            SeekFrom::End(value) => i128::from(self.metadata()?.len()) + i128::from(value),
        };
        if !(0..=i128::from(u32::MAX)).contains(&offset) {
            return Err(io::Error::other("browser file position exceeds 4 GiB"));
        }
        self.offset = offset as u64;
        Ok(self.offset)
    }
}
impl Write for File {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.writable {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        host_write(&self.path, self.offset as f64, bytes).map_err(host_error)?;
        self.offset += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Default)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    create: bool,
    create_new: bool,
    truncate: bool,
}
impl OpenOptions {
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
    pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let path = normalized(path)?;
        let size = host_size(&path);
        if size == -2.0 {
            return Err(io::Error::other("cannot open browser directory"));
        }
        if self.create_new && size >= 0.0 {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if size == -1.0 && !(self.create || self.create_new) {
            return Err(missing());
        }
        if size == -1.0 || self.truncate {
            if !self.write {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            host_operation("truncate", &path, "").map_err(host_error)?;
        }
        Ok(File {
            path,
            offset: 0,
            readable: self.read,
            writable: self.write,
        })
    }
}

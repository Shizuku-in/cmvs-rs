//! Synchronous browser-worker filesystem adapter over `globalThis.cmvsFs`.
use cmvs_core::{
    host::filesystem::File, host::filesystem::HostFile, host::filesystem::HostFs,
    host::filesystem::Metadata, host::filesystem::OpenOptions,
};
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

fn size_to_metadata(size: f64) -> Metadata {
    Metadata {
        len: size.max(0.0) as u64,
        is_file: size >= 0.0,
        is_dir: size == -2.0,
        is_symlink: false,
    }
}

pub struct WebFile {
    path: String,
    offset: u64,
    readable: bool,
    writable: bool,
}

impl Read for WebFile {
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

impl Seek for WebFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let offset = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(value) => i128::from(self.offset) + i128::from(value),
            SeekFrom::End(value) => i128::from(self.metadata()?.len()) + i128::from(value),
        };
        const MAX_SAFE_INTEGER: i128 = (1i128 << 53) - 1;
        if !(0..=MAX_SAFE_INTEGER).contains(&offset) {
            return Err(io::Error::other("browser file position out of range"));
        }
        self.offset = offset as u64;
        Ok(self.offset)
    }
}

impl Write for WebFile {
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

impl HostFile for WebFile {
    fn metadata(&self) -> io::Result<Metadata> {
        WebFs.metadata(Path::new(&self.path))
    }

    fn sync_all(&self) -> io::Result<()> {
        Ok(())
    }

    // Each worker owns its private save overlay; there are no shared file handles.
    fn try_lock(&self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WebFs;

impl HostFs for WebFs {
    fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
        let path = normalized(path)?;
        let size = host_size(&path);
        if size == -2.0 {
            return Err(io::Error::other("cannot open browser directory"));
        }
        if options.create_new && size >= 0.0 {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if size == -1.0 && !(options.create || options.create_new) {
            return Err(missing());
        }
        if size == -1.0 || options.truncate {
            if !options.write {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            host_operation("truncate", &path, "").map_err(host_error)?;
        }
        Ok(File::from_host(Box::new(WebFile {
            path,
            offset: 0,
            readable: options.read,
            writable: options.write,
        })))
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        let size = host_size(&normalized(path)?);
        if size == -1.0 {
            return Err(missing());
        }
        Ok(size_to_metadata(size))
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.metadata(path)
    }

    fn try_exists(&self, path: &Path) -> io::Result<bool> {
        Ok(host_size(&normalized(path)?) != -1.0)
    }

    fn is_file(&self, path: &Path) -> bool {
        normalized(path).is_ok_and(|p| host_size(&p) >= 0.0)
    }

    fn is_dir(&self, path: &Path) -> bool {
        normalized(path).is_ok_and(|p| host_size(&p) == -2.0)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let name = normalized(path)?;
        if host_size(&name) == -1.0 {
            return Err(missing());
        }
        Ok(name.into())
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(normalized(path)?.into())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        host_operation("mkdir", &normalized(path)?, "").map_err(host_error)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let path = normalized(path)?;
        if host_size(&path) == -1.0 {
            return Err(missing());
        }
        host_operation("remove", &path, "").map_err(host_error)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let from = normalized(from)?;
        if host_size(&from) == -1.0 {
            return Err(missing());
        }
        host_operation("rename", &from, &normalized(to)?).map_err(host_error)
    }
}

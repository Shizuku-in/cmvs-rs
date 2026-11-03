//! Native filesystem passthrough and a synchronous browser-worker adapter.
//! Browser files stay in the host; Read + Seek fetch bounded slices of archives.
#[cfg(not(target_arch = "wasm32"))]
pub use std::fs::*;

#[cfg(not(target_arch = "wasm32"))]
pub fn is_file(path: impl AsRef<std::path::Path>) -> bool {
    path.as_ref().is_file()
}
#[cfg(not(target_arch = "wasm32"))]
pub fn is_dir(path: impl AsRef<std::path::Path>) -> bool {
    path.as_ref().is_dir()
}
#[cfg(not(target_arch = "wasm32"))]
pub fn try_exists(path: impl AsRef<std::path::Path>) -> std::io::Result<bool> {
    path.as_ref().try_exists()
}

#[cfg(not(target_arch = "wasm32"))]
pub use std::path::absolute;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::*;

/// Used only to disambiguate temporary files; browser workers have private overlays.
pub fn session_id() -> u32 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::process::id()
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

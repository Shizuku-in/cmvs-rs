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

/// Moves `from` over `to`, replacing an existing file.
#[cfg(not(any(target_arch = "wasm32", target_os = "horizon")))]
pub fn replace(
    from: impl AsRef<std::path::Path>,
    to: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    rename(from, to)
}
/// Moves `from` over `to`, replacing an existing file. Horizon's filesystem rejects an
/// occupied target, so the old file is removed first: unlike elsewhere this is not
/// atomic, and an interruption between the steps leaves only `from`.
#[cfg(target_os = "horizon")]
pub fn replace(
    from: impl AsRef<std::path::Path>,
    to: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    match rename(from, to) {
        Err(error) if from.is_file() && to.is_file() => {
            remove_file(to).map_err(|_| error)?;
            rename(from, to)
        }
        result => result,
    }
}

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

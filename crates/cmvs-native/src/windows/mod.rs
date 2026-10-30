//! Win32 implementations.
mod font_dialog;
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

pub fn choose_font(owner: RawWindowHandle, current: &str) -> Result<Option<String>, NativeError> {
    let RawWindowHandle::Win32(handle) = owner else {
        return Err(NativeError::Unsupported(
            "the font dialog for a non-Win32 window",
        ));
    };
    font_dialog::choose_font(handle.hwnd, current).map_err(NativeError::Failed)
}

/// The system font folder, then the per-user folder of fonts installed without
/// administrator rights.
pub fn font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        directories.push(PathBuf::from(windows).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        directories.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    directories
}

//! Placeholder for platforms whose services are reserved.
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

pub fn choose_font(_owner: RawWindowHandle, _current: &str) -> Result<Option<String>, NativeError> {
    Err(NativeError::Unsupported("the native font dialog"))
}

pub fn font_directories() -> Vec<PathBuf> {
    Vec::new()
}

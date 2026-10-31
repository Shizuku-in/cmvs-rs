//! Operating-system services for CMVS hosts. Every platform module implements the same
//! functions; platforms without an implementation report `NativeError::Unsupported`.
//! Windows and Linux provide font services; other platforms are reserved.
use raw_window_handle::RawWindowHandle;
use std::{fmt, path::PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(target_os = "linux")]
mod linux;
// Compiled everywhere so reserved-platform services remain checked.
#[cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]
mod unsupported;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(any(windows, target_os = "linux")))]
use unsupported as platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeError {
    /// The service is not implemented for this platform or window kind yet.
    Unsupported(&'static str),
    /// The system call failed.
    Failed(String),
}
impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "{what} is not implemented on this platform"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}
impl std::error::Error for NativeError {}

/// Modal system font dialog owned by `owner`, starting from `current`; Ok(None) when the
/// user cancels. Offers faces for Japanese text, without vertical variants, that the
/// host can rasterize.
pub fn choose_font(owner: RawWindowHandle, current: &str) -> Result<Option<String>, NativeError> {
    platform::choose_font(owner, current)
}

/// Directories holding the system's installed fonts, most general first.
pub fn font_directories() -> Vec<PathBuf> {
    platform::font_directories()
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{RawWindowHandle, WebWindowHandle};

    #[test]
    fn reserved_platforms_report_unsupported_services() {
        let owner = RawWindowHandle::Web(WebWindowHandle::new(1));
        assert_eq!(
            unsupported::choose_font(owner, "x"),
            Err(NativeError::Unsupported("the native font dialog"))
        );
        assert!(unsupported::font_directories().is_empty());
        assert_eq!(
            NativeError::Unsupported("the native font dialog").to_string(),
            "the native font dialog is not implemented on this platform"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_lists_the_system_font_folder_and_rejects_foreign_windows() {
        assert!(font_directories()[0].ends_with("Fonts"));
        let owner = RawWindowHandle::Web(WebWindowHandle::new(1));
        assert!(matches!(
            choose_font(owner, "x"),
            Err(NativeError::Unsupported(_))
        ));
    }
}

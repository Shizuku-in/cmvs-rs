//! macOS font services using the system JXA bridge; no shell or extra packages.
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::{path::PathBuf, process::Command};

const CHOOSER: &str = include_str!("font_dialog.js");

pub fn font_directories() -> Vec<PathBuf> {
    directories(std::env::var_os("HOME").map(PathBuf::from))
}

fn directories(home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = home.filter(|path| path.is_absolute()) {
        paths.push(home.join("Library/Fonts"));
    }
    paths
}

pub fn choose_font(owner: RawWindowHandle, current: &str) -> Result<Option<String>, NativeError> {
    if !matches!(owner, RawWindowHandle::AppKit(_)) {
        return Err(NativeError::Unsupported(
            "the native font dialog for this window kind",
        ));
    }
    // argv carries user data separately from executable script text. The chooser
    // runs in its own process and cannot be attached to the host's NSWindow.
    let output = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", CHOOSER, "--", current])
        .output()
        .map_err(|e| NativeError::Failed(format!("cannot open macOS font chooser: {e}")))?;
    decode_selection(output.status.success(), &output.stdout)
}

fn decode_selection(success: bool, bytes: &[u8]) -> Result<Option<String>, NativeError> {
    if !success || bytes.len() > 4096 {
        return Err(NativeError::Failed(
            "macOS font chooser failed or returned excessive output".into(),
        ));
    }
    let value = std::str::from_utf8(bytes)
        .map_err(|_| NativeError::Failed("font chooser returned invalid UTF-8".into()))?
        .trim_end_matches(['\r', '\n']);
    if value == "CANCEL" {
        return Ok(None);
    }
    match value.strip_prefix("FONT:") {
        Some(name)
            if !name.is_empty() && name.len() <= 512 && !name.chars().any(char::is_control) =>
        {
            Ok(Some(name.to_owned()))
        }
        _ => Err(NativeError::Failed(
            "font chooser returned an invalid family".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_protocol_distinguishes_cancel_from_failure() {
        assert_eq!(decode_selection(true, b"CANCEL\n"), Ok(None));
        assert_eq!(
            decode_selection(true, "FONT:ヒラギノ角ゴシック\n".as_bytes()),
            Ok(Some("ヒラギノ角ゴシック".into()))
        );
        for bytes in [b"FONT:".as_slice(), b"FONT:a\nb", b"unexpected", b"\xff"] {
            assert!(decode_selection(true, bytes).is_err());
        }
        assert!(decode_selection(false, b"CANCEL").is_err());
    }

    #[test]
    fn foreign_windows_are_rejected_without_launching_osascript() {
        assert!(matches!(
            choose_font(
                RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(1)),
                "x"
            ),
            Err(NativeError::Unsupported(_))
        ));
    }

    #[test]
    fn includes_supplemental_fonts_and_optional_user_fonts() {
        assert!(!font_directories().is_empty());
        let system = directories(None);
        assert!(system.contains(&PathBuf::from("/System/Library/Fonts/Supplemental")));
        // Use an absolute path on the test host as well as on macOS.
        let home = std::env::temp_dir();
        let paths = directories(Some(home.clone()));
        assert_eq!(paths.last(), Some(&home.join("Library/Fonts")));
    }
}

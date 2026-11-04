//! macOS font services using the system JXA bridge; no shell or extra packages.
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::{path::PathBuf, process::Command};

const TEXT_EDITOR: &str = include_str!("text_dialog.js");

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

/// The system Standard Additions input dialog runs independently of the host NSWindow.
pub fn edit_text(
    owner: RawWindowHandle,
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
) -> Result<Option<String>, NativeError> {
    if !matches!(owner, RawWindowHandle::AppKit(_)) {
        return Err(NativeError::Unsupported(
            "the native text dialog for this window kind",
        ));
    }
    crate::text_dialog::validate_request(title, prompt, initial)?;
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            TEXT_EDITOR,
            "--",
            title,
            prompt,
            initial,
        ])
        .output()
        .map_err(|e| NativeError::Failed(format!("cannot open macOS text editor: {e}")))?;
    decode_text_result(output.status.success(), &output.stdout, max_chars)
}

fn decode_text_result(
    success: bool,
    bytes: &[u8],
    max_chars: u32,
) -> Result<Option<String>, NativeError> {
    if !success {
        return Err(NativeError::Failed("macOS text dialog failed".into()));
    }
    if matches!(bytes, b"CANCEL" | b"CANCEL\n" | b"CANCEL\r\n") {
        return Ok(None);
    }
    match bytes.strip_prefix(b"TEXT:") {
        Some(text) => crate::text_dialog::decode_text(text, max_chars).map(Some),
        None => Err(NativeError::Failed(
            "macOS text dialog returned an invalid response".into(),
        )),
    }
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
    fn text_dialog_protocol_preserves_comment_and_cancel() {
        assert_eq!(decode_text_result(true, b"CANCEL\n", 50), Ok(None));
        assert_eq!(
            decode_text_result(true, b"TEXT:\n", 50),
            Ok(Some(String::new()))
        );
        assert_eq!(
            decode_text_result(true, b"TEXT:CANCEL\n", 50),
            Ok(Some("CANCEL".into()))
        );
        assert_eq!(
            decode_text_result(true, "TEXT: あい \n".as_bytes(), 50),
            Ok(Some(" あい ".into()))
        );
        assert!(decode_text_result(false, b"CANCEL\n", 50).is_err());
        assert!(decode_text_result(true, b"unexpected", 50).is_err());
        assert!(decode_text_result(true, b"TEXT:\xff", 50).is_err());
        assert!(matches!(
            edit_text(
                RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(1)),
                "",
                "",
                "",
                50
            ),
            Err(NativeError::Unsupported(_))
        ));
    }

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

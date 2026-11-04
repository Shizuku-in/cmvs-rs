//! Zenity comment entry, with all text passed as process arguments, never shell code.
use crate::{
    NativeError,
    text_dialog::{decode_text, validate_request},
};
use raw_window_handle::RawWindowHandle;
use std::process::Command;

fn command(
    owner: RawWindowHandle,
    title: &str,
    prompt: &str,
    initial: &str,
) -> Result<Command, NativeError> {
    if !matches!(
        owner,
        RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) | RawWindowHandle::Wayland(_)
    ) {
        return Err(NativeError::Unsupported(
            "the native text dialog for this window kind",
        ));
    }
    validate_request(title, prompt, initial)?;
    let mut command = Command::new("zenity");
    command
        .args(["--entry", "--no-markup", "--width=520"])
        .arg(format!("--title={title}"))
        .arg(format!("--text={prompt}"))
        .arg(format!("--entry-text={initial}"));
    match owner {
        RawWindowHandle::Xlib(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        RawWindowHandle::Xcb(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        _ => {} // Wayland: independent toplevel, as in the font chooser.
    }
    Ok(command)
}

fn decode(code: Option<i32>, bytes: &[u8], max_chars: u32) -> Result<Option<String>, NativeError> {
    match code {
        Some(0) => decode_text(bytes, max_chars).map(Some),
        Some(1) => Ok(None),
        _ => Err(NativeError::Failed("Zenity text dialog failed".into())),
    }
}

pub fn edit_text(
    owner: RawWindowHandle,
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
) -> Result<Option<String>, NativeError> {
    let output = command(owner, title, prompt, initial)?
        .output()
        .map_err(|e| {
            NativeError::Failed(format!("cannot open text editor (Zenity required): {e}"))
        })?;
    decode(output.status.code(), &output.stdout, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_literal_text_and_attaches_only_real_x11_windows() {
        let owner = RawWindowHandle::Xlib(raw_window_handle::XlibWindowHandle::new(42));
        let command = command(owner, "Title", "<b>Prompt</b>", "--text=$(x); 'あ'").unwrap();
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.contains(&"--entry-text=--text=$(x); 'あ'"));
        assert!(args.contains(&"--no-markup"));
        assert!(args.contains(&"--attach=42"));
        let owner = RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(1));
        assert!(matches!(
            edit_text(owner, "", "", "", 50),
            Err(NativeError::Unsupported(_))
        ));
    }

    #[test]
    fn cancel_and_process_failure_are_distinct_from_empty_accept() {
        assert_eq!(decode(Some(1), b"", 50), Ok(None));
        assert_eq!(decode(Some(0), b"\n", 50), Ok(Some(String::new())));
        assert_eq!(
            decode(Some(0), "あいう\n".as_bytes(), 2),
            Ok(Some("あい".into()))
        );
        assert!(decode(Some(2), b"", 50).is_err());
        assert!(decode(None, b"", 50).is_err());
    }
}

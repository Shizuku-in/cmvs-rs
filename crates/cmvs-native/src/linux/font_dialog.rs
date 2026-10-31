//! Japanese font-family selection through Zenity, without invoking a shell.
use super::fontconfig::fontconfig;
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::{collections::BTreeSet, process::Command};

fn families(output: &str) -> BTreeSet<&str> {
    output
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.starts_with('@') && !name.contains('\0'))
        .collect()
}

pub fn choose_font(owner: RawWindowHandle, current: &str) -> Result<Option<String>, NativeError> {
    if !matches!(
        owner,
        RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) | RawWindowHandle::Wayland(_)
    ) {
        return Err(NativeError::Unsupported(
            "the native font dialog for this window kind",
        ));
    }
    // List families rather than Pango descriptions: the game's FONT stores only a family,
    // with size and boldness controlled separately by its message settings.
    let output = fontconfig("%{family[0]}\n", true)?;
    let names = families(&output);
    if names.is_empty() || names.len() > 8192 {
        return Err(NativeError::Failed(
            "no Japanese font families available, or font list exceeds 8192 entries".into(),
        ));
    }
    let mut command = Command::new("zenity");
    command
        .args([
            "--list",
            "--title=CMVS font",
            "--column=Font family",
            "--print-column=1",
            "--width=520",
            "--height=600",
        ])
        .arg(format!(
            "--text=Current font: {current}\nChoose a font family with Japanese and basic Latin support."
        ));
    match owner {
        RawWindowHandle::Xlib(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        RawWindowHandle::Xcb(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        _ => {} // Zenity manages its own Wayland toplevel; no fabricated parent handle.
    }
    command.arg("--").args(&names);
    let result = command.output().map_err(|e| {
        NativeError::Failed(format!("cannot open font chooser (Zenity required): {e}"))
    })?;
    match result.status.code() {
        Some(1) => Ok(None),
        Some(0) => {
            let selected = String::from_utf8(result.stdout)
                .map_err(|_| NativeError::Failed("font chooser returned invalid UTF-8".into()))?;
            let selected = selected.trim();
            if !names.contains(selected) {
                return Err(NativeError::Failed(
                    "font chooser returned an unlisted family".into(),
                ));
            }
            Ok(Some(selected.to_owned()))
        }
        _ => Err(NativeError::Failed(format!(
            "font chooser failed: {}",
            result.status
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn family_list_is_sorted_deduplicated_and_excludes_vertical_faces() {
        assert_eq!(
            families("Noto Sans CJK JP\n\n@Vertical\nNoto Sans CJK JP\nOther Font\n"),
            BTreeSet::from(["Noto Sans CJK JP", "Other Font"])
        );
    }
    #[test]
    fn foreign_window_is_rejected_before_starting_a_dialog() {
        assert!(matches!(
            choose_font(
                RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(1)),
                "x"
            ),
            Err(NativeError::Unsupported(_))
        ));
    }
}

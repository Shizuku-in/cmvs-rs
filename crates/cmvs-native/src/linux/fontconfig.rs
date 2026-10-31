//! Installed font discovery through Fontconfig, without changing system settings.
use crate::NativeError;
use std::{collections::BTreeSet, path::PathBuf, process::Command};

pub(super) fn fontconfig(format: &str, japanese: bool) -> Result<String, NativeError> {
    let mut command = Command::new("fc-list");
    if japanese {
        // A CJK supplement such as Droid Sans Fallback can lack basic Latin.
        command.arg(":lang=ja:charset=20-7e");
    }
    let output = command
        .args(["--format", format])
        .output()
        .map_err(|e| NativeError::Failed(format!("cannot run Fontconfig fc-list: {e}")))?;
    if !output.status.success() || output.stdout.len() > 4 * 1024 * 1024 {
        return Err(NativeError::Failed(
            "Fontconfig font enumeration failed or exceeded 4 MiB".into(),
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| NativeError::Failed("Fontconfig returned invalid UTF-8".into()))
}

pub fn font_directories() -> Vec<PathBuf> {
    let Ok(output) = fontconfig("%{file}\n", false) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|file| {
            let path = std::path::Path::new(file);
            path.is_absolute()
                .then(|| path.parent().map(PathBuf::from))
                .flatten()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

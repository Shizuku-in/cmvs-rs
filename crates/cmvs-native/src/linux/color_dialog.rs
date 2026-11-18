use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::{io::ErrorKind, process::Command};

pub fn choose_color(
    owner: RawWindowHandle,
    swatches: [u32; 2],
) -> Result<Option<u32>, NativeError> {
    let mut command = Command::new("zenity");
    command.args(["--color-selection", "--show-palette"]);
    command.arg(format!("--color=#{:06x}", swatches[0] & 0xff_ffff));
    match owner {
        RawWindowHandle::Xlib(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        RawWindowHandle::Xcb(handle) => {
            command.arg(format!("--attach={}", handle.window));
        }
        RawWindowHandle::Wayland(_) => {}
        _ => {
            return Err(NativeError::Unsupported(
                "the color dialog for this window kind",
            ));
        }
    }
    let output = command.output().map_err(|error| match error.kind() {
        ErrorKind::NotFound => NativeError::Unsupported("the Zenity color dialog"),
        _ => NativeError::Failed(format!("cannot open Zenity color dialog: {error}")),
    })?;
    match output.status.code() {
        Some(0) => {
            let color = std::str::from_utf8(&output.stdout)
                .ok()
                .and_then(parse_color);
            color
                .map(Some)
                .ok_or_else(|| NativeError::Failed("Zenity returned an invalid color".into()))
        }
        Some(1) => Ok(None),
        _ => Err(NativeError::Failed("Zenity color dialog failed".into())),
    }
}

fn parse_color(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#').filter(|hex| hex.len() == 6) {
        return u32::from_str_radix(hex, 16).ok();
    }
    let text = text.strip_suffix(')')?;
    let rgb = if let Some(rgba) = text.strip_prefix("rgba(") {
        let (rgb, alpha) = rgba.rsplit_once(',')?;
        if !(0.0..=1.0).contains(&alpha.trim().parse::<f32>().ok()?) {
            return None;
        }
        rgb
    } else {
        text.strip_prefix("rgb(")?
    };
    let mut channels = rgb.split(',');
    let [r, g, b] =
        std::array::from_fn(|_| channels.next().and_then(|c| c.trim().parse::<u8>().ok()));
    if channels.next().is_some() {
        return None;
    }
    Some(u32::from_be_bytes([0, r?, g?, b?]))
}

#[cfg(test)]
mod tests {
    use super::parse_color;

    #[test]
    fn reads_zenity_rgb_without_swapping_red_and_blue() {
        assert_eq!(parse_color("rgb(250, 128, 2)\n"), Some(0xfa_8002));
        assert_eq!(parse_color("#fa8002\n"), Some(0xfa_8002));
        assert_eq!(parse_color("rgba(250,128,2,0.5)"), Some(0xfa_8002));
        assert_eq!(parse_color("rgb(256,1,2)"), None);
        assert_eq!(parse_color("rgb(1,2,3,4)"), None);
    }
}

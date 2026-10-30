//! Modal ChooseFontW standing in for the original font dialog of 0x213d (0x140081520),
//! whose list held EnumFontFamiliesExA faces of SHIFTJIS_CHARSET without '@' names.
use std::num::NonZeroIsize;
use windows_sys::Win32::{
    Graphics::Gdi::{LOGFONTW, SHIFTJIS_CHARSET},
    UI::Controls::Dialogs::{
        CF_INITTOLOGFONTSTRUCT, CF_NOVERTFONTS, CF_SCREENFONTS, CF_SELECTSCRIPT, CF_TTONLY,
        CHOOSEFONTW, ChooseFontW, CommDlgExtendedError,
    },
};

/// LOGFONT face names hold 31 UTF-16 units and a terminator.
const FACE_UNITS: usize = 32;

/// Shows the dialog owned by `owner`, starting from `current`. The script set is fixed to
/// Shift-JIS and vertical faces are hidden, matching the original list; only TrueType
/// faces are offered because the preview renderer reads outline fonts. Ok(None) means
/// the user cancelled.
pub(super) fn choose_font(owner: NonZeroIsize, current: &str) -> Result<Option<String>, String> {
    let mut font = LOGFONTW {
        lfCharSet: SHIFTJIS_CHARSET,
        ..Default::default()
    };
    let units: Vec<u16> = current.encode_utf16().collect();
    if units.len() >= FACE_UNITS {
        return Err("current font face exceeds a LOGFONT name".into());
    }
    font.lfFaceName[..units.len()].copy_from_slice(&units);
    let mut dialog = CHOOSEFONTW {
        lStructSize: size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: owner.get() as _,
        lpLogFont: &mut font,
        Flags: CF_INITTOLOGFONTSTRUCT
            | CF_SCREENFONTS
            | CF_SELECTSCRIPT
            | CF_NOVERTFONTS
            | CF_TTONLY,
        ..Default::default()
    };
    // SAFETY: `dialog` is fully initialized with its own size, points at `font`, which
    // outlives the modal call, and names a window handle supplied by the caller.
    #[allow(unsafe_code)]
    let chosen = unsafe { ChooseFontW(&mut dialog) } != 0;
    if !chosen {
        // SAFETY: reads the calling thread's last common-dialog error; no arguments.
        #[allow(unsafe_code)]
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(format!(
                "ChooseFontW failed with common dialog error {error:#x}"
            ))
        };
    }
    let end = font
        .lfFaceName
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(FACE_UNITS);
    String::from_utf16(&font.lfFaceName[..end])
        .map(Some)
        .map_err(|_| "ChooseFontW returned an invalid face name".into())
}

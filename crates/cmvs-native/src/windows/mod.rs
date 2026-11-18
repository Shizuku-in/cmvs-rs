//! Win32 implementations.
pub(crate) mod cursor;
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

pub fn choose_color(
    owner: RawWindowHandle,
    swatches: [u32; 2],
) -> Result<Option<u32>, NativeError> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CC_RGBINIT, CHOOSECOLORW, ChooseColorW, CommDlgExtendedError,
    };
    let RawWindowHandle::Win32(handle) = owner else {
        return Err(NativeError::Unsupported(
            "the color dialog for a non-Win32 window",
        ));
    };
    let to_colorref = |rgb: u32| (rgb & 0xff_ffff).swap_bytes() >> 8;
    let mut custom = [0; 16];
    custom[..2].copy_from_slice(&swatches.map(to_colorref));
    let mut dialog = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: handle.hwnd.get() as _,
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT,
        ..Default::default()
    };
    // SAFETY: the modal call borrows the initialized structure and its 16 writable swatches.
    #[allow(unsafe_code)]
    if unsafe { ChooseColorW(&mut dialog) } != 0 {
        return Ok(Some(to_colorref(dialog.rgbResult)));
    }
    // SAFETY: reads the calling thread's common-dialog error.
    #[allow(unsafe_code)]
    let error = unsafe { CommDlgExtendedError() };
    if error == 0 {
        Ok(None)
    } else {
        Err(NativeError::Failed(format!(
            "ChooseColorW failed with common dialog error {error:#x}"
        )))
    }
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

mod text_dialog;
pub fn edit_text(
    owner: RawWindowHandle,
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
) -> Result<Option<String>, NativeError> {
    let RawWindowHandle::Win32(handle) = owner else {
        return Err(NativeError::Unsupported(
            "the text dialog for a non-Win32 window",
        ));
    };
    text_dialog::edit_text(handle.hwnd.get() as _, title, prompt, initial, max_chars)
        .map_err(NativeError::Failed)
}

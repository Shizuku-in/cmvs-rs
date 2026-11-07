//! System IME and message dialogs, modal on the window's thread. Both draw over the
//! last presented frame, so they need an open [`super::VitaWindow`].
#![allow(unsafe_code)]
use super::{
    text::{truncate, utf16},
    window::OPEN,
};
use std::{ffi::CString, sync::atomic::Ordering};

unsafe extern "C" {
    fn cmvs_vita_ime(
        title: *const u16,
        initial: *const u16,
        max_chars: u32,
        out: *mut u16,
        out_len: u32,
    ) -> i32;
    fn cmvs_vita_message(text: *const std::ffi::c_char) -> i32;
}

/// SCE_IME_DIALOG_MAX_TITLE_LENGTH.
const TITLE_UNITS: usize = 128;
/// SCE_MSG_DIALOG_USER_MSG_SIZE, including the terminator.
const MESSAGE_BYTES: usize = 512;

/// Shows `text` with an OK button until dismissed. False when it could not be shown;
/// text past the dialog's 511 bytes is cut.
pub fn show_message(text: &str) -> bool {
    if !OPEN.load(Ordering::Acquire) {
        return false;
    }
    let text = text.replace('\0', " ");
    let text = CString::new(truncate(&text, MESSAGE_BYTES - 1)).expect("no interior NUL");
    // SAFETY: NUL-terminated within the dialog limit; outlives the blocking call.
    unsafe { cmvs_vita_message(text.as_ptr()) == 0 }
}

/// Shows the IME with `title` and `prompt` as its heading, limited to `max_chars`
/// (1..=127). `check` runs on OK: an error is shown in a message dialog and the IME
/// reopens with the entered text. Ok(None) means the user cancelled.
pub fn ime_dialog(
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
    mut check: impl FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, &'static str> {
    if !(1..=127).contains(&max_chars) {
        return Err("IME limit must be 1..=127 characters");
    }
    if !OPEN.load(Ordering::Acquire) {
        return Err("the IME needs an open Vita window");
    }
    let heading = match (title.is_empty(), prompt.is_empty()) {
        (false, false) => format!("{title} — {prompt}"),
        (true, _) => prompt.to_owned(),
        (false, true) => title.to_owned(),
    };
    let heading = utf16(&heading, TITLE_UNITS);
    let limit = max_chars as usize;
    let mut text = initial.to_owned();
    loop {
        let initial = utf16(&text, limit);
        let mut out = vec![0u16; limit + 1];
        // SAFETY: both inputs are NUL-terminated and outlive the call; `out` holds
        // max_chars + 1 units, the C side's precondition.
        let status = unsafe {
            cmvs_vita_ime(
                heading.as_ptr(),
                initial.as_ptr(),
                max_chars,
                out.as_mut_ptr(),
                out.len() as u32,
            )
        };
        match status {
            0 => return Ok(None),
            1 => {}
            _ => return Err("IME dialog failed"),
        }
        let end = out.iter().position(|&unit| unit == 0).unwrap_or(out.len());
        text = String::from_utf16(&out[..end]).map_err(|_| "IME returned invalid UTF-16")?;
        match check(&text) {
            Ok(()) => return Ok(Some(text)),
            Err(why) => {
                if !show_message(&why) {
                    return Err("IME check message could not be shown");
                }
            }
        }
    }
}

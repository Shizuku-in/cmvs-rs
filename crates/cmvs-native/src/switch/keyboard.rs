//! System software keyboard (swkbd library applet), modal on the calling thread.
#![allow(unsafe_code)]
use std::ffi::{CStr, CString, c_char, c_void};

type Check<'a> = &'a mut dyn FnMut(&str) -> Result<(), String>;
type RawCheck = unsafe extern "C" fn(*const c_char, *mut c_char, usize, *mut c_void) -> i32;

unsafe extern "C" {
    fn cmvs_nx_keyboard(
        header: *const c_char,
        sub: *const c_char,
        initial: *const c_char,
        max_chars: u32,
        check: RawCheck,
        user: *mut c_void,
        out: *mut c_char,
        out_size: usize,
    ) -> i32;
}

/// Output, and the check callback's shared input buffer: 127 characters of up to
/// four UTF-8 bytes fit with room to spare.
const BUFFER: usize = 1024;

/// SAFETY: `user` is the `*mut Check` passed by `software_keyboard`, live for the whole
/// modal call; `text` is NUL-terminated; `message` holds `capacity` bytes.
unsafe extern "C" fn check_text(
    text: *const c_char,
    message: *mut c_char,
    capacity: usize,
    user: *mut c_void,
) -> i32 {
    let check = unsafe { &mut *user.cast::<Check<'_>>() };
    let result = match unsafe { CStr::from_ptr(text) }.to_str() {
        Ok(text) => check(text),
        Err(_) => Err("invalid UTF-8 input".into()),
    };
    let Err(why) = result else {
        return 0;
    };
    if capacity == 0 {
        return 1;
    }
    let mut end = why.len().min(capacity - 1);
    while !why.is_char_boundary(end) {
        end -= 1;
    }
    // SAFETY: `end + 1 <= capacity` bytes are written into the callback buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(why.as_ptr(), message.cast::<u8>(), end);
        *message.add(end) = 0;
    }
    1
}

/// Shows the keyboard with `title` as header and `prompt` beneath it, limited to
/// `max_chars` (1..=127). `check` runs on OK: an error message keeps the keyboard open
/// and is shown to the user. Ok(None) means the user cancelled.
pub fn software_keyboard(
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
    mut check: impl FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, &'static str> {
    if !(1..=127).contains(&max_chars) {
        return Err("software keyboard limit must be 1..=127 characters");
    }
    let text = |s: &str| CString::new(s).map_err(|_| "keyboard text contains NUL");
    let (title, prompt, initial) = (text(title)?, text(prompt)?, text(initial)?);
    let mut out = vec![0u8; BUFFER];
    let mut check: Check<'_> = &mut check;
    // SAFETY: all strings are NUL-terminated and outlive the call; `out` holds BUFFER
    // bytes; C clears the callback before returning.
    let status = unsafe {
        cmvs_nx_keyboard(
            title.as_ptr(),
            prompt.as_ptr(),
            initial.as_ptr(),
            max_chars,
            check_text,
            (&raw mut check).cast(),
            out.as_mut_ptr().cast(),
            out.len(),
        )
    };
    match status {
        0 => Ok(None),
        1 => {
            let end = out.iter().position(|&b| b == 0).unwrap_or(out.len());
            out.truncate(end);
            String::from_utf8(out)
                .map(Some)
                .map_err(|_| "software keyboard returned invalid UTF-8")
        }
        _ => Err("software keyboard failed"),
    }
}

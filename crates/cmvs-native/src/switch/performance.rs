//! Applet service requests (platforms/switch/runtime/system.c): CPU boost and the
//! error dialog.
#![allow(unsafe_code)]
use std::ffi::CString;

unsafe extern "C" {
    fn cmvs_nx_set_cpu_boost(enabled: i32) -> i32;
    fn cmvs_nx_show_error(dialog: *const std::ffi::c_char, details: *const std::ffi::c_char)
    -> i32;
}

/// Shows the error applet with a short `dialog` text and `details` behind its Details
/// button, blocking until dismissed. Returns false when it could not be shown.
pub fn show_system_error(dialog: &str, details: &str) -> bool {
    let text = |s: &str| CString::new(s.replace('\0', " ")).expect("no interior NUL");
    let (dialog, details) = (text(dialog), text(details));
    // SAFETY: both strings are NUL-terminated and outlive the blocking call.
    unsafe { cmvs_nx_show_error(dialog.as_ptr(), details.as_ptr()) == 0 }
}

/// Requests FastLoad boost (CPU 1785 MHz, GPU at its minimum) or the normal clocks.
/// Returns false where the system refuses, e.g. in applet mode or before 7.0.0.
pub fn set_cpu_boost(enabled: bool) -> bool {
    // SAFETY: a plain applet service call without pointers.
    unsafe { cmvs_nx_set_cpu_boost(i32::from(enabled)) == 0 }
}

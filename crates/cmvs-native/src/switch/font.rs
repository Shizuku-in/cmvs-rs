//! System shared fonts.
#![allow(unsafe_code)]

unsafe extern "C" {
    fn cmvs_nx_shared_font(out: *mut u8, capacity: usize) -> usize;
}

/// The system's Japan/US/Europe TrueType font, or None when pl does not provide it.
pub fn shared_font() -> Option<Vec<u8>> {
    // SAFETY: a null output only queries the size.
    let size = unsafe { cmvs_nx_shared_font(std::ptr::null_mut(), 0) };
    if size == 0 {
        return None;
    }
    let mut bytes = vec![0u8; size];
    // SAFETY: C writes at most `capacity` bytes, and only when the font fits.
    let copied = unsafe { cmvs_nx_shared_font(bytes.as_mut_ptr(), bytes.len()) };
    (copied == size).then_some(bytes)
}

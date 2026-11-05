//! CPU boost requests through applet services (platforms/switch/runtime/system.c).
#![allow(unsafe_code)]

unsafe extern "C" {
    fn cmvs_nx_set_cpu_boost(enabled: i32) -> i32;
}

/// Requests FastLoad boost (CPU 1785 MHz, GPU at its minimum) or the normal clocks.
/// Returns false where the system refuses, e.g. in applet mode or before 7.0.0.
pub fn set_cpu_boost(enabled: bool) -> bool {
    // SAFETY: a plain applet service call without pointers.
    unsafe { cmvs_nx_set_cpu_boost(i32::from(enabled)) == 0 }
}

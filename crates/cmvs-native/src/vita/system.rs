//! Clock and scheduling requests (platforms/psvita/runtime/system.c).
#![allow(unsafe_code)]

unsafe extern "C" {
    fn cmvs_vita_set_max_clocks() -> i32;
    fn cmvs_vita_prefer_core(core: i32) -> i32;
}

/// Raises the CPU to 444 MHz with the bus and GPU at their application maximum until
/// the process exits. False when the system refuses.
pub fn set_max_clocks() -> bool {
    // SAFETY: plain power service calls without pointers.
    unsafe { cmvs_vita_set_max_clocks() == 0 }
}

/// Moves the calling thread to user core 0, 1 or 2; false leaves it unchanged.
pub fn prefer_core(core: u32) -> bool {
    // SAFETY: changes only the calling thread's affinity.
    core <= 2 && unsafe { cmvs_vita_prefer_core(core as i32) == 0 }
}

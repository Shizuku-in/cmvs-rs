//! Deterministic MSVC-compatible pseudo-random values and VM commands.
//!
//! Choose an explicit seed for repeatable runs. The runtime also advances the
//! generator per frame, so instruction order alone does not determine the sequence.

// MSVC CRT rand() at 0x1400c77b4: LCG 214013/2531011 with 15-bit output.
// The original seeds from timeGetTime and draws once per main-loop frame.
use crate::{bytecode::DecodeError, dispatch::Call};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrtRandom {
    state: u32,
}

/// Wall-clock milliseconds truncated to `u32`, matching the original `timeGetTime()`
/// startup seed for interactive hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn host_time_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u32)
}

impl CrtRandom {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }
    pub fn next_value(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(214013).wrapping_add(2531011);
        (self.state >> 16) & 0x7fff
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_>) -> Result<(), DecodeError> {
        let divisor = call.argument(4, "divisor")? as i32;
        if divisor == 0 {
            return Err(call.error("rand() % 0 faults in the original"));
        }
        let mut next = self.clone();
        // rand() is non-negative, so the C remainder never overflows and stays non-negative.
        let value = next.next_value() as i32 % divisor;
        call.complete(0x4004)?;
        call.vm
            .state_mut()
            .set_system_integer(0, value as u32)
            .expect("slot 0 exists");
        *self = next;
        Ok(())
    }
}

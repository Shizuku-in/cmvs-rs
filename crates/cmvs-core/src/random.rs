//! MSVC CRT rand() (statically linked at 0x1400c77b4): LCG 214013/2531011 with 15-bit output.
//! The original seeds it from timeGetTime and also draws once per main-loop frame, so values are
//! reproducible only through an explicit research seed; the generator and command semantics match.
use crate::{bytecode::DecodeError, dispatch::Call, machine::Machine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrtRandom {
    state: u32,
}

impl CrtRandom {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }
    pub fn next_value(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(214013).wrapping_add(2531011);
        (self.state >> 16) & 0x7fff
    }
    /// 0x20d4 / 0x140074b10: system result 0 = rand() % SP-4, returning 0x4004.
    /// The generator advances only when the command completes.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0xd4)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(&mut Call::new(vm, pc, 0xd4))?;
        Ok(true)
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
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

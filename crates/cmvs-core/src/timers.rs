//! Ten cumulative VM timers at engine +14748 (snapshot record 0x109), and the
//! independent 0x200e/0x200f stopwatch at +0x4160.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const TIMER_SLOTS: usize = 10;

#[derive(Debug, Default)]
pub struct VmTimers {
    values: [u32; TIMER_SLOTS],
    clock: u32,
    marked_at: u32,
}
impl VmTimers {
    pub fn values(&self) -> &[u32; TIMER_SLOTS] {
        &self.values
    }
    /// Record 0x109 restore: the reader copies +14748..+14788 verbatim.
    pub(crate) fn restore_values(&mut self, values: [u32; TIMER_SLOTS]) {
        self.values = values;
    }
    /// Explicit wrapping millisecond clock corresponding to 0x140008bc0.
    pub fn clock(&self) -> u32 {
        self.clock
    }
    /// 0x140067930 advances these once on entry, before callback dispatch/instructions.
    /// A cooperative execution slice is not a new VM run.
    pub fn begin_run(&mut self, delta_ms: u32) {
        self.clock = self.clock.wrapping_add(delta_ms);
        for value in &mut self.values {
            *value = value.wrapping_add(delta_ms);
        }
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x0b | 0x0c | 0x0e | 0x0f))) = vm.pending_system() else {
            return Ok(false);
        };
        // 0x1400824f0/0x140082506: no arguments, independent of the ten timer slots.
        if matches!(command, 0x0e | 0x0f) {
            vm.complete_system(SystemResult(0x4000))?;
            if command == 0x0e {
                self.marked_at = self.clock;
            } else {
                vm.state_mut()
                    .set_system_integer(0, self.clock.wrapping_sub(self.marked_at))
                    .expect("slot 0 exists");
            }
            return Ok(true);
        }
        let fail = |why| error(pc, command + 0x2000, why);
        let slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))? as usize;
        let value = *self
            .values
            .get(slot)
            .ok_or_else(|| fail("timer slot outside 0..9"))?;
        vm.complete_system(SystemResult(0x4004))?;
        if command == 0x0b {
            self.values[slot] = 0;
        } else {
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

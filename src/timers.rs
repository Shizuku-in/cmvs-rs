//! Ten cumulative VM timers at engine +14748 (snapshot record 0x109).
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const TIMER_SLOTS: usize = 10;

#[derive(Debug, Default)]
pub struct VmTimers {
    values: [u32; TIMER_SLOTS],
}
impl VmTimers {
    pub fn values(&self) -> &[u32; TIMER_SLOTS] {
        &self.values
    }
    /// 0x140067930 advances these once on entry, before callback dispatch/instructions.
    /// A cooperative execution slice is not a new VM run.
    pub fn begin_run(&mut self, delta_ms: u32) {
        for value in &mut self.values {
            *value = value.wrapping_add(delta_ms);
        }
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x0b | 0x0c))) = vm.pending_system() else {
            return Ok(false);
        };
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

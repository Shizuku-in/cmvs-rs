//! Integer helpers 0x20d0..0x20d3, inline in the system dispatcher (0x140082db4..
//! 0x140082e7f); addresses refer to the documented binary hash. Arguments are signed and the
//! result goes to system result 0.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// 0x20d0: |SP-4| (cdq/xor/sub, so i32::MIN stays), 0x4004. 0x20d1: SP-4 raised to at least
/// SP-8, then lowered to at most SP-12 (the upper bound wins), 0x400c. 0x20d2 / 0x20d3: the
/// larger / smaller of SP-4 and SP-8, 0x4008.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, command @ 0xd0..=0xd3)) = vm.pending_system() else {
        return Ok(false);
    };
    let count = match command {
        0xd0 => 1,
        0xd1 => 3,
        _ => 2,
    };
    let mut args = [0i32; 3];
    for (i, arg) in args.iter_mut().take(count).enumerate() {
        *arg = vm
            .stack()
            .peek_u32(4 * (i + 1))
            .map_err(|e| error(pc, command + 0x2000, e.0))? as i32;
    }
    let [a, b, c] = args;
    let value = match command {
        0xd0 => a.wrapping_abs(),
        0xd1 => a.max(b).min(c),
        0xd2 => a.max(b),
        _ => a.min(b),
    };
    vm.complete_system(SystemResult(0x4000 | (4 * count) as u16))?;
    vm.state_mut()
        .set_system_integer(0, value as u32)
        .expect("slot 0 exists");
    Ok(true)
}

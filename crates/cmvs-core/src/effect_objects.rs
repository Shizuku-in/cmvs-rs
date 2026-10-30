//! Engine +4024: four effect objects. Only 0x2290..0x2297 and 0x22a1..0x22a9 create them
//! and none of those is implemented, so every slot stays empty here. Addresses refer to the
//! binary hash in research.md.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// 0x22a0 / 0x14007dbe0: SP-4 slot 0..3 destroys and frees that object (0x140040820) when
/// present; other values are ignored. Returns 0x4004 without a system result.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, 0x2a0)) = vm.pending_system() else {
        return Ok(false);
    };
    vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22a0, e.0))?;
    vm.complete_system(SystemResult(0x4004))?;
    Ok(true)
}

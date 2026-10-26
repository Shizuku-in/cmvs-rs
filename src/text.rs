//! Stateless string utility commands of the 0x20f0..0x20ff group; only verified ones are handled.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// 0x20f8 / 0x1400747c0: system result 0 = lstrlenA of the SP-4 string reference.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, 0xf8)) = vm.pending_system() else {
        return Ok(false);
    };
    let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x20f8, e.0))?;
    let length = vm.resolve_string(raw)?.len() as u32;
    vm.complete_system(SystemResult(0x4004))?;
    vm.state_mut()
        .set_system_integer(0, length)
        .expect("slot 0 exists");
    Ok(true)
}

//! Script-controlled skip flag at engine +1520; addresses refer to the documented binary hash.
//! The constructor (0x140014e60), reset 0x2003 (0x140080700) and save loading (0x140081960)
//! clear it. No configuration or menu helper writes it, unlike the READ_SKIP setting at +1524.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkipFlag {
    active: bool,
}
impl SkipFlag {
    /// Read by 0x2224's input-skip branch and 0x22ef, which also need input and stay unhandled.
    pub fn active(&self) -> bool {
        self.active
    }
    /// 0x22eb / 0x14007c890: nonzero SP-4 sets the flag to bool(SP-8); zero SP-4 copies the
    /// flag to system result 0 without reading SP-8. Both pop 8 bytes (0x4008).
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x2eb)) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x22eb, e.0))
        };
        let value = if arg(4)? != 0 {
            Some(arg(8)? != 0)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4008))?;
        match value {
            Some(active) => self.active = active,
            None => vm
                .state_mut()
                .set_system_integer(0, u32::from(self.active))
                .expect("slot 0 exists"),
        }
        Ok(true)
    }
}

//! Script-controlled message progression flags; addresses refer to the documented binary hash.
//! Both live in the +1496 settings block that message objects read (e.g. 0x14005e220).
//! The constructor (0x140014e60), reset 0x2003 (0x140080700) and save loading (0x140081960)
//! clear them; no configuration helper writes either one.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MessageFlags {
    skip: bool,
    auto_advance: bool,
}
impl MessageFlags {
    /// +1520, 0x22eb. Also read by 0x2224's input-skip branch and 0x22ef, which need input.
    pub fn skip(&self) -> bool {
        self.skip
    }
    /// +1552, 0x22e5; the 自動送りを実行 checkbox (control 1018) of dialog 138 also edits it.
    pub fn auto_advance(&self) -> bool {
        self.auto_advance
    }
    /// 0x22eb / 0x14007c890 and 0x22e5 / 0x14007c5d0: nonzero SP-4 sets the flag to
    /// bool(SP-8); zero SP-4 copies it to system result 0 without reading SP-8. Both pop 8 bytes.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x2eb | 0x2e5))) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, command + 0x2000, e.0))
        };
        let value = if arg(4)? != 0 {
            Some(arg(8)? != 0)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4008))?;
        let flag = if command == 0x2eb {
            &mut self.skip
        } else {
            &mut self.auto_advance
        };
        match value {
            Some(active) => *flag = active,
            None => vm
                .state_mut()
                .set_system_integer(0, u32::from(*flag))
                .expect("slot 0 exists"),
        }
        Ok(true)
    }
}

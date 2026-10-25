//! Platform-independent display state set by verified scene commands.
//! Renderers read it each frame; this module draws nothing and owns no device.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// CD3DDevice +84/+80; constructor 0x14001d5d0 and reset 0x140080700 both zero them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearState {
    /// 0x22cb keeps the raw u32; frames clear to opaque 0x00RRGGBB from its low 24 bits.
    pub color: u32,
    /// 0x2250: also clear depth to 1.0, not only the target.
    pub depth: bool,
}
impl ClearState {
    pub fn rgba(&self) -> [u8; 4] {
        let [b, g, r, _] = self.color.to_le_bytes();
        [r, g, b, 255]
    }
}

#[derive(Debug, Default)]
pub struct SceneHost {
    clear: ClearState,
}
impl SceneHost {
    pub fn clear(&self) -> ClearState {
        self.clear
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if !matches!(command, 0x2cb | 0x250) {
            return Ok(false);
        }
        let value = vm
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, command + 0x2000, e.0))?;
        vm.complete_system(SystemResult(0x4004))?;
        if command == 0x2cb {
            self.clear.color = value;
        } else {
            self.clear.depth = value != 0;
        }
        Ok(true)
    }
}

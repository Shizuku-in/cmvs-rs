//! Runtime configuration commands, researched against cmvs-20160201 x64.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// Shift-JIS `ＭＳ ゴシック`, the initialized global at 0x1400fc150.
pub const DEFAULT_FONT_FACE: &[u8] = b"\x82l\x82r \x83S\x83V\x83b\x83N";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsHost {
    font_face: Vec<u8>,
    /// +1556, +1560, +1568, +1572. Consumers of automatic advance remain a barrier.
    auto_fields: [u32; 4],
}
impl Default for SettingsHost {
    fn default() -> Self {
        Self {
            font_face: DEFAULT_FONT_FACE.to_vec(),
            auto_fields: [10, 0, 0, 1],
        }
    }
}
impl SettingsHost {
    pub fn font_face(&self) -> &[u8] {
        &self.font_face
    }
    /// 0x22de / 0x14007c2d0: SP-4 selects write/read, SP-8 is a string reference.
    /// Read writes the global font name to the destination and result 0 = 1.
    /// Native font selection is still a barrier; accepting another face without
    /// changing subsequent font construction would misrepresent the command.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if let Some((pc, command)) = vm.pending_system() {
            let auto_index = match command {
                0x2e7 => Some(0),
                0x2e8 => Some(1),
                0x2f8 => Some(2),
                0x2e6 => Some(3),
                _ => None,
            };
            if let Some(index) = auto_index {
                let mode = vm
                    .stack()
                    .peek_u32(4)
                    .map_err(|e| error(pc, command + 0x2000, e.0))?;
                let value = if mode != 0 {
                    let raw = vm
                        .stack()
                        .peek_u32(8)
                        .map_err(|e| error(pc, command + 0x2000, e.0))?;
                    Some(if index == 3 { u32::from(raw != 0) } else { raw })
                } else {
                    None
                };
                vm.complete_system(SystemResult(0x4008))?;
                if let Some(value) = value {
                    self.auto_fields[index] = value;
                } else {
                    vm.state_mut()
                        .set_system_integer(0, self.auto_fields[index])
                        .expect("slot 0 exists");
                }
                return Ok(true);
            }
            // Constructor 0x140014e60. Setters require their consumers and backend
            // effects before they may be accepted; these reads expose current defaults.
            let value = match command {
                0x2df => Some(1),
                0x2e0 => Some(0),
                0x2e1 => Some(4),
                0x2e2 => Some(1),
                0x2e3 => Some(2),
                0x2e4 => Some(0),
                0x2e9 => Some(1),
                _ => None,
            };
            if let Some(value) = value {
                let mode = vm
                    .stack()
                    .peek_u32(4)
                    .map_err(|e| error(pc, command + 0x2000, e.0))?;
                if mode != 0 {
                    return Err(error(
                        pc,
                        command + 0x2000,
                        "runtime configuration setter is not implemented",
                    ));
                }
                // Bounded integer wrappers load SP-8 even on query.
                if matches!(command, 0x2e1..=0x2e3) {
                    vm.stack()
                        .peek_u32(8)
                        .map_err(|e| error(pc, command + 0x2000, e.0))?;
                }
                vm.complete_system(SystemResult(0x4008))?;
                vm.state_mut()
                    .set_system_integer(0, value)
                    .expect("slot 0 exists");
                return Ok(true);
            }
        }
        let Some((pc, 0x2de)) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x22de, e.0))
        };
        let (write, reference) = (arg(4)? != 0, arg(8)?);
        if write {
            if vm.resolve_string(reference)? != self.font_face {
                return Err(error(
                    pc,
                    0x22de,
                    "changing the default font face is not implemented",
                ));
            }
            vm.complete_system(SystemResult(0x4008))?;
        } else {
            let slot = crate::text::bank_slot(reference).ok_or_else(|| {
                error(
                    pc,
                    0x22de,
                    "font name destination outside the string bank is not modeled",
                )
            })?;
            vm.complete_system(SystemResult(0x4008))?;
            vm.state_mut()
                .set_string(slot, &self.font_face)
                .expect("bounded font name and valid slot");
            vm.state_mut()
                .set_system_integer(0, 1)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

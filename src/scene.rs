//! Platform-independent display state set by verified scene commands.
//! Renderers read it each frame; this module draws nothing and owns no device.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};
use std::collections::BTreeMap;

/// Engine +1976 holds 256 object pointers (snapshot records 0x700).
pub const OBJECT_SLOTS: usize = 256;

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

/// A 0x38c8-byte object freshly built by 0x14003a3c0: a parent pointer, 96 zeroed 64-byte
/// entries and counters. Its fields join the model as property commands are verified.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneObject {}

#[derive(Debug, Default)]
pub struct SceneHost {
    clear: ClearState,
    objects: BTreeMap<u8, SceneObject>,
}
impl SceneHost {
    pub fn clear(&self) -> ClearState {
        self.clear
    }
    pub fn object(&self, slot: u8) -> Option<&SceneObject> {
        self.objects.get(&slot)
    }
    pub fn objects(&self) -> &BTreeMap<u8, SceneObject> {
        &self.objects
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if !matches!(command, 0x2cb | 0x250 | 0x20 | 0x21) {
            return Ok(false);
        }
        let op = command + 0x2000;
        let value = vm.stack().peek_u32(4).map_err(|e| error(pc, op, e.0))?;
        let slot = if matches!(command, 0x20 | 0x21) {
            // 0x14006bb00/0x14006bbc0 compare signed against 256: larger values only set
            // +12056 bit 0x10 (unmodeled) and negative values would index before the table.
            match u8::try_from(value as i32) {
                Ok(slot) => Some(slot),
                Err(_) => return Err(error(pc, op, "object slot outside 0..255")),
            }
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4004))?;
        match (command, slot) {
            (0x2cb, _) => self.clear.color = value,
            (0x250, _) => self.clear.depth = value != 0,
            // 0x2020 destroys any existing object in the slot before building a new one.
            (0x20, Some(slot)) => {
                self.objects.insert(slot, SceneObject::default());
            }
            (_, Some(slot)) => {
                self.objects.remove(&slot);
            }
            _ => unreachable!("slot is parsed for object commands"),
        }
        Ok(true)
    }
}

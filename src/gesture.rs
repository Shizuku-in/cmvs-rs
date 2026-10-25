//! Engine +4664 pointer drag/flick recognizer (constructor 0x1400533d0): script configuration only.
//! The original samples the mouse inside up to six rectangles each frame; nothing is recognized here.
//! Fields stay named by offset until their input semantics are verified.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};
use std::collections::BTreeMap;

/// Tracking fields reset by every setter that shares the tail of 0x1400548e0.
const RESET_MINUS_ONE: [u16; 3] = [44, 88, 172];
const RESET_ZERO: [u16; 11] = [48, 52, 72, 76, 80, 84, 92, 108, 120, 180, 184];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerGesture {
    fields: BTreeMap<u16, u32>,
}

impl Default for PointerGesture {
    /// Constructor values for every field the modeled commands touch.
    fn default() -> Self {
        let mut fields = BTreeMap::from([(32, 64), (36, 90)]);
        for offset in [0, 4, 8, 12, 16, 20, 24, 28].into_iter().chain(RESET_ZERO) {
            fields.insert(offset, 0);
        }
        for offset in RESET_MINUS_ONE {
            fields.insert(offset, u32::MAX);
        }
        Self { fields }
    }
}

impl PointerGesture {
    pub fn field(&self, offset: u16) -> Option<u32> {
        self.fields.get(&offset).copied()
    }
    pub fn fields(&self) -> &BTreeMap<u16, u32> {
        &self.fields
    }
    fn reset_tracking(&mut self) {
        for offset in RESET_MINUS_ONE {
            self.fields.insert(offset, u32::MAX);
        }
        for offset in RESET_ZERO {
            self.fields.insert(offset, 0);
        }
    }
    /// 0x2350..0x2364 setters (0x14007fb60..0x14007fe60) on the +4664 object.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        let flag_field = match command {
            0x350 => Some(0),
            0x352 => Some(8),
            0x356 => Some(12),
            0x357 => Some(16),
            0x358 => Some(20),
            0x351 | 0x35a | 0x35b | 0x364 => None,
            _ => return Ok(false),
        };
        let op = command + 0x2000;
        let value = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let first = value(4)?;
        let second = if command == 0x351 { value(8)? } else { 0 };
        let flags = if command == 0x351 { 0x4008 } else { 0x4004 };
        vm.complete_system(SystemResult(flags))?;
        match (command, flag_field) {
            (_, Some(offset)) => {
                self.fields.insert(offset, u32::from(first != 0));
                self.reset_tracking();
            }
            (0x351, _) => {
                self.fields.insert(24, u32::from(first != 0));
                self.fields.insert(28, u32::from(second != 0));
                self.reset_tracking();
            }
            (0x364, _) => {
                self.fields.insert(4, first);
                self.reset_tracking();
            }
            // 0x1400535f0 accepts 1..=90 and 0x1400535e0 non-negative values; neither resets.
            (0x35a, _) if (1..=90).contains(&first) => {
                self.fields.insert(36, first);
            }
            (0x35b, _) if first as i32 >= 0 => {
                self.fields.insert(32, first);
            }
            _ => {}
        }
        Ok(true)
    }
}

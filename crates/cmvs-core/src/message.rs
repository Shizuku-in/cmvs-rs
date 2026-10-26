//! Message windows; addresses refer to the documented binary hash.
//!
//! Engine +4064 holds eight message window pointers (0x9f0-byte objects, constructor
//! 0x14005cf70) and +4128 twelve text objects (0x250 bytes, constructor 0x14005c550); the
//! engine constructor 0x140014e60 zeroes both tables. 0x2140 / 0x140075a50 creates a window
//! whose first word (+0) is its text object index (SP-8, below 12). Creation is not modeled,
//! so every window slot is empty here; only the destruction 0x2141 is handled.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const MESSAGE_WINDOWS: usize = 8;
pub const TEXT_OBJECTS: usize = 12;

#[derive(Debug, Default)]
pub struct MessageHost {
    /// +4064: the text object index stored at window +0, for each present window.
    windows: [Option<usize>; MESSAGE_WINDOWS],
    /// +4128: whether the text object pointer is non-null.
    texts: [bool; TEXT_OBJECTS],
}

impl MessageHost {
    /// The text object index of the window in `slot`, if that window exists.
    pub fn window_text(&self, slot: usize) -> Option<usize> {
        self.windows.get(slot).copied().flatten()
    }
    pub fn text_present(&self, index: usize) -> bool {
        self.texts.get(index).copied().unwrap_or(false)
    }
    /// 0x2141 / 0x140075f70: SP-4 slot compared signed against 8. A present window first
    /// frees the text object its +0 word names (destructor 0x14005c710) and clears that
    /// entry, then frees itself (0x14005e1e0 releases its +2520 scene object, which is not
    /// in the +1976 table) and clears the slot; empty slots change nothing. Returns 0x4004.
    /// Slots >= 8 only set +12056 bit 0x10000 (unmodeled); negative slots would index
    /// before the table.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x141)) = vm.pending_system() else {
            return Ok(false);
        };
        let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2141, e.0))? as i32;
        if slot < 0 {
            return Err(error(
                pc,
                0x2141,
                "negative message window slot indexes before the table",
            ));
        }
        if slot as usize >= MESSAGE_WINDOWS {
            return Err(error(
                pc,
                0x2141,
                "message window slot outside 0..7 sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4004))?;
        if let Some(text) = self.windows[slot as usize].take() {
            self.texts[text] = false;
        }
        Ok(true)
    }
}

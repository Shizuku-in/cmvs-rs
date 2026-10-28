//! Engine +4320 history list: append 0x2136 and clear 0x212f.
//! Addresses are bound to the binary hash in research.md. No UI or serialization.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};
use std::collections::VecDeque;

/// 0x14005e7e0 constructs a list with capacity 128.
pub const HISTORY_CAPACITY: usize = 128;
/// Research allocation bound for the dynamically allocated third string, not a format limit.
pub const MAX_HISTORY_TEXT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// SP-4/-8/-12/-16. Strings 0,1,3 occupy 32-byte fields at +8/+40/+72;
    /// string 2 is separately allocated and referenced at +104. No encoding assumed.
    pub strings: [Vec<u8>; 4],
    /// SP-20..SP-40, copied to +112..+132 without conversion.
    pub words: [u32; 6],
}

#[derive(Debug, Default)]
pub struct HistoryHost {
    entries: VecDeque<HistoryEntry>,
}
impl HistoryHost {
    /// Newest first; original list order, also used by its snapshot writer.
    pub fn entries(&self) -> &VecDeque<HistoryEntry> {
        &self.entries
    }

    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        // 0x140083240 -> 0x14005e800 frees the nodes and their dynamic strings,
        // then clears the head pointer. The configured capacity is unchanged.
        if matches!(vm.pending_system(), Some((_, 0x12f))) {
            vm.complete_system(SystemResult(0x4000))?;
            self.entries.clear();
            return Ok(true);
        }
        let Some((pc, 0x136)) = vm.pending_system() else {
            return Ok(false);
        };
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x2136, e.0))
        };
        let mut strings: [Vec<u8>; 4] = Default::default();
        for (i, bytes) in strings.iter_mut().enumerate() {
            *bytes = vm.resolve_string(word(4 + 4 * i)?)?;
            if let Some(end) = bytes.iter().position(|b| *b == 0) {
                bytes.truncate(end);
            }
            if i != 2 && bytes.len() >= 32 {
                return Err(error(
                    pc,
                    0x2136,
                    "history string overflows its 32-byte field",
                ));
            }
            if i == 2 && bytes.len() > MAX_HISTORY_TEXT {
                return Err(error(
                    pc,
                    0x2136,
                    "history text exceeds research allocation limit",
                ));
            }
        }
        let mut words = [0; 6];
        for (i, value) in words.iter_mut().enumerate() {
            *value = word(20 + 4 * i)?;
        }
        vm.complete_system(SystemResult(0x4028))?;
        // 0x14005ee00 removes the tail before adding the new head.
        while self.entries.len() >= HISTORY_CAPACITY {
            self.entries.pop_back();
        }
        self.entries.push_front(HistoryEntry { strings, words });
        Ok(true)
    }
}

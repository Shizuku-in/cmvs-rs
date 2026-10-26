//! Engine +4736: a timed action sequence (0x614-byte buffer from 0x140014270: a five-word
//! header plus 16 entries of 96 bytes) that the frame update 0x140015900 runs through
//! 0x140014b10; addresses refer to the documented binary hash. Reset 0x2280 and entry
//! count query 0x2285 are modeled; adding or running entries stays unhandled.
use crate::{bytecode::DecodeError, machine::Machine, vm::SystemResult};

/// Header words as 0x140014b10 reads them; the constructor zeroes the whole buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequenceHeader {
    /// Word 0: the frame update returns at once while it is zero.
    pub active: u32,
    /// Word 1: entry count.
    pub count: u32,
    /// Word 2: next entry; the update stops once it reaches `count`.
    pub index: u32,
    /// Word 3: start time for wait entries.
    pub start: u32,
    /// Word 4: time already consumed by earlier wait entries.
    pub waited: u32,
}

#[derive(Debug, Default)]
pub struct SequenceHost {
    header: SequenceHeader,
}

impl SequenceHost {
    pub fn header(&self) -> SequenceHeader {
        self.header
    }
    /// 0x2280: the dispatcher calls 0x140014320 inline, zeroing the five header words, and
    /// returns 0x4000 (nothing popped). Entries keep their bytes but are no longer run.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((_, command @ (0x280 | 0x285))) = vm.pending_system() else {
            return Ok(false);
        };
        vm.complete_system(SystemResult(0x4000))?;
        if command == 0x280 {
            self.header = SequenceHeader::default();
        } else {
            // 0x14007f8e0 -> 0x1400143c0 reads header word 1, not active/index.
            vm.state_mut()
                .set_system_integer(0, self.header.count)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

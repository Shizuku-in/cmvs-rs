//! Engine +4736: a timed action sequence (0x614-byte buffer from 0x140014270: a five-word
//! header plus 16 entries of 96 bytes) that the frame update 0x140015900 runs through
//! 0x140014b10; addresses refer to the documented binary hash. Wait entries and their
//! start/stop/finish lifecycle are modeled; appending other action types stays unhandled.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const SEQUENCE_ENTRIES: usize = 16;

/// Semantics of the supported 96-byte record type. This is not a binary save image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceEntry {
    Wait { duration: u32 },
}

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

impl SequenceHeader {
    /// 0x1400143d0: any nonzero active word exposes the raw index, otherwise -1.
    /// Does not compare the index to count or modify the sequence.
    pub fn current_index(self) -> u32 {
        if self.active != 0 {
            self.index
        } else {
            u32::MAX
        }
    }
}

#[derive(Debug, Default)]
pub struct SequenceHost {
    header: SequenceHeader,
    entries: [Option<SequenceEntry>; SEQUENCE_ENTRIES],
    /// Explicit wrapping millisecond clock, independent of callback registration ticks.
    clock: u32,
}

impl SequenceHost {
    pub fn with_tick(clock: u32) -> Self {
        Self {
            clock,
            ..Self::default()
        }
    }
    pub fn header(&self) -> SequenceHeader {
        self.header
    }
    /// Includes inactive records retained by 0x2280/0x2284, unlike the active count.
    pub fn entries(&self) -> &[Option<SequenceEntry>; SEQUENCE_ENTRIES] {
        &self.entries
    }
    pub fn clock(&self) -> u32 {
        self.clock
    }

    /// 0x140015900 calls 0x140014b10 before updating scene objects. Waits use absolute
    /// clock - start - already-waited (u32 wrapping), allowing consecutive waits to
    /// consume leftover time within the same update. No 200 ms frame clamp.
    pub fn advance_frame(&mut self, delta: u32) {
        self.clock = self.clock.wrapping_add(delta);
        self.run_waits(false);
    }

    fn run_waits(&mut self, finish: bool) {
        if self.header.active == 0 || self.header.index >= self.header.count {
            return;
        }
        while self.header.index < self.header.count {
            let Some(SequenceEntry::Wait { duration }) = self.entries[self.header.index as usize]
            else {
                unreachable!("only validated wait entries can be appended");
            };
            if !finish {
                let elapsed = self
                    .clock
                    .wrapping_sub(self.header.start)
                    .wrapping_sub(self.header.waited);
                if elapsed < duration {
                    return;
                }
                self.header.waited = self.header.waited.wrapping_add(duration);
            }
            self.header.index += 1;
        }
        self.header.active = 0;
    }
    /// 0x2280: the dispatcher calls 0x140014320 inline, zeroing the five header words, and
    /// returns 0x4000 (nothing popped). Entries keep their bytes but are no longer run.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ 0x280..=0x286)) = vm.pending_system() else {
            return Ok(false);
        };
        if command == 0x286 {
            let duration = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2286, e.0))?;
            vm.complete_system(SystemResult(0x4004))?;
            // 0x1400143e0: ignore an append at capacity but still complete the command.
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::Wait { duration });
                self.header.count += 1;
            }
            return Ok(true);
        }
        vm.complete_system(SystemResult(0x4000))?;
        if command == 0x280 {
            self.header = SequenceHeader::default();
        } else if command == 0x282 {
            // 0x140014350: an active or empty sequence is unchanged; otherwise restart.
            if self.header.active == 0 && self.header.count != 0 {
                self.header.start = self.clock;
                self.header.waited = 0;
                self.header.index = 0;
                self.header.active = 1;
            }
        } else if command == 0x283 {
            // 0x1400143a0 leaves count, index, timing and entries untouched.
            if self.header.active != 0 && self.header.count != 0 {
                self.header.active = 0;
            }
        } else if command == 0x284 {
            // 0x14007f860 executes the remaining active entries in fast mode, then resets.
            self.run_waits(true);
            self.header = SequenceHeader::default();
        } else {
            // 0x14007f8e0 -> 0x1400143c0 reads header word 1, not active/index.
            // 0x14007f830 -> 0x1400143d0 reads word 2 only when active is nonzero.
            let value = if command == 0x281 {
                self.header.current_index()
            } else {
                self.header.count
            };
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

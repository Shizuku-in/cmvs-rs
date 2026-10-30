//! Engine +4320 history list: append 0x2136, clear 0x212f and the queries 0x2130..0x2135.
//! Addresses are bound to the binary hash in research.md.
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

#[derive(Debug, Clone, Default)]
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
        if let Some((pc, command @ 0x130..=0x135)) = vm.pending_system() {
            return self.query(vm, pc, command).map(|()| true);
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

    /// 0x2130 / 0x140075730 -> 0x14005e9c0: system result 0 = entry count; 0x4000. The
    /// others take SP-4 entry (newest = 0; the walk stops at once for negative values):
    /// 0x2131 / 0x2132 / 0x2133 / 0x2134 (0x14005ea70 / 0x14005eac0 / 0x14005ea20 /
    /// 0x14005eb00) copy string +40 / *+104 / +8 / +72 to the SP-8 string destination, empty
    /// for a missing entry; 0x2135 (0x14005e9e0) sets system result 0 to word SP-8 % 6
    /// (unsigned), or -1 for a missing entry. All return 0x4008. Destinations other than
    /// the string bank are not modeled.
    fn query(&mut self, vm: &mut Machine<'_>, pc: usize, command: u16) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        if command == 0x130 {
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, self.entries.len() as u32)
                .expect("slot 0 exists");
            return Ok(());
        }
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let entry = self.entries.get((word(4)? as i32).max(0) as usize);
        let argument = word(8)?;
        if command == 0x135 {
            let value = entry.map_or(u32::MAX, |e| e.words[(argument % 6) as usize]);
            vm.complete_system(SystemResult(0x4008))?;
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(());
        }
        let slot = crate::text::bank_slot(argument)
            .ok_or_else(|| error(pc, op, "history string destination outside the string bank"))?;
        let field = match command {
            0x131 => 1,
            0x132 => 2,
            0x133 => 0,
            _ => 3,
        };
        let text = entry.map_or(&[][..], |e| e.strings[field].as_slice());
        if text.len() >= 1024 {
            return Err(error(pc, op, "history string overflows its destination"));
        }
        vm.complete_system(SystemResult(0x4008))?;
        vm.state_mut()
            .set_string(slot, text)
            .map_err(|why| error(pc, op, why))?;
        Ok(())
    }
}

/// Constructor 0x140016e20: +11820, the number of newest entries the snapshot keeps.
/// Only 0x22b1 changes it, and no Amatsutsumi script uses that command.
pub const SAVED_ENTRIES: usize = 32;

impl HistoryHost {
    /// Record 0x540 (0x14005eb60): marker 0xffff0000, capacity, count, then the newest
    /// `SAVED_ENTRIES` entries newest first as strings +8, +40, *+104, +72 and six words.
    pub(crate) fn saved(&self) -> Vec<u8> {
        let count = self.entries.len().min(SAVED_ENTRIES);
        let mut out = Vec::new();
        for word in [0xffff_0000, HISTORY_CAPACITY as u32, count as u32] {
            out.extend(u32::to_le_bytes(word));
        }
        for entry in self.entries.iter().take(count) {
            for text in &entry.strings {
                out.extend(text);
                out.push(0);
            }
            for word in entry.words {
                out.extend(word.to_le_bytes());
            }
        }
        out
    }

    /// Loader: 0x14005e800 clears the list, then 0x14005ecb0 appends each saved entry at
    /// the tail (0x14005e890), keeping the saved newest-first order.
    pub(crate) fn restore(&mut self, record: &[u8]) -> Result<(), &'static str> {
        let mut at = 0;
        let mut word = || -> Result<u32, &'static str> {
            let bytes = record.get(at..at + 4).ok_or("truncated history record")?;
            at += 4;
            Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
        };
        if word()? != 0xffff_0000 || word()? != HISTORY_CAPACITY as u32 {
            return Err("unsupported history record version or capacity");
        }
        let count = word()? as usize;
        if count > HISTORY_CAPACITY {
            return Err("history record exceeds the list capacity");
        }
        let mut entries = VecDeque::with_capacity(count);
        for _ in 0..count {
            let mut strings: [Vec<u8>; 4] = Default::default();
            for (i, text) in strings.iter_mut().enumerate() {
                let tail = record.get(at..).ok_or("truncated history record")?;
                let end = tail
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or("history string lacks its terminator")?;
                if (i != 2 && end >= 32) || end > MAX_HISTORY_TEXT {
                    return Err("history string exceeds its field");
                }
                *text = tail[..end].to_vec();
                at += end + 1;
            }
            let mut words = [0; 6];
            for value in &mut words {
                let bytes = record.get(at..at + 4).ok_or("truncated history record")?;
                *value = u32::from_le_bytes(bytes.try_into().unwrap());
                at += 4;
            }
            entries.push_back(HistoryEntry { strings, words });
        }
        if at != record.len() {
            return Err("trailing bytes after history entries");
        }
        self.entries = entries;
        Ok(())
    }
}

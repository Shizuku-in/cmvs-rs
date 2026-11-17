//! Message history entries and script-visible history queries.
//!
//! [`HistoryHost`] stores recorded text and supports append, clear, and lookup
//! operations. It does not display a history window or rasterize text.

// Engine +4320: append 0x2136, clear 0x212f, and queries 0x2130..0x2135.
use crate::{bytecode::DecodeError, dispatch::Call, evaluate::error};
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

    pub(crate) fn handle(&mut self, call: &mut Call<'_>) -> Result<(), DecodeError> {
        use crate::cvns::cmd;
        let pc = call.pc;
        let command = call.command;
        // 0x140083240 -> 0x14005e800 frees the nodes and their dynamic strings,
        // then clears the head pointer. The configured capacity is unchanged.
        if command == 0x12f {
            call.complete(0x4000)?;
            self.entries.clear();
            return Ok(());
        }
        if matches!(
            command,
            0x130..=0x135 | cmd::HISTORY_QUERY_2 | cmd::HISTORY_QUERY
        ) {
            return self.query(call);
        }
        let is_cvns = matches!(command, cmd::HISTORY_APPEND | cmd::HISTORY_APPEND_2);
        let op = command + 0x2000;
        let str_count = if is_cvns { 3 } else { 4 };
        let word_count = match command {
            cmd::HISTORY_APPEND_2 => 2,
            cmd::HISTORY_APPEND => 4,
            _ => 6,
        };
        let mut strings: [Vec<u8>; 4] = Default::default();
        // `sub_465d40 -> sub_442120` (and cvns-20050216 `sub_4320A0 -> sub_4239F0`): CVNS
        // arg1 (`SP-8`) is the dynamic dialogue text (`strings[2]`), while arg2 (`SP-12`)
        // goes into the 32-byte field at +36 (`strings[1]`).
        let dest_for = |i: usize| if is_cvns { [0, 2, 1][i] } else { i };
        for i in 0..str_count {
            let dest = dest_for(i);
            let name = ["string 0", "string 1", "string 2", "string 3"][dest];
            let reference = call.argument(4 + 4 * i, name)?;
            let mut value = call.vm.resolve_string(reference)?;
            if let Some(end) = value.iter().position(|b| *b == 0) {
                value.truncate(end);
            }
            if dest != 2 && value.len() >= 32 {
                return Err(error(pc, op, "history string overflows its 32-byte field"));
            }
            if dest == 2 && value.len() > MAX_HISTORY_TEXT {
                return Err(error(
                    pc,
                    op,
                    "history text exceeds research allocation limit",
                ));
            }
            strings[dest] = value;
        }
        let mut words = [0; 6];
        let words_base = 4 + 4 * str_count;
        for (i, value) in words.iter_mut().take(word_count).enumerate() {
            *value = call.argument(
                words_base + 4 * i,
                ["word 0", "word 1", "word 2", "word 3", "word 4", "word 5"][i],
            )?;
        }
        call.complete(match command {
            cmd::HISTORY_APPEND_2 => 0x4014,
            cmd::HISTORY_APPEND => 0x401c,
            _ => 0x4028,
        })?;
        // 0x14005ee00 removes the tail before adding the new head.
        while self.entries.len() >= HISTORY_CAPACITY {
            self.entries.pop_back();
        }
        self.entries.push_front(HistoryEntry { strings, words });
        Ok(())
    }

    /// 0x2130 / 0x140075730 -> 0x14005e9c0: system result 0 = entry count; 0x4000. The
    /// others take SP-4 entry (newest = 0; the walk stops at once for negative values):
    /// 0x2131 / 0x2132 / 0x2133 / 0x2134 (0x14005ea70 / 0x14005eac0 / 0x14005ea20 /
    /// 0x14005eb00) copy string +40 / *+104 / +8 / +72 to the SP-8 string destination, empty
    /// for a missing entry; 0x2135 (0x14005e9e0) sets system result 0 to word SP-8 % 6
    /// (unsigned), or -1 for a missing entry. All return 0x4008. Destinations other than
    /// the string bank are not modeled.
    fn query(&mut self, call: &mut Call<'_>) -> Result<(), DecodeError> {
        use crate::cvns::cmd;
        let pc = call.pc;
        let command = call.command;
        let op = command + 0x2000;
        if command == 0x130 {
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, self.entries.len() as u32)
                .expect("slot 0 exists");
            return Ok(());
        }
        let index = call.argument(4, "entry index")? as i32;
        let entry = self.entries.get(index.max(0) as usize);
        let is_word = matches!(command, 0x135 | cmd::HISTORY_QUERY_2 | cmd::HISTORY_QUERY);
        let argument = call.argument(4 + 4, if is_word { "word index" } else { "destination" })?;
        if is_word {
            let modulus = match command {
                cmd::HISTORY_QUERY_2 => 2,
                cmd::HISTORY_QUERY => 4,
                _ => 6,
            };
            let value = entry.map_or(u32::MAX, |e| e.words[(argument % modulus) as usize]);
            call.complete(0x4008)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(());
        }
        let slot = crate::text::bank_slot_for(call.vm.state(), argument)
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
        call.complete(0x4008)?;
        call.vm
            .state_mut()
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

#[cfg(test)]
mod tests {
    use super::{Call, HistoryHost};
    use crate::{
        machine::{Machine, Step},
        ps3::Script,
    };

    enum Arg<'a> {
        Int(u32),
        Str(&'a [u8]),
    }

    fn build_script(calls: &[(u16, &[Arg<'_>])]) -> Script {
        let mut code = Vec::new();
        let mut text = Vec::new();
        for &(op, args_sp_first) in calls {
            for arg in args_sp_first.iter().rev() {
                let val = match *arg {
                    Arg::Int(v) => v,
                    Arg::Str(bytes) => {
                        let off = text.len() as u32;
                        text.extend_from_slice(bytes);
                        text.push(0);
                        off
                    }
                };
                code.extend(0x440u16.to_le_bytes());
                code.extend(4u16.to_le_bytes());
                code.extend(val.to_le_bytes());
            }
            code.extend(op.to_le_bytes());
        }
        code.extend(0x2000u16.to_le_bytes());
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(&code);
        payload.extend(&text);
        let mut raw = b"PSS0".to_vec();
        for n in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(&payload);
        Script::decode(&raw).unwrap()
    }

    #[test]
    fn cvns_append_maps_arg1_to_dynamic_text_and_arg2_to_speaker() {
        let long_dialogue = b"Dialogue text well over 32 bytes for CVNS dynamic slot 2.";
        assert!(long_dialogue.len() > 32);
        let script = build_script(&[
            (
                0x2537,
                &[
                    Arg::Str(b"voice_001"),
                    Arg::Str(long_dialogue),
                    Arg::Str(b"Miharu"),
                    Arg::Int(11),
                    Arg::Int(22),
                    Arg::Int(33),
                    Arg::Int(44),
                ],
            ),
            (0x2133, &[Arg::Int(0), Arg::Int(0x8000_0000)]),
            (0x2132, &[Arg::Int(0), Arg::Int(0x8000_0001)]),
            (0x2131, &[Arg::Int(0), Arg::Int(0x8000_0002)]),
            (0x2536, &[Arg::Int(0), Arg::Int(5)]),
        ]);
        let mut vm = Machine::new(&script).unwrap();
        let mut history = HistoryHost::default();
        loop {
            match vm.step().unwrap() {
                Step::Continue => {}
                Step::System { pc, command } => {
                    history
                        .handle(&mut Call::new(&mut vm, pc, command))
                        .unwrap();
                }
                Step::Yield => break,
            }
        }

        assert_eq!(history.entries().len(), 1);
        let entry = &history.entries()[0];
        assert_eq!(entry.strings[0], b"voice_001");
        assert_eq!(entry.strings[1], b"Miharu");
        assert_eq!(entry.strings[2], long_dialogue);
        assert_eq!(entry.words[..4], [11, 22, 33, 44]);

        assert_eq!(vm.state().string(0).unwrap(), b"voice_001");
        assert_eq!(vm.state().string(1).unwrap(), long_dialogue);
        assert_eq!(vm.state().string(2).unwrap(), b"Miharu");
        assert_eq!(vm.state().system_integer(0), Some(22));

        // arg2 (`strings[1]`) must still enforce the 32-byte fixed field bound.
        let overflow_speaker = build_script(&[(
            0x2537,
            &[
                Arg::Str(b"v"),
                Arg::Str(b"dialogue"),
                Arg::Str(&[b'S'; 32]),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
            ],
        )]);
        let mut vm_err = Machine::new(&overflow_speaker).unwrap();
        while !matches!(vm_err.step().unwrap(), Step::System { .. }) {}
        let (pc, command) = vm_err.pending_system().unwrap();
        assert_eq!(
            history
                .handle(&mut Call::new(&mut vm_err, pc, command))
                .unwrap_err()
                .reason,
            "history string overflows its 32-byte field"
        );
    }

    #[test]
    fn cvns_profiles_query_the_matching_history_word_count() {
        use crate::{
            dispatch::{self, Flow, Hosts},
            profile::Registry,
        };

        for (id, word_count, append) in [
            ("cvns-20040515", 2, 0x212c),
            ("cvns-20040730", 2, 0x212c),
            ("cvns-20050216", 2, 0x212c),
            ("cvns-20050727", 4, 0x2137),
            ("cvns-20090203", 4, 0x2137),
        ] {
            let profile = Registry::builtin().profile(id).unwrap();
            let words = [77, 88, 99, 111];
            let mut args = vec![
                Arg::Str(b"voice"),
                Arg::Str(b"Short line"),
                Arg::Str(b"Speaker"),
            ];
            args.extend(words[..word_count].iter().copied().map(Arg::Int));
            let script = build_script(&[
                (append, &args),
                (0x2136, &[Arg::Int(0), Arg::Int(0)]),
                (0x2136, &[Arg::Int(0), Arg::Int(1)]),
                (0x2136, &[Arg::Int(0), Arg::Int(2)]),
                (0x2136, &[Arg::Int(0), Arg::Int(3)]),
                (0x2136, &[Arg::Int(0), Arg::Int(u32::MAX)]),
                (0x2136, &[Arg::Int(1), Arg::Int(0)]),
            ]);
            let mut vm = Machine::new(&script).unwrap();
            let query_command = profile.command_map().unwrap().runtime(0x136);
            vm.set_command_map(profile.command_map());
            let mut history = HistoryHost::default();
            let mut results = Vec::new();
            loop {
                match vm.step().unwrap() {
                    Step::Continue => {}
                    Step::System { command, .. } => {
                        assert_eq!(
                            dispatch::run(
                                profile.semantics(command),
                                &mut vm,
                                &mut Hosts {
                                    history: Some(&mut history),
                                    ..Default::default()
                                },
                            )
                            .unwrap(),
                            Some(Flow::Done),
                        );
                        assert_eq!(vm.stack().len_bytes(), 0);
                        if command == query_command {
                            results.push(vm.state().system_integer(0).unwrap());
                        }
                    }
                    Step::Yield => break,
                }
            }
            let expected = if word_count == 2 {
                [77, 88, 77, 88, 88, u32::MAX]
            } else {
                [77, 88, 99, 111, 111, u32::MAX]
            };
            assert_eq!(results, expected, "{id}");
            assert_eq!(history.entries().len(), 1);
            let entry = &history.entries()[0];
            assert_eq!(entry.strings[0], b"voice");
            assert_eq!(entry.strings[1], b"Speaker");
            assert_eq!(entry.strings[2], b"Short line");
            assert_eq!(entry.words[..word_count], words[..word_count]);
            assert!(entry.words[word_count..].iter().all(|&w| w == 0));
        }
    }
}

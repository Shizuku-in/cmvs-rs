//! Public script initialization and explicit cross-script control.
//! No save snapshot, callback scheduling, rendering, or filesystem search policy.
use crate::{
    cpz::{ArchiveIndex, Cpz7Parameters},
    evaluate::error,
    machine::Machine,
    ps3::Script,
    vm::SystemResult,
};
use std::{
    collections::BTreeMap,
    error::Error,
    io::{Read, Seek},
};

pub type LoadError = Box<dyn Error>;

/// Inspect literal archive registrations in the straight-line startup prefix.
/// REALIVE's CPZ7 parameters are per archive, not a game-wide MD5 replacement.
/// Dynamic expressions and control flow are not evaluated by this inspection helper.
pub fn cpz7_parameters_from_start(
    script: &Script,
    basename: &[u8],
) -> Result<Option<Cpz7Parameters>, crate::FormatError> {
    use crate::{
        FormatError,
        bytecode::{self, InstructionKind, Term},
    };
    let instructions =
        bytecode::decode_layout(script.code(), script.header.entry_pc, &script.labels())
            .map_err(|_| FormatError("invalid startup script layout"))?;
    let strings = script.strings()?;
    let literal = |at: usize, kind: u16, tag: u16| -> Option<u32> {
        let expr = instructions.get(at)?;
        if expr.pc < script.header.entry_pc as usize {
            return None;
        }
        if instructions.get(at + 1)?.opcode != 0x430 {
            return None;
        }
        let InstructionKind::Expression(expression) = &expr.kind else {
            return None;
        };
        if expression.kind != kind {
            return None;
        }
        match expression.terms.as_slice() {
            [
                Term::Scalar {
                    tag: actual, raw, ..
                },
            ] if *actual == tag => Some(*raw),
            _ => None,
        }
    };
    let mut pack_mode = None;
    let mut found = None;
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.pc < script.header.entry_pc as usize {
            continue;
        }
        match &instruction.kind {
            InstructionKind::Jump { .. }
            | InstructionKind::ConditionalJump { .. }
            | InstructionKind::CompareJump { .. }
            | InstructionKind::LongJump { .. }
            | InstructionKind::ScriptReturn { .. } => break,
            InstructionKind::System { command: 0x1f } => {
                pack_mode = Some(
                    i.checked_sub(2)
                        .and_then(|at| literal(at, 0x200, 0x100))
                        .ok_or(FormatError("dynamic startup archive mode"))?,
                );
            }
            InstructionKind::System {
                command: 0x268..=0x26e,
            } if pack_mode == Some(13) => {
                let Some(at) = i.checked_sub(12) else {
                    return Err(FormatError("incomplete CPZ7 registration"));
                };
                let path_offset = literal(at + 8, 0x201, 0x120)
                    .ok_or(FormatError("dynamic CPZ7 archive path"))?;
                let path = strings
                    .iter()
                    .find(|entry| entry.offset == path_offset as usize)
                    .ok_or(FormatError("invalid CPZ7 archive path reference"))?
                    .bytes;
                let name = path
                    .rsplit(|b| matches!(b, b'/' | b'\\'))
                    .next()
                    .unwrap_or(path);
                if !name.eq_ignore_ascii_case(basename) {
                    continue;
                }
                if literal(at + 10, 0x200, 0x100).is_none() {
                    return Err(FormatError("dynamic CPZ7 archive slot"));
                }
                let mut keys = [0; 4];
                for (n, key) in keys.iter_mut().enumerate() {
                    *key = literal(at + 6 - n * 2, 0x200, 0x100)
                        .ok_or(FormatError("dynamic CPZ7 archive key"))?;
                }
                let parameters = Cpz7Parameters::from(keys);
                if found.is_some_and(|previous| previous != parameters) {
                    return Err(FormatError("conflicting CPZ7 archive registrations"));
                }
                found = Some(parameters);
            }
            _ => {}
        }
    }
    match pack_mode {
        Some(12) => Ok(Some(Cpz7Parameters::default())),
        Some(13) => Ok(found),
        _ => Err(FormatError("startup does not select a supported CPZ7 mode")),
    }
}

pub trait ScriptSource {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError>;
}

pub struct ArchiveScripts<R> {
    reader: R,
    index: ArchiveIndex,
}
impl<R: Read + Seek> ArchiveScripts<R> {
    pub fn new(mut reader: R) -> Result<Self, LoadError> {
        let index = ArchiveIndex::read(&mut reader)?;
        Ok(Self { reader, index })
    }

    pub fn with_cpz7_parameters(
        mut reader: R,
        parameters: Cpz7Parameters,
    ) -> Result<Self, LoadError> {
        let index = ArchiveIndex::read_with_cpz7_parameters(&mut reader, parameters)?;
        Ok(Self { reader, index })
    }
}
impl<R: Read + Seek> ScriptSource for ArchiveScripts<R> {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError> {
        let entry = self.index.find(name)?;
        Ok(Script::decode(
            &self.index.read_entry(&mut self.reader, entry)?,
        )?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub script_slot: u8,
    pub target: u32,
    /// Fields +14812, +14808, +14804 in original argument-push order.
    pub parameters: [u32; 3],
    pub registered_tick: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlEvent {
    Applied,
    CommonEntered {
        slot: u8,
    },
    /// Original 0x2080 also captures an engine-wide save snapshot.
    /// The core transition is ready, but callers must stop until they implement it.
    MainPreparedNeedsSnapshot,
}

#[derive(Clone)]
pub struct ScriptControl {
    callbacks: BTreeMap<u8, Callback>,
    settings: BTreeMap<u16, u32>,
    virtual_tick: u32,
}

impl ScriptControl {
    /// An explicit virtual time domain for deterministic research, not wall-clock emulation.
    pub fn new(virtual_tick: u32) -> Self {
        Self {
            callbacks: BTreeMap::new(),
            settings: BTreeMap::new(),
            virtual_tick,
        }
    }
    pub fn callbacks(&self) -> &BTreeMap<u8, Callback> {
        &self.callbacks
    }
    /// The automatic dispatch 0x140064f50 runs at every VM run entry: nothing while the
    /// 0x208c flag (+12240) is set. With the 0x2089 setting (+14788) the registered entries
    /// 3..6 and 28 are due every time, 10..17 while message window 0..7 is shown (+4) and
    /// 26 when its dispatch word is set; without it only 26 (word set) or else 28. Entry 26's
    /// word is set only by the window-close paths (0x140009cd0, menu 32756), which Rust
    /// does not model, so it is never due here. Returned in push order: the last one runs
    /// first.
    pub fn due_callbacks(&self, windows_shown: [bool; 8]) -> Vec<(u8, usize)> {
        if self.settings.get(&0x208c).copied().unwrap_or(0) != 0 {
            return Vec::new();
        }
        let registered = |index: u8| {
            self.callbacks
                .get(&index)
                .filter(|c| c.enabled)
                .map(|c| (c.script_slot, c.target as usize))
        };
        if self.settings.get(&0x2089).copied().unwrap_or(0) == 0 {
            return registered(28).into_iter().collect();
        }
        (0..32u8)
            .filter(|&index| match index {
                3..=6 | 28 => true,
                10..=17 => windows_shown[usize::from(index - 10)],
                _ => false,
            })
            .filter_map(registered)
            .collect()
    }
    pub fn settings(&self) -> &BTreeMap<u16, u32> {
        &self.settings
    }
    pub fn set_virtual_tick(&mut self, tick: u32) {
        self.virtual_tick = tick;
    }

    /// 0x140080530: a main replacement disables only callbacks owned by slot 0.
    pub(crate) fn main_replaced(&mut self) {
        for callback in self.callbacks.values_mut() {
            if callback.script_slot == 0 {
                callback.enabled = false;
                callback.target = u32::MAX;
            }
        }
    }

    pub fn try_handle<S: ScriptSource>(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut S,
    ) -> Result<Option<ControlEvent>, LoadError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(None);
        };
        let op = command + 0x2000;
        let argument = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let flags = match command {
            0x81 => {
                let slot = argument(4)?;
                if slot >= 3 {
                    return Err(error(pc, op, "common slot must be 0..2").into());
                }
                let name = vm.resolve_string(argument(8)?)?;
                let script = source.load(&name)?;
                vm.enter_common(slot, name, script)?;
                return Ok(Some(ControlEvent::CommonEntered {
                    slot: vm.current_slot(),
                }));
            }
            0x80 => {
                let name = vm.resolve_string(argument(4)?)?;
                let script = source.load(&name)?;
                vm.switch_main_core(name, script)?;
                self.main_replaced();
                return Ok(Some(ControlEvent::MainPreparedNeedsSnapshot));
            }
            0x89 | 0x8c => {
                self.settings.insert(op, u32::from(argument(4)? != 0));
                0x4004
            }
            0x83 => {
                let high = argument(4)?;
                let low = argument(8)?;
                self.settings.insert(op, low | high.wrapping_shl(16));
                0x4008
            }
            0x88 => {
                let slot = argument(4)?;
                let target = argument(8)?;
                let parameters = [argument(20)?, argument(16)?, argument(12)?];
                if slot > 63 {
                    return Err(error(pc, op, "callback slot outside 0..63").into());
                }
                if target != u32::MAX && !vm.is_boundary(vm.current_slot(), target as usize) {
                    return Err(
                        error(pc, op, "callback target is not an instruction boundary").into(),
                    );
                }
                self.callbacks.insert(
                    slot as u8,
                    Callback {
                        script_slot: vm.current_slot(),
                        target,
                        parameters,
                        registered_tick: self.virtual_tick,
                        enabled: target != u32::MAX,
                    },
                );
                0x4014
            }
            0x8a => {
                let slot = (argument(4)? & 63) as u8;
                let callback = self
                    .callbacks
                    .get(&slot)
                    .filter(|c| c.enabled)
                    .ok_or_else(|| error(pc, op, "callback is absent or disabled"))?;
                vm.call_script(callback.script_slot, callback.target as usize)?;
                return Ok(Some(ControlEvent::Applied));
            }
            0x8b => {
                let slot = argument(4)?;
                if slot > 63 {
                    return Err(error(pc, op, "callback slot outside 0..63").into());
                }
                if let Some(callback) = self.callbacks.get_mut(&(slot as u8)) {
                    callback.enabled = false;
                    callback.target = u32::MAX;
                }
                0x4004
            }
            _ => return Ok(None),
        };
        vm.complete_system(SystemResult(flags))?;
        Ok(Some(ControlEvent::Applied))
    }
}

/// Record 0x10b: +14788 (the 0x2089 setting) and 64 callback entries of 28 bytes:
/// +0 dispatch word (zeroed on registration), +4 script slot, +8 target (-1 = none),
/// +12/+16/+20 the SP-12/SP-16/SP-20 parameters, +24 registration clock (0x140071ed0).
pub const CALLBACK_RECORD: usize = 4 + 64 * 28;

impl ScriptControl {
    /// Writer 0x140069270 copies +14788..+16584 verbatim. Rust never runs the automatic
    /// dispatcher, so every dispatch word is the zero that registration leaves.
    pub fn saved_callbacks(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CALLBACK_RECORD);
        let setting = self.settings.get(&0x2089).copied().unwrap_or(0);
        out.extend(setting.to_le_bytes());
        for slot in 0..64u8 {
            let entry = match self.callbacks.get(&slot) {
                Some(c) => [
                    0,
                    u32::from(c.script_slot),
                    c.target,
                    c.parameters[2],
                    c.parameters[1],
                    c.parameters[0],
                    c.registered_tick,
                ],
                // Constructor 0x140068580: slot 0, target -1, other words zero.
                None => [0, 0, u32::MAX, 0, 0, 0, 0],
            };
            for word in entry {
                out.extend(word.to_le_bytes());
            }
        }
        out
    }

    /// Loader 0x140081960 after 0x140069fd0: callbacks registered by common scripts survive
    /// the load, saved ones of common scripts are disabled, and main-script callbacks come
    /// from the save. The 0x2089 setting is restored from +14788.
    pub fn restore_callbacks(&mut self, record: &[u8]) -> Result<(), &'static str> {
        if record.len() != CALLBACK_RECORD {
            return Err("callback record has unexpected length");
        }
        let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let mut callbacks = BTreeMap::new();
        for slot in 0..64u8 {
            let at = 4 + 28 * usize::from(slot);
            let [dispatch, script, target, p12, p16, p20, tick] =
                std::array::from_fn(|i| word(at + 4 * i));
            if dispatch != 0 {
                return Err("saved callback is mid-dispatch, which is not modeled");
            }
            if script > 3 {
                return Err("saved callback script slot is out of range");
            }
            let mut saved = Callback {
                script_slot: script as u8,
                target,
                parameters: [p20, p16, p12],
                registered_tick: tick,
                enabled: target != u32::MAX,
            };
            if script != 0 {
                saved.target = u32::MAX;
                saved.enabled = false;
            }
            let current = self.callbacks.get(&slot).filter(|c| c.script_slot != 0);
            let entry = match current {
                Some(current) => Some(current.clone()),
                None if saved == Callback::unused() => None,
                None => Some(saved),
            };
            if let Some(entry) = entry {
                callbacks.insert(slot, entry);
            }
        }
        self.callbacks = callbacks;
        self.settings.insert(0x2089, u32::from(word(0) != 0));
        Ok(())
    }
}

impl Callback {
    /// The constructor's entry, indistinguishable from never registered.
    fn unused() -> Self {
        Self {
            script_slot: 0,
            target: u32::MAX,
            parameters: [0; 3],
            registered_tick: 0,
            enabled: false,
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn main_replacement_disables_only_main_callbacks_and_preserves_control_settings() {
        let mut host = ScriptControl::new(123);
        host.settings.insert(0x2089, 1);
        for slot in 0..4 {
            host.callbacks.insert(
                slot,
                Callback {
                    script_slot: slot,
                    target: 42,
                    parameters: [1, 2, 3],
                    registered_tick: 123,
                    enabled: true,
                },
            );
        }
        let before = host.callbacks.clone();
        host.main_replaced();
        assert!(!host.callbacks[&0].enabled);
        assert_eq!(host.callbacks[&0].target, u32::MAX);
        assert_eq!(host.callbacks[&0].parameters, [1, 2, 3]);
        for slot in 1..4 {
            assert_eq!(host.callbacks[&slot], before[&slot]);
        }
        assert_eq!(host.settings[&0x2089], 1);
    }
}

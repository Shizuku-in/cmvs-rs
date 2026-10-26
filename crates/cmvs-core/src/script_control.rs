//! Public script initialization and explicit cross-script control.
//! No save snapshot, callback scheduling, rendering, or filesystem search policy.
use crate::{cpz::Cpz6Index, evaluate::error, machine::Machine, ps3::Script, vm::SystemResult};
use std::{
    collections::BTreeMap,
    error::Error,
    io::{Read, Seek},
};

pub type LoadError = Box<dyn Error>;

pub trait ScriptSource {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError>;
}

pub struct ArchiveScripts<R> {
    reader: R,
    index: Cpz6Index,
}
impl<R: Read + Seek> ArchiveScripts<R> {
    pub fn new(mut reader: R) -> Result<Self, LoadError> {
        let index = Cpz6Index::read(&mut reader)?;
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
    pub fn settings(&self) -> &BTreeMap<u16, u32> {
        &self.settings
    }
    pub fn set_virtual_tick(&mut self, tick: u32) {
        self.virtual_tick = tick;
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
                for callback in self.callbacks.values_mut() {
                    if callback.script_slot == 0 {
                        callback.enabled = false;
                        callback.target = u32::MAX;
                    }
                }
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

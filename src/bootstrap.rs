//! Deterministic startup effects. No files are opened, saved or rendered here.
//! Unhandled environment-dependent commands remain pending in the VM.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    machine::{Machine, Step},
    vm::SystemResult,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Bootstrap {
    settings: BTreeMap<u16, u32>,
    paths: BTreeMap<(u16, u32), Vec<u8>>,
    handled: usize,
}

impl Bootstrap {
    /// Keys are raw command IDs, avoiding unsupported guesses about field names.
    pub fn settings(&self) -> &BTreeMap<u16, u32> {
        &self.settings
    }
    /// Raw game-relative path bytes. (Command ID, slot) identifies each registration.
    /// Absolute save roots, archive loading and file lookup precedence remain separate.
    pub fn paths(&self) -> &BTreeMap<(u16, u32), Vec<u8>> {
        &self.paths
    }
    pub fn handled_count(&self) -> usize {
        self.handled
    }

    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<Option<Step>, DecodeError> {
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
            0x1d..=0x1f => {
                let raw = argument(4)?;
                self.settings.insert(op, raw);
                0x4004
            }
            0x90 | 0x34d => {
                self.settings.insert(op, 1);
                0x4000
            }
            0x91..=0x94 => {
                let start = argument(4)?;
                let count = argument(8)?;
                let raw = argument(12)?;
                if (start as i32) < 0 || (count as i32) < 0 {
                    return Err(error(pc, op, "negative initialization range"));
                }
                let result = match command {
                    0x91 => vm.state_mut().fill_bits(start, count, raw != 0),
                    0x92 => vm.state_mut().fill_integers(start, count, raw as i32),
                    0x93 => vm.state_mut().fill_floats(start, count, raw),
                    0x94 => {
                        let bytes = vm.resolve_string(raw)?;
                        vm.state_mut().fill_strings(start, count, &bytes)
                    }
                    _ => unreachable!(),
                };
                result.map_err(|why| error(pc, op, why))?;
                0x400c
            }
            0x10..=0x14 | 0x19 | 0x1a => {
                let slot = argument(4)?;
                let raw = argument(8)?;
                let limit = if command == 0x11 { 10 } else { 5 };
                if slot >= limit {
                    return Err(error(pc, op, "resource path slot outside supported range"));
                }
                let path = vm.resolve_string(raw)?;
                if path.len() >= 2048 {
                    return Err(error(pc, op, "resource path too long"));
                }
                self.paths.insert((op, slot), path);
                0x4008
            }
            0x15 | 0x16 => {
                let path = vm.resolve_string(argument(4)?)?;
                if path.len() >= 2048 {
                    return Err(error(pc, op, "resource path too long"));
                }
                self.paths.insert((op, 0), path);
                0x4004
            }
            _ => return Ok(None),
        };
        let event = vm.complete_system(SystemResult(flags))?;
        self.handled += 1;
        Ok(Some(event))
    }
}

//! Deterministic startup effects. No files are opened, saved or rendered here.
//! Unhandled environment-dependent commands remain pending in the VM.
use crate::{
    bytecode::DecodeError,
    dispatch::Call,
    evaluate::error,
    machine::{Machine, Step},
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Bootstrap {
    settings: BTreeMap<u16, u32>,
    paths: BTreeMap<(u16, u32), Vec<u8>>,
    handled: usize,
    cursors: Vec<Vec<u8>>,
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
    /// Registration order maps to original one-based IDs; duplicates are retained.
    pub fn cursor_names(&self) -> &[Vec<u8>] {
        &self.cursors
    }

    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<Option<Step>, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(None);
        };
        if !is_command(command) {
            return Ok(None);
        }
        self.handle(&mut Call::new(vm, pc, command))
            .map(|flow| Some(flow.wait_step()))
    }
    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<crate::dispatch::Flow, DecodeError> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        if command == 0x98 {
            let raw = call.argument(4, "cursor name")?;
            let name = call.vm.resolve_string(raw)?;
            if name.len() >= 128 || self.cursors.len() >= 1024 {
                return Err(error(
                    pc,
                    op,
                    "cursor name or registry exceeds research limit",
                ));
            }
            let id = self.cursors.len() as u32 + 1;
            let event = call.finish(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, id)
                .map_err(|why| error(pc, op, why))?;
            self.cursors.push(name);
            self.handled += 1;
            return Ok(event);
        }
        let flags = match command {
            0x1d..=0x1f => {
                let raw = call.argument(4, argument_names(command)[0])?;
                self.settings.insert(op, raw);
                0x4004
            }
            0x90 | 0x34d => {
                self.settings.insert(op, 1);
                0x4000
            }
            0x91..=0x94 => {
                let start = call.argument(4, argument_names(command)[0])?;
                let count = call.argument(8, argument_names(command)[1])?;
                let raw = call.argument(12, argument_names(command)[2])?;
                if (start as i32) < 0 || (count as i32) < 0 {
                    return Err(error(pc, op, "negative initialization range"));
                }
                let result = match command {
                    0x91 => call.vm.state_mut().fill_bits(start, count, raw != 0),
                    0x92 => call.vm.state_mut().fill_integers(start, count, raw as i32),
                    0x93 => call.vm.state_mut().fill_floats(start, count, raw),
                    0x94 => {
                        let bytes = call.vm.resolve_string(raw)?;
                        call.vm.state_mut().fill_strings(start, count, &bytes)
                    }
                    _ => unreachable!(),
                };
                result.map_err(|why| error(pc, op, why))?;
                0x400c
            }
            0x10..=0x14 | 0x19 | 0x1a => {
                let slot = call.argument(4, argument_names(command)[0])?;
                let raw = call.argument(8, argument_names(command)[1])?;
                let limit = if command == 0x11 { 10 } else { 5 };
                if slot >= limit {
                    return Err(error(pc, op, "resource path slot outside supported range"));
                }
                let path = call.vm.resolve_string(raw)?;
                if path.len() >= 2048 {
                    return Err(error(pc, op, "resource path too long"));
                }
                self.paths.insert((op, slot), path);
                0x4008
            }
            0x15 | 0x16 => {
                let raw = call.argument(4, argument_names(command)[0])?;
                let path = call.vm.resolve_string(raw)?;
                if path.len() >= 2048 {
                    return Err(error(pc, op, "resource path too long"));
                }
                self.paths.insert((op, 0), path);
                0x4004
            }
            _ => unreachable!("registered bootstrap command"),
        };
        let event = call.finish(flags)?;
        self.handled += 1;
        Ok(event)
    }
}

fn is_command(command: u16) -> bool {
    matches!(command,0x10..=0x16|0x19|0x1a|0x1d..=0x1f|0x90..=0x94|0x98|0x34d)
}
fn argument_names(command: u16) -> &'static [&'static str] {
    match command {
        0x10..=0x14 | 0x19 | 0x1a => &["path slot", "resource path"],
        0x15 | 0x16 => &["resource path"],
        0x1d..=0x1f => &["setting value"],
        0x91..=0x94 => &["range start", "range count", "initial value"],
        _ => unreachable!("registered bootstrap arguments"),
    }
}

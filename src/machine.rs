//! Bounded multi-slot stepping. Unimplemented host commands remain pending.
use crate::{
    bytecode::{self, DecodeError, Expression, Instruction, InstructionKind, Term},
    evaluate::{State, error},
    ps3::Script,
    vm::{ParameterStack, StringRef, SystemResult},
};
use std::{borrow::Cow, collections::BTreeMap};

struct Image<'a> {
    script: Cow<'a, Script>,
    instructions: BTreeMap<usize, Instruction>,
    name: Vec<u8>,
}

impl<'a> Image<'a> {
    fn prepare(script: Cow<'a, Script>, name: Vec<u8>, entry: u32) -> Result<Self, DecodeError> {
        let instructions = bytecode::decode_layout(script.code(), entry, &script.labels())?
            .into_iter()
            .map(|i| (i.pc, i))
            .collect();
        Ok(Self {
            script,
            instructions,
            name,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Yield,
    System { pc: usize, command: u16 },
}

pub struct Machine<'a> {
    images: BTreeMap<u8, Image<'a>>,
    slot: u8,
    pc: usize,
    state: State,
    stack: ParameterStack,
    frames: Vec<usize>,
    cross_calls: Vec<u8>,
    pending: Option<(usize, u16)>,
    repeat_count: u32,
}

impl<'a> Machine<'a> {
    pub fn new(script: &'a Script) -> Result<Self, DecodeError> {
        Self::at(script, script.header.entry_pc)
    }
    /// An alternate entry is for isolated research, not an initialized game snapshot.
    pub fn at(script: &'a Script, entry: u32) -> Result<Self, DecodeError> {
        let image = Image::prepare(Cow::Borrowed(script), b"entry.ps3".to_vec(), entry)?;
        Ok(Self {
            images: BTreeMap::from([(0, image)]),
            slot: 0,
            pc: entry as usize,
            state: State::default(),
            stack: ParameterStack::new(65536),
            frames: Vec::new(),
            cross_calls: Vec::new(),
            pending: None,
            repeat_count: 0,
        })
    }
    pub fn pc(&self) -> usize {
        self.pc
    }
    fn image(&self) -> &Image<'a> {
        &self.images[&self.slot]
    }
    pub fn current_slot(&self) -> u8 {
        self.slot
    }
    pub fn current_name(&self) -> &[u8] {
        &self.image().name
    }
    pub fn set_current_name(&mut self, name: Vec<u8>) {
        self.images.get_mut(&self.slot).expect("active image").name = name;
    }
    pub fn loaded_script(&self, slot: u8) -> Option<&Script> {
        self.images.get(&slot).map(|i| i.script.as_ref())
    }
    pub fn loaded_names(&self) -> Vec<(u8, &[u8])> {
        self.images
            .iter()
            .map(|(slot, i)| (*slot, i.name.as_slice()))
            .collect()
    }
    pub fn is_boundary(&self, slot: u8, pc: usize) -> bool {
        self.images
            .get(&slot)
            .is_some_and(|image| image.instructions.contains_key(&pc))
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
    pub fn stack(&self) -> &ParameterStack {
        &self.stack
    }
    pub fn repeat_count(&self) -> u32 {
        self.repeat_count
    }
    pub fn pending_system(&self) -> Option<(usize, u16)> {
        self.pending
    }
    pub fn resolve_string(&self, raw: u32) -> Result<Vec<u8>, DecodeError> {
        let (bytes, offset) = match StringRef::decode(raw) {
            StringRef::Text(offset) => (self.image().script.text(), offset as usize),
            StringRef::ScriptData(offset) => (self.image().script.unknown(), offset as usize),
            StringRef::StringBank(index) => {
                let index = if index > 127 { 0 } else { index };
                return self
                    .state
                    .string(index)
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| error(self.pc, 0x201, "read of unknown string slot"));
            }
            StringRef::Frame(_) => {
                return Err(error(
                    self.pc,
                    0x201,
                    "frame string resolution not implemented",
                ));
            }
        };
        let tail = bytes
            .get(offset..)
            .ok_or_else(|| error(self.pc, 0x201, "string reference out of bounds"))?;
        let end = tail
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| error(self.pc, 0x201, "unterminated string reference"))?;
        Ok(tail[..end].to_vec())
    }

    fn expression(&mut self, expr: &Expression) -> Result<(), DecodeError> {
        match (expr.kind, expr.terms.as_slice()) {
            (0x200, _) => {
                self.state
                    .evaluate_integer_with_labels(expr, &self.image().script.labels())?;
            }
            (
                0x201,
                [
                    Term::Scalar {
                        tag: 0x120,
                        raw,
                        pc,
                    },
                ],
            ) => {
                let tail = self
                    .image()
                    .script
                    .text()
                    .get(*raw as usize..)
                    .ok_or_else(|| error(*pc, 0x120, "text reference out of bounds"))?;
                if !tail.contains(&0) {
                    return Err(error(*pc, 0x120, "unterminated text reference"));
                }
                self.state.result = Some(*raw);
                // This evaluator does not modify integer condition flags.
            }
            (
                0x202,
                [
                    Term::Scalar {
                        tag: 0x12a, raw, ..
                    },
                ],
            ) => {
                self.state.result = Some(*raw);
                self.state.condition = Some(f32::from_bits(*raw) != 0.0);
            }
            _ => {
                return Err(error(
                    expr.pc,
                    expr.kind,
                    "expression type/operation not implemented",
                ));
            }
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Step, DecodeError> {
        if let Some((pc, command)) = self.pending {
            return Ok(Step::System { pc, command });
        }
        let insn = self
            .image()
            .instructions
            .get(&self.pc)
            .cloned()
            .ok_or_else(|| error(self.pc, 0, "PC outside decoded instruction boundaries"))?;
        let pc = insn.pc;
        let op = insn.opcode;
        let mut next = insn.end;
        match insn.kind {
            InstructionKind::Expression(expr) => self.expression(&expr)?,
            InstructionKind::Jump { target } | InstructionKind::LongJump { target, .. } => {
                next = target as usize
            }
            InstructionKind::ConditionalJump { target } => {
                let condition = self
                    .state
                    .condition
                    .ok_or_else(|| error(pc, op, "condition flag is unknown"))?;
                if (op == 0x401 && !condition) || (op == 0x402 && condition) {
                    next = target as usize;
                }
            }
            InstructionKind::CompareJump { value, target } => {
                if self
                    .state
                    .result
                    .ok_or_else(|| error(pc, op, "result register is unknown"))?
                    == value
                {
                    next = target as usize;
                }
            }
            InstructionKind::Label { index } => {
                next = self.image().script.labels()[index as usize] as usize;
                if op == 0x410 {
                    if self.frames.len() >= 256 {
                        return Err(error(pc, op, "call frame research limit"));
                    }
                    self.stack
                        .push_u32(insn.end as u32)
                        .map_err(|e| error(pc, op, e.0))?;
                    self.frames.push(self.stack.len_bytes());
                }
            }
            InstructionKind::StackAdjust { bytes } if op == 0x412 => {
                self.stack
                    .discard(bytes as usize)
                    .map_err(|e| error(pc, op, e.0))?;
            }
            InstructionKind::StackAdjust { bytes } if op == 0x411 => {
                if self.frames.is_empty() {
                    return Err(error(pc, op, "return without a caller"));
                }
                next = self.stack.peek_u32(4).map_err(|e| error(pc, op, e.0))? as usize;
                if !self.is_boundary(self.slot, next) {
                    return Err(error(pc, op, "invalid return address"));
                }
                self.stack
                    .discard(4 + bytes as usize)
                    .map_err(|e| error(pc, op, e.0))?;
                self.frames.pop();
            }
            InstructionKind::Simple if op == 0x430 => {
                let raw = self
                    .state
                    .result
                    .ok_or_else(|| error(pc, op, "result register is unknown"))?;
                self.stack.push_u32(raw).map_err(|e| error(pc, op, e.0))?;
            }
            InstructionKind::StackImmediate { bytes: 4, raw } => {
                self.stack.push_u32(raw).map_err(|e| error(pc, op, e.0))?;
            }
            InstructionKind::ScriptReturn { .. } => {
                let slot = self.stack.peek_u32(4).map_err(|e| error(pc, op, e.0))?;
                next = self.stack.peek_u32(8).map_err(|e| error(pc, op, e.0))? as usize;
                let repeat = self.stack.peek_u32(12).map_err(|e| error(pc, op, e.0))?;
                if slot > 3
                    || !self.is_boundary(slot as u8, next)
                    || self.cross_calls.last().copied() != Some(slot as u8)
                {
                    return Err(error(pc, op, "invalid cross-script return context"));
                }
                self.stack.discard(12).map_err(|e| error(pc, op, e.0))?;
                self.slot = slot as u8;
                self.repeat_count = repeat;
                self.cross_calls.pop();
            }
            InstructionKind::System { command: 0 } => {
                self.pc = next;
                self.repeat_count = 0;
                return Ok(Step::Yield);
            }
            InstructionKind::System { command } => {
                self.pending = Some((pc, command));
                return Ok(Step::System { pc, command });
            }
            _ => return Err(error(pc, op, "instruction execution is not implemented")),
        }
        self.pc = next;
        Ok(Step::Continue)
    }

    fn transfer_stack(
        &self,
        argument_bytes: usize,
        op: u16,
    ) -> Result<ParameterStack, DecodeError> {
        if !self.is_boundary(self.slot, self.pc + 2) {
            return Err(error(
                self.pc,
                op,
                "cross-script return target is not an instruction boundary",
            ));
        }
        if self.cross_calls.len() >= 256 {
            return Err(error(self.pc, op, "cross-script call research limit"));
        }
        let mut stack = self.stack.clone();
        stack
            .discard(argument_bytes)
            .map_err(|e| error(self.pc, op, e.0))?;
        for value in [
            self.repeat_count,
            (self.pc + 2) as u32,
            u32::from(self.slot),
        ] {
            stack.push_u32(value).map_err(|e| error(self.pc, op, e.0))?;
        }
        Ok(stack)
    }

    /// Complete successful 0x2081: initialize public slot+1, then return via 0x0414.
    pub fn enter_common(
        &mut self,
        slot: u32,
        name: Vec<u8>,
        script: Script,
    ) -> Result<(), DecodeError> {
        if !matches!(self.pending, Some((_, 0x81))) {
            return Err(error(self.pc, 0x2081, "no pending common-script load"));
        }
        if slot >= 3 {
            return Err(error(self.pc, 0x2081, "common slot must be 0..2"));
        }
        let slot = slot as u8 + 1;
        if self.slot == slot || self.cross_calls.contains(&slot) {
            return Err(error(
                self.pc,
                0x2081,
                "cannot replace an active common script",
            ));
        }
        let entry = script.header.entry_pc as usize;
        let image = Image::prepare(Cow::Owned(script), name, entry as u32)?;
        let stack = self.transfer_stack(8, 0x2081)?;
        self.images.insert(slot, image);
        self.stack = stack;
        self.cross_calls.push(self.slot);
        self.slot = slot;
        self.pc = entry;
        self.repeat_count = 0;
        self.pending = None;
        Ok(())
    }

    /// Core 0x2080 transition only. The host must separately implement save snapshots.
    pub fn switch_main_core(&mut self, name: Vec<u8>, script: Script) -> Result<(), DecodeError> {
        if !matches!(self.pending, Some((_, 0x80))) {
            return Err(error(self.pc, 0x2080, "no pending main-script switch"));
        }
        self.stack
            .peek_u32(4)
            .map_err(|e| error(self.pc, 0x2080, e.0))?;
        let entry = script.header.entry_pc as usize;
        let image = Image::prepare(Cow::Owned(script), name, entry as u32)?;
        self.images.insert(0, image);
        self.slot = 0;
        self.pc = entry;
        self.repeat_count = 0;
        self.stack = ParameterStack::new(65536);
        self.frames.clear();
        self.cross_calls.clear();
        self.pending = None;
        Ok(())
    }

    pub fn call_script(&mut self, slot: u8, target: usize) -> Result<(), DecodeError> {
        if !matches!(self.pending, Some((_, 0x8a))) {
            return Err(error(self.pc, 0x208a, "no pending callback call"));
        }
        if !self.is_boundary(slot, target) {
            return Err(error(self.pc, 0x208a, "invalid callback script/target"));
        }
        let stack = self.transfer_stack(4, 0x208a)?;
        self.cross_calls.push(self.slot);
        self.slot = slot;
        self.pc = target;
        self.stack = stack;
        self.repeat_count = 0;
        self.pending = None;
        Ok(())
    }

    /// Resume an ordinary command after a host has explicitly implemented its effect.
    /// Script transfer requires its own future API; zero flags cannot be used to skip it.
    pub fn complete_system(&mut self, flags: SystemResult) -> Result<Step, DecodeError> {
        let (pc, command) = self
            .pending
            .ok_or_else(|| error(self.pc, 0, "no pending system command"))?;
        if !flags.advances_pc() || matches!(command, 0x80 | 0x81) {
            return Err(error(
                pc,
                command + 0x2000,
                "script transfer/retry needs a dedicated host implementation",
            ));
        }
        self.stack
            .apply_cleanup(flags)
            .map_err(|e| error(pc, command + 0x2000, e.0))?;
        self.pc = pc + 2;
        self.pending = None;
        self.repeat_count = if flags.increments_repeat_counter() {
            self.repeat_count.wrapping_add(1)
        } else {
            0
        };
        Ok(if flags.yields() {
            Step::Yield
        } else {
            Step::Continue
        })
    }
}

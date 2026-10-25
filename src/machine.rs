//! Bounded single-script stepping. Unimplemented host commands stop without guessing results.
use crate::{
    bytecode::{self, DecodeError, Expression, Instruction, InstructionKind, Term},
    evaluate::{State, error},
    ps3::Script,
    vm::{ParameterStack, StringRef, SystemResult},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Yield,
    System { pc: usize, command: u16 },
}

pub struct Machine<'a> {
    script: &'a Script,
    instructions: BTreeMap<usize, Instruction>,
    pc: usize,
    state: State,
    stack: ParameterStack,
    frames: Vec<usize>,
    pending: Option<(usize, u16)>,
    repeat_count: u32,
}

impl<'a> Machine<'a> {
    pub fn new(script: &'a Script) -> Result<Self, DecodeError> {
        Self::at(script, script.header.entry_pc)
    }
    /// An alternate entry is for isolated research, not an initialized game snapshot.
    pub fn at(script: &'a Script, entry: u32) -> Result<Self, DecodeError> {
        let instructions = bytecode::decode_layout(script.code(), entry, &script.labels())?
            .into_iter()
            .map(|i| (i.pc, i))
            .collect();
        Ok(Self {
            script,
            instructions,
            pc: entry as usize,
            state: State::default(),
            stack: ParameterStack::new(65536),
            frames: Vec::new(),
            pending: None,
            repeat_count: 0,
        })
    }
    pub fn pc(&self) -> usize {
        self.pc
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
            StringRef::Text(offset) => (self.script.text(), offset as usize),
            StringRef::ScriptData(offset) => (self.script.unknown(), offset as usize),
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
                self.state.evaluate_integer(expr)?;
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
                next = self.script.labels()[index as usize] as usize;
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
                if !self.instructions.contains_key(&next) {
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

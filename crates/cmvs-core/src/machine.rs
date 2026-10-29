//! Bounded multi-slot stepping. Unimplemented host commands remain pending.
use crate::{
    bytecode::{self, DecodeError, Expression, Instruction, InstructionKind, Term},
    evaluate::{Env, Frame, Journal, State, error},
    ps3::Script,
    vm::{ParameterStack, StringRef, SystemResult, frame_offset},
};
use std::{borrow::Cow, collections::BTreeMap, rc::Rc};

/// Frame table +82548 has 64 entries indexed by depth +82804; entry 0 is the base.
const MAX_CALL_DEPTH: usize = 63;
/// The +82864 accumulator and each string slot are 1024 bytes including the NUL.
const STRING_CAPACITY: usize = 1023;

fn c_string(bytes: &[u8], offset: u32, pc: usize, tag: u16) -> Result<&[u8], DecodeError> {
    let tail = bytes
        .get(offset as usize..)
        .ok_or_else(|| error(pc, tag, "string reference out of bounds"))?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| error(pc, tag, "unterminated string reference"))?;
    Ok(&tail[..end])
}
fn append(buffer: &mut Vec<u8>, bytes: &[u8], pc: usize, tag: u16) -> Result<(), DecodeError> {
    if buffer.len() + bytes.len() > STRING_CAPACITY {
        return Err(error(pc, tag, "string accumulator exceeds 1023 bytes"));
    }
    buffer.extend_from_slice(bytes);
    Ok(())
}
/// 0x140085ea0 copies only slots 0..127; other indexes leave the accumulator unchanged.
fn append_slot(
    state: &State,
    buffer: &mut Vec<u8>,
    index: u32,
    pc: usize,
    tag: u16,
) -> Result<(), DecodeError> {
    if index > 127 {
        return Ok(());
    }
    let bytes = state
        .string(index)
        .ok_or_else(|| error(pc, tag, "read of unknown string slot"))?;
    append(buffer, bytes, pc, tag)
}

struct Image<'a> {
    script: Cow<'a, Script>,
    data: Vec<u8>,
    /// Shared so each step clones a pointer, not an expression tree.
    instructions: BTreeMap<usize, Rc<Instruction>>,
    labels: Vec<u32>,
    name: Vec<u8>,
}

impl<'a> Image<'a> {
    fn prepare(script: Cow<'a, Script>, name: Vec<u8>, entry: u32) -> Result<Self, DecodeError> {
        let labels = script.labels();
        let instructions = bytecode::decode_layout(script.code(), entry, &labels)?
            .into_iter()
            .map(|i| (i.pc, Rc::new(i)))
            .collect();
        Ok(Self {
            data: script.unknown().to_vec(),
            script,
            instructions,
            labels,
            name,
        })
    }
}

/// The VM fields of an engine snapshot; see `Machine::capture_vm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmImage {
    pub script_names: [Vec<u8>; 4],
    pub script_sizes: [u32; 4],
    pub active_slot: u32,
    pub pc: u32,
    pub repeat_count: u32,
    pub stack_pointer: u32,
    pub frame_depth: u32,
    pub frames: [u32; 64],
    pub stack: Vec<u8>,
    pub system_results: [u32; 11],
    /// CSV2 section 2: the main script memory image with its current data segment.
    pub main_image: Vec<u8>,
    /// Record 0x900: the same data segment.
    pub main_data: Vec<u8>,
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
    /// VM domain of the engine snapshot 0x140069270: records 0x102..0x108, 0x112..0x115,
    /// 0x240..0x248, 0x900 and CSV2 section 2. Unknown stack bytes and unverified system
    /// results are written as zero; stale frame entries above the depth are not kept.
    pub fn capture_vm(&self) -> Result<VmImage, &'static str> {
        if !self.cross_calls.is_empty() {
            return Err("saving inside a cross-script call is not supported");
        }
        self.capture_vm_unchecked()
    }
    /// `capture_vm` without the cross-script check, for inspection only.
    pub(crate) fn capture_vm_unchecked(&self) -> Result<VmImage, &'static str> {
        if self.frames.len() > MAX_CALL_DEPTH {
            return Err("call depth exceeds the frame table");
        }
        let main = self.images.get(&0).ok_or("no main script image")?;
        let mut names: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::new());
        let mut sizes = [0u32; 4];
        for (slot, image) in &self.images {
            names[usize::from(*slot)] = image.name.clone();
            let data = if *slot == 0 { &main.data } else { &image.data };
            sizes[usize::from(*slot)] = image
                .script
                .memory_image(data)
                .map_err(|e| e.0)?
                .len()
                .try_into()
                .map_err(|_| "script image exceeds 4 GiB")?;
        }
        let mut frames = [0u32; 64];
        for (entry, &base) in frames[1..].iter_mut().zip(&self.frames) {
            *entry = base as u32;
        }
        let mut system_results = [0u32; 11];
        for (i, word) in system_results.iter_mut().take(7).enumerate() {
            *word = self.state.system_integer(i as u32).unwrap_or(0);
        }
        Ok(VmImage {
            script_names: names,
            script_sizes: sizes,
            active_slot: u32::from(self.slot),
            pc: self.pc as u32,
            repeat_count: self.repeat_count,
            stack_pointer: self.stack.len_bytes() as u32,
            frame_depth: self.frames.len() as u32,
            frames,
            stack: self.stack.saved_image(),
            system_results,
            main_image: main.script.memory_image(&main.data).map_err(|e| e.0)?,
            main_data: main.data.clone(),
        })
    }

    /// Loader 0x140081960 / 0x140080650 / 0x140069fd0 for the VM domain: the saved main
    /// image replaces slot 0 with its data segment; common scripts stay loaded and must carry
    /// the saved names. Registers outside the snapshot (expression result, condition, string
    /// accumulator) keep their current values, as in the original.
    pub fn restore_vm(&mut self, image: &VmImage) -> Result<(), &'static str> {
        let mut script = Script::from_memory_image(&image.main_image).map_err(|e| e.0)?;
        script
            .apply_snapshot_data(&image.main_data)
            .map_err(|e| e.0)?;
        for slot in 1..4u8 {
            let loaded = self.images.get(&slot).map(|i| i.name.as_slice());
            let saved = &image.script_names[usize::from(slot)];
            if loaded.unwrap_or_default() != saved.as_slice() {
                return Err("saved common script differs from the loaded one");
            }
        }
        if image.active_slot > 3
            || (image.active_slot != 0 && !self.images.contains_key(&(image.active_slot as u8)))
        {
            return Err("saved active script slot is not loaded");
        }
        let depth = image.frame_depth as usize;
        let sp = image.stack_pointer as usize;
        if depth > MAX_CALL_DEPTH || image.frames[0] != 0 {
            return Err("saved call frames exceed the frame table");
        }
        let frames: Vec<usize> = image.frames[1..=depth]
            .iter()
            .map(|&b| b as usize)
            .collect();
        if frames.iter().any(|&base| base > sp) {
            return Err("saved call frame lies above the stack pointer");
        }
        let stack = ParameterStack::from_saved(&image.stack, sp, 65536).map_err(|e| e.0)?;
        let entry = script.header.entry_pc;
        let main = Image::prepare(Cow::Owned(script), image.script_names[0].clone(), entry)
            .map_err(|_| "saved main script layout is invalid")?;
        let mut candidate_state = self.state.clone();
        for (i, &word) in image.system_results.iter().take(7).enumerate() {
            candidate_state.set_system_integer(i as u32, word)?;
        }
        let slot = image.active_slot as u8;
        if slot == 0 && !main.instructions.contains_key(&(image.pc as usize)) {
            return Err("saved PC is not an instruction boundary");
        }
        if slot != 0 && !self.is_boundary(slot, image.pc as usize) {
            return Err("saved PC is not an instruction boundary");
        }
        self.images.insert(0, main);
        self.slot = slot;
        self.pc = image.pc as usize;
        self.repeat_count = image.repeat_count;
        self.stack = stack;
        self.frames = frames;
        self.cross_calls.clear();
        self.pending = None;
        self.state = candidate_state;
        Ok(())
    }

    /// 0x140068770: when callback `index` is registered, push the current repeat count, PC
    /// and slot as a cross-script return frame and continue at the callback with repeat 0.
    pub(crate) fn enter_callback(&mut self, slot: u8, target: usize) -> Result<(), &'static str> {
        if !self.is_boundary(slot, target) {
            return Err("callback target is not an instruction boundary");
        }
        let mut stack = self.stack.clone();
        for value in [self.repeat_count, self.pc as u32, u32::from(self.slot)] {
            stack.push_u32(value).map_err(|e| e.0)?;
        }
        self.stack = stack;
        self.cross_calls.push(self.slot);
        self.slot = slot;
        self.pc = target;
        self.repeat_count = 0;
        Ok(())
    }
    pub(crate) fn set_repeat_count(&mut self, count: u32) {
        self.repeat_count = count;
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
    /// Current slot's mutable script data, initialized from the PS2A unknown segment.
    pub fn script_data(&self) -> &[u8] {
        &self.image().data
    }
    pub fn repeat_count(&self) -> u32 {
        self.repeat_count
    }
    pub fn pending_system(&self) -> Option<(usize, u16)> {
        self.pending
    }
    pub fn resolve_string(&self, raw: u32) -> Result<Vec<u8>, DecodeError> {
        let (bytes, offset) = match StringRef::decode(raw) {
            StringRef::Text(offset) => (self.image().script.text(), offset),
            StringRef::ScriptData(offset) => (self.script_data(), offset),
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
        Ok(c_string(bytes, offset, self.pc, 0x201)?.to_vec())
    }

    /// Word at the current call frame minus `distance`, as tag 0x122/0x103 operands read it.
    fn frame_word(&self, distance: u32, pc: usize, tag: u16) -> Result<u32, DecodeError> {
        let offset = frame_offset(self.frame_base(), distance, true)
            .ok_or_else(|| error(pc, tag, "frame operand below stack base"))?;
        self.stack
            .read_u32_at(offset)
            .map_err(|e| error(pc, tag, e.0))
    }

    /// Operand reader 0x1400652e0 for string tags: appends to the accumulator.
    fn append_operand(
        &self,
        state: &State,
        buffer: &mut Vec<u8>,
        (tag, value, pc): (u16, u32, usize),
    ) -> Result<(), DecodeError> {
        let script = &self.image().script;
        let bytes = match tag {
            // Placeholder left by a previous operator: the original reader returns 0.
            0 => return Ok(()),
            0x120 => c_string(script.text(), value, pc, tag)?,
            0x125 => c_string(self.script_data(), value, pc, tag)?,
            0x121 => return append_slot(state, buffer, value & 0x3fff_ffff, pc, tag),
            0x122 => match StringRef::decode(self.frame_word(value, pc, tag)?) {
                StringRef::Text(offset) => c_string(script.text(), offset, pc, tag)?,
                StringRef::ScriptData(offset) => c_string(self.script_data(), offset, pc, tag)?,
                StringRef::StringBank(index) => {
                    return append_slot(state, buffer, index, pc, tag);
                }
                StringRef::Frame(_) => {
                    return Err(error(pc, tag, "frame-local string not implemented"));
                }
            },
            _ => return Err(error(pc, tag, "string operand type not implemented")),
        };
        append(buffer, bytes, pc, tag)
    }

    /// 0x0201 / 0x140066b00: clears the accumulator, appends operands as operators read them,
    /// and sets the result only from the tag of the bottom entry. Commits all writes or none.
    fn string_expression(&mut self, expr: &Expression) -> Result<(), DecodeError> {
        let mut journal = Journal::default();
        let outcome = self.string_expression_in(expr, &mut journal);
        if outcome.is_err() {
            self.state.rollback(
                journal,
                Some(&mut self.stack),
                Some(&mut self.images.get_mut(&self.slot).expect("active image").data),
            );
        }
        outcome
    }
    fn string_expression_in(
        &mut self,
        expr: &Expression,
        journal: &mut Journal,
    ) -> Result<(), DecodeError> {
        let mut buffer = Vec::new();
        let mut entries: Vec<(u16, u32, usize)> = Vec::new();
        for term in &expr.terms {
            match term {
                Term::Nested {
                    pc,
                    tag: 0x121,
                    expression,
                } => {
                    // The nested integer evaluator also updates result, condition and integer 4096.
                    let base = self.frame_base();
                    let image = self.images.get_mut(&self.slot).expect("active image");
                    let mut env = Env {
                        labels: &image.labels,
                        data: Some(&mut image.data),
                        frame: Some(Frame {
                            stack: &mut self.stack,
                            base,
                        }),
                    };
                    let index = self.state.evaluate_in(expression, &mut env, journal)?;
                    entries.push((0x121, index as u32 | 0x8000_0000, *pc));
                }
                Term::Scalar { pc, tag, raw } => entries.push((*tag, *raw, *pc)),
                Term::Operator { pc, opcode } if (0x160..=0x172).contains(opcode) => {
                    let top = entries
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let second = *entries
                        .last()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    if *opcode == 0x170 {
                        self.append_operand(&self.state, &mut buffer, top)?;
                        match second {
                            (0x121, value, _) if value & 0x3fff_ffff > 127 => {}
                            (0x121, value, at) => self
                                .state
                                .set_string_journaled(value & 0x3fff_ffff, &buffer, journal)
                                .map_err(|why| error(at, 0x121, why))?,
                            (tag, _, at) => {
                                return Err(error(
                                    at,
                                    tag,
                                    "string assignment target not implemented",
                                ));
                            }
                        }
                    } else {
                        // Binary operators only read both operands, so strings concatenate.
                        self.append_operand(&self.state, &mut buffer, second)?;
                        self.append_operand(&self.state, &mut buffer, top)?;
                    }
                    *entries.last_mut().expect("checked above") = (0, 0, *pc);
                }
                Term::Nested { pc, tag, .. }
                | Term::Indexed { pc, tag, .. }
                | Term::Operator { pc, opcode: tag } => {
                    return Err(error(*pc, *tag, "string expression term not implemented"));
                }
            }
        }
        let &(tag, value, pc) = entries
            .first()
            .ok_or_else(|| error(expr.pc, expr.kind, "empty string expression"))?;
        let result = match tag {
            0x120 => {
                c_string(self.image().script.text(), value, pc, tag)?;
                Some(value)
            }
            0x121 => Some(value),
            0x122 => Some(self.frame_word(value, pc, tag)?),
            0x125 => Some(value | 0x4000_0000),
            0x127 => Some(value | 0xc000_0000),
            _ => None, // The original leaves +82812 unchanged.
        };
        if result.is_some() {
            self.state.set_result_journaled(result, journal);
        }
        // This evaluator does not modify integer condition flags itself.
        self.state.set_string_buffer_journaled(buffer, journal);
        Ok(())
    }

    /// frame[depth]: 0x140080530 zeroes entry 0 with SP, and 0x0410 writes only depth >= 1.
    fn frame_base(&self) -> usize {
        self.frames.last().copied().unwrap_or(0)
    }
    /// Runs a numeric evaluation with the current labels and frame; rolls back on failure.
    fn numeric<T>(
        &mut self,
        run: impl FnOnce(&mut State, &mut Env, &mut Journal) -> Result<T, DecodeError>,
    ) -> Result<T, DecodeError> {
        let base = self.frame_base();
        let mut journal = Journal::default();
        let outcome = {
            let image = self.images.get_mut(&self.slot).expect("active image");
            let mut env = Env {
                labels: &image.labels,
                data: Some(&mut image.data),
                frame: Some(Frame {
                    stack: &mut self.stack,
                    base,
                }),
            };
            run(&mut self.state, &mut env, &mut journal)
        };
        if outcome.is_err() {
            self.state.rollback(
                journal,
                Some(&mut self.stack),
                Some(&mut self.images.get_mut(&self.slot).expect("active image").data),
            );
        }
        outcome
    }

    fn expression(&mut self, expr: &Expression) -> Result<(), DecodeError> {
        match expr.kind {
            0x200 => {
                self.numeric(|state, env, journal| state.evaluate_in(expr, env, journal))?;
            }
            0x201 => self.string_expression(expr)?,
            0x202 => {
                self.numeric(|state, env, journal| state.evaluate_float_in(expr, env, journal))?;
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
        match &insn.kind {
            InstructionKind::Expression(expr) => self.expression(expr)?,
            InstructionKind::Jump { target } | InstructionKind::LongJump { target, .. } => {
                next = *target as usize
            }
            InstructionKind::ConditionalJump { target } => {
                let condition = self
                    .state
                    .condition
                    .ok_or_else(|| error(pc, op, "condition flag is unknown"))?;
                if (op == 0x401 && !condition) || (op == 0x402 && condition) {
                    next = *target as usize;
                }
            }
            InstructionKind::CompareJump { value, target } => {
                if self
                    .state
                    .result
                    .ok_or_else(|| error(pc, op, "result register is unknown"))?
                    == *value
                {
                    next = *target as usize;
                }
            }
            InstructionKind::Label { index } => {
                next = self.image().labels[*index as usize] as usize;
                if op == 0x410 {
                    if self.frames.len() >= MAX_CALL_DEPTH {
                        return Err(error(pc, op, "call frame table holds 63 nested calls"));
                    }
                    self.stack
                        .push_u32(insn.end as u32)
                        .map_err(|e| error(pc, op, e.0))?;
                    self.frames.push(self.stack.len_bytes());
                }
            }
            InstructionKind::StackAdjust { bytes } if op == 0x412 => {
                self.stack
                    .discard(*bytes as usize)
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
                    .discard(4 + *bytes as usize)
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
            InstructionKind::StackImmediate { bytes, raw } => {
                self.stack
                    .reserve(*raw, usize::from(*bytes))
                    .map_err(|e| error(pc, op, e.0))?;
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
            &InstructionKind::System { command } => {
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

    /// Verified 0xa000 return: yield and increment repeat count without advancing PC or SP.
    pub fn yield_pending_system(&mut self) -> Result<Step, DecodeError> {
        if self.pending.is_none() {
            return Err(error(self.pc, 0, "no pending system command"));
        }
        self.repeat_count = self.repeat_count.wrapping_add(1);
        Ok(Step::Yield)
    }
}

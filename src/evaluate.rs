//! Stateful integer evaluation with explicit unknown values and transactional errors.
use crate::bytecode::{DecodeError, Expression, Term};
use std::collections::BTreeMap;

pub(crate) fn error(pc: usize, opcode: u16, reason: &'static str) -> DecodeError {
    DecodeError {
        pc,
        opcode: Some(opcode),
        reason,
    }
}

#[derive(Debug)]
enum Undo {
    Integer(u32, Option<i32>),
    Bit(u32, Option<bool>),
    String(u32, Option<Vec<u8>>),
    Buffer(Vec<u8>),
    Result(Option<u32>),
    Condition(Option<bool>),
}

/// Previous values recorded before each write, so a failed evaluation is undone
/// without cloning the whole State.
#[derive(Debug, Default)]
pub(crate) struct Journal(Vec<Undo>);

/// Bounded research state. Missing variables are unknown, not implicitly zero.
#[derive(Debug, Clone)]
pub struct State {
    integers: BTreeMap<u32, i32>,
    bits: BTreeMap<u32, bool>,
    floats: BTreeMap<u32, u32>,
    strings: BTreeMap<u32, Vec<u8>>,
    system_integers: [Option<u32>; 7],
    integer_limit: u32,
    bit_limit: u32,
    string_buffer: Vec<u8>,
    pub result: Option<u32>,
    pub condition: Option<bool>,
}

impl Default for State {
    fn default() -> Self {
        Self::new(8192, 262144)
    }
}

impl State {
    /// Limits are research policy, not assertions about all CMVS versions.
    pub fn new(integer_limit: u32, bit_limit: u32) -> Self {
        Self {
            integers: BTreeMap::new(),
            bits: BTreeMap::new(),
            floats: BTreeMap::new(),
            strings: BTreeMap::new(),
            system_integers: [None; 7],
            integer_limit,
            bit_limit,
            string_buffer: Vec::new(),
            result: None,
            condition: None,
        }
    }
    pub fn integer(&self, index: u32) -> Option<i32> {
        self.integers.get(&index).copied()
    }
    /// Integer system result slots +82816..+82840, distinct from expression result +82812.
    pub fn system_integer(&self, index: u32) -> Option<u32> {
        self.system_integers.get(index as usize).copied().flatten()
    }
    pub fn set_system_integer(&mut self, index: u32, value: u32) -> Result<(), &'static str> {
        let slot = self
            .system_integers
            .get_mut(index as usize)
            .ok_or("integer system result slot must be 0..6")?;
        *slot = Some(value);
        Ok(())
    }
    pub fn bit(&self, index: u32) -> Option<bool> {
        self.bits.get(&index).copied()
    }
    pub fn integers(&self) -> &BTreeMap<u32, i32> {
        &self.integers
    }
    pub fn bits(&self) -> &BTreeMap<u32, bool> {
        &self.bits
    }
    pub fn float_bits(&self, index: u32) -> Option<u32> {
        self.floats.get(&index).copied()
    }
    pub fn string(&self, index: u32) -> Option<&[u8]> {
        self.strings.get(&index).map(Vec::as_slice)
    }
    /// The 1024-byte accumulator at +82864, cleared by every 0x0201 evaluation.
    pub fn string_buffer(&self) -> &[u8] {
        &self.string_buffer
    }
    pub(crate) fn set_string_buffer_journaled(&mut self, bytes: Vec<u8>, journal: &mut Journal) {
        let old = std::mem::replace(&mut self.string_buffer, bytes);
        journal.0.push(Undo::Buffer(old));
    }
    pub(crate) fn set_result_journaled(&mut self, value: Option<u32>, journal: &mut Journal) {
        journal.0.push(Undo::Result(self.result));
        self.result = value;
    }
    pub(crate) fn set_string_journaled(
        &mut self,
        index: u32,
        bytes: &[u8],
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        Self::range(index, 1, 128)?;
        if bytes.len() >= 1024 || bytes.contains(&0) {
            return Err("invalid bounded string slot value");
        }
        let old = self.strings.insert(index, bytes.to_vec());
        journal.0.push(Undo::String(index, old));
        Ok(())
    }
    fn set_integer_journaled(
        &mut self,
        index: u32,
        value: i32,
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        if index >= self.integer_limit {
            return Err("integer variable outside research limit");
        }
        let old = self.integers.insert(index, value);
        journal.0.push(Undo::Integer(index, old));
        Ok(())
    }
    fn set_bit_journaled(
        &mut self,
        index: u32,
        value: bool,
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        if index >= self.bit_limit {
            return Err("bit variable outside research limit");
        }
        let old = self.bits.insert(index, value);
        journal.0.push(Undo::Bit(index, old));
        Ok(())
    }
    /// Restores every journaled write in reverse order.
    pub(crate) fn rollback(&mut self, journal: Journal) {
        fn restore<V>(map: &mut BTreeMap<u32, V>, index: u32, old: Option<V>) {
            match old {
                Some(value) => map.insert(index, value),
                None => map.remove(&index),
            };
        }
        for undo in journal.0.into_iter().rev() {
            match undo {
                Undo::Integer(index, old) => restore(&mut self.integers, index, old),
                Undo::Bit(index, old) => restore(&mut self.bits, index, old),
                Undo::String(index, old) => restore(&mut self.strings, index, old),
                Undo::Buffer(old) => self.string_buffer = old,
                Undo::Result(old) => self.result = old,
                Undo::Condition(old) => self.condition = old,
            }
        }
    }
    pub fn set_string(&mut self, index: u32, bytes: &[u8]) -> Result<(), &'static str> {
        self.fill_strings(index, 1, bytes)
    }
    fn range(start: u32, count: u32, limit: u32) -> Result<std::ops::Range<u32>, &'static str> {
        let end = start.checked_add(count).ok_or("variable range overflow")?;
        if end > limit {
            return Err("variable range outside research limit");
        }
        Ok(start..end)
    }
    pub fn fill_integers(
        &mut self,
        start: u32,
        count: u32,
        value: i32,
    ) -> Result<(), &'static str> {
        for index in Self::range(start, count, self.integer_limit)? {
            self.integers.insert(index, value);
        }
        Ok(())
    }
    pub fn fill_bits(&mut self, start: u32, count: u32, value: bool) -> Result<(), &'static str> {
        for index in Self::range(start, count, self.bit_limit)? {
            self.bits.insert(index, value);
        }
        Ok(())
    }
    pub fn fill_floats(&mut self, start: u32, count: u32, bits: u32) -> Result<(), &'static str> {
        // Research capacity; the model preserves raw f32 bits, including NaN payloads.
        for index in Self::range(start, count, 2048)? {
            self.floats.insert(index, bits);
        }
        Ok(())
    }
    pub fn fill_strings(
        &mut self,
        start: u32,
        count: u32,
        bytes: &[u8],
    ) -> Result<(), &'static str> {
        if bytes.len() >= 1024 || bytes.contains(&0) {
            return Err("invalid bounded string slot value");
        }
        for index in Self::range(start, count, 128)? {
            self.strings.insert(index, bytes.to_vec());
        }
        Ok(())
    }
    pub fn set_integer(&mut self, index: u32, value: i32) -> Result<(), &'static str> {
        if index >= self.integer_limit {
            return Err("integer variable outside research limit");
        }
        self.integers.insert(index, value);
        Ok(())
    }
    pub fn set_bit(&mut self, index: u32, value: bool) -> Result<(), &'static str> {
        if index >= self.bit_limit {
            return Err("bit variable outside research limit");
        }
        self.bits.insert(index, value);
        Ok(())
    }
    /// Commit all nested expression writes only if the complete expression succeeds.
    pub fn evaluate_integer(&mut self, expr: &Expression) -> Result<i32, DecodeError> {
        self.evaluate_integer_with_labels(expr, &[])
    }
    pub fn evaluate_integer_with_labels(
        &mut self,
        expr: &Expression,
        labels: &[u32],
    ) -> Result<i32, DecodeError> {
        let mut journal = Journal::default();
        let outcome = self.evaluate(expr, 0, labels, &mut journal);
        if outcome.is_err() {
            self.rollback(journal);
        }
        outcome
    }
    /// Nested evaluation inside a larger transaction; the caller owns rollback.
    pub(crate) fn evaluate_journaled(
        &mut self,
        expr: &Expression,
        labels: &[u32],
        journal: &mut Journal,
    ) -> Result<i32, DecodeError> {
        self.evaluate(expr, 0, labels, journal)
    }
    fn read(&self, cell: Cell, pc: usize, op: u16) -> Result<i32, DecodeError> {
        match cell {
            Cell::Value(value) => Ok(value),
            Cell::Integer(index) => self
                .integer(index)
                .ok_or_else(|| error(pc, op, "read of unknown integer variable")),
            Cell::Bit(index) => self
                .bit(index)
                .map(i32::from)
                .ok_or_else(|| error(pc, op, "read of unknown bit variable")),
        }
    }
    fn write(
        &mut self,
        cell: Cell,
        value: i32,
        pc: usize,
        op: u16,
        journal: &mut Journal,
    ) -> Result<(), DecodeError> {
        match cell {
            Cell::Integer(index) => self.set_integer_journaled(index, value, journal),
            Cell::Bit(index) => self.set_bit_journaled(index, value != 0, journal),
            Cell::Value(_) => Err("assignment requires a writable variable"),
        }
        .map_err(|why| error(pc, op, why))
    }
    fn evaluate(
        &mut self,
        expr: &Expression,
        depth: usize,
        labels: &[u32],
        journal: &mut Journal,
    ) -> Result<i32, DecodeError> {
        if depth >= 64 {
            return Err(error(
                expr.pc,
                expr.kind,
                "expression evaluation depth limit",
            ));
        }
        if expr.kind != 0x200 {
            return Err(error(expr.pc, expr.kind, "not an integer expression"));
        }
        let mut stack = Vec::new();
        for term in &expr.terms {
            match term {
                Term::Scalar {
                    tag: 0x100, raw, ..
                } => stack.push(Cell::Value(*raw as i32)),
                Term::Scalar {
                    pc,
                    tag: 0x10e,
                    raw,
                } => {
                    let target = labels
                        .get(*raw as usize)
                        .ok_or_else(|| error(*pc, 0x10e, "label operand index out of bounds"))?;
                    stack.push(Cell::Value(*target as i32));
                }
                Term::Scalar {
                    pc,
                    tag: 0x10f,
                    raw,
                } => {
                    let value = match *raw {
                        0..=6 => self
                            .system_integer(*raw)
                            .ok_or_else(|| error(*pc, 0x10f, "unknown integer system result"))?,
                        7..=10 => {
                            return Err(error(
                                *pc,
                                0x10f,
                                "floating system result conversion not implemented",
                            ));
                        }
                        _ => 0, // Original operand reader returns zero outside slots 0..10.
                    };
                    stack.push(Cell::Value(value as i32));
                }
                Term::Nested {
                    pc,
                    tag,
                    expression,
                } if matches!(tag, 0x200 | 0x101 | 0x102) => {
                    let value = self.evaluate(expression, depth + 1, labels, journal)?;
                    let cell = match tag {
                        0x200 => Cell::Value(value),
                        0x101 if value >= 0 && (value as u32) < self.integer_limit => {
                            Cell::Integer(value as u32)
                        }
                        0x102 if value >= 0 && (value as u32) < self.bit_limit => {
                            Cell::Bit(value as u32)
                        }
                        _ => return Err(error(*pc, *tag, "variable index outside research limit")),
                    };
                    stack.push(cell);
                }
                Term::Operator {
                    pc,
                    opcode: op @ (0x173 | 0x174),
                } => {
                    let cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *op, "expression stack underflow"))?;
                    let old = self.read(cell, *pc, *op)?;
                    let value = if *op == 0x173 {
                        old.wrapping_add(1)
                    } else {
                        old.wrapping_sub(1)
                    };
                    self.write(cell, value, *pc, *op, journal)?;
                    stack.push(Cell::Value(value));
                }
                Term::Operator { pc, opcode } => {
                    let right_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let left_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let value = if *opcode == 0x170 {
                        let right = self.read(right_cell, *pc, *opcode)?;
                        self.write(left_cell, right, *pc, *opcode, journal)?;
                        right
                    } else {
                        // Preserve descriptor laziness: nested writes may change either source.
                        let left = self.read(left_cell, *pc, *opcode)?;
                        let right = self.read(right_cell, *pc, *opcode)?;
                        let binary = match opcode {
                            0x175 => 0x163,
                            0x176 => 0x164,
                            0x177 => 0x160,
                            0x178 => 0x161,
                            0x179 => 0x165,
                            0x17a => 0x166,
                            0x17b => 0x162,
                            0x17c => 0x167,
                            0x17d => 0x168,
                            0x17e => 0x169,
                            other => *other,
                        };
                        let value = binary_value(*pc, binary, left, right)?;
                        if (0x175..=0x17e).contains(opcode) {
                            self.write(left_cell, value, *pc, *opcode, journal)?;
                        }
                        value
                    };
                    stack.push(Cell::Value(value));
                }
                Term::Scalar { pc, tag, .. }
                | Term::Nested { pc, tag, .. }
                | Term::Indexed { pc, tag, .. } => {
                    return Err(error(*pc, *tag, "operand storage/type is not implemented"));
                }
            }
        }
        if stack.len() != 1 {
            return Err(error(
                expr.pc,
                expr.kind,
                "expression does not produce exactly one value",
            ));
        }
        let value = self.read(stack[0], expr.pc, expr.kind)?;
        // The original integer evaluator writes its result to global integer 4096.
        self.set_integer_journaled(4096, value, journal)
            .map_err(|why| error(expr.pc, expr.kind, why))?;
        self.set_result_journaled(Some(value as u32), journal);
        journal.0.push(Undo::Condition(self.condition));
        self.condition = Some(value != 0);
        Ok(value)
    }
}

#[derive(Clone, Copy)]
enum Cell {
    Value(i32),
    Integer(u32),
    Bit(u32),
}

pub(crate) fn binary_value(
    pc: usize,
    opcode: u16,
    left: i32,
    right: i32,
) -> Result<i32, DecodeError> {
    Ok(match opcode {
        0x160 => left.wrapping_mul(right),
        0x161 => left
            .checked_div(right)
            .ok_or_else(|| error(pc, opcode, "invalid signed division"))?,
        0x162 => left
            .checked_rem(right)
            .ok_or_else(|| error(pc, opcode, "invalid signed remainder"))?,
        0x163 => left.wrapping_add(right),
        0x164 => left.wrapping_sub(right),
        0x165 => left & right,
        0x166 => left | right,
        0x167 => left ^ right,
        0x168 => left.wrapping_shl(right as u32),
        0x169 => left.wrapping_shr(right as u32),
        0x16a => i32::from(left > right),
        0x16b => i32::from(left >= right),
        0x16c => i32::from(left < right),
        0x16d => i32::from(left <= right),
        0x16e => i32::from(left != 0 && right != 0),
        0x16f => i32::from(left != 0 || right != 0),
        0x171 => i32::from(left == right),
        0x172 => i32::from(left != right),
        _ => return Err(error(pc, opcode, "unsupported integer operator")),
    })
}

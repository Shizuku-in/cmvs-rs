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

/// Bounded research state. Missing variables are unknown, not implicitly zero.
#[derive(Debug, Clone)]
pub struct State {
    integers: BTreeMap<u32, i32>,
    bits: BTreeMap<u32, bool>,
    integer_limit: u32,
    bit_limit: u32,
    pub result: Option<u32>,
    pub condition: Option<bool>,
}

impl Default for State {
    fn default() -> Self {
        Self::new(8192, 65536)
    }
}

impl State {
    /// Limits are research policy, not assertions about all CMVS versions.
    pub fn new(integer_limit: u32, bit_limit: u32) -> Self {
        Self {
            integers: BTreeMap::new(),
            bits: BTreeMap::new(),
            integer_limit,
            bit_limit,
            result: None,
            condition: None,
        }
    }
    pub fn integer(&self, index: u32) -> Option<i32> {
        self.integers.get(&index).copied()
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
        let mut candidate = self.clone();
        let value = candidate.evaluate(expr, 0)?;
        *self = candidate;
        Ok(value)
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
    fn write(&mut self, cell: Cell, value: i32, pc: usize, op: u16) -> Result<(), DecodeError> {
        match cell {
            Cell::Integer(index) => self.set_integer(index, value),
            Cell::Bit(index) => self.set_bit(index, value != 0),
            Cell::Value(_) => Err("assignment requires a writable variable"),
        }
        .map_err(|why| error(pc, op, why))
    }
    fn evaluate(&mut self, expr: &Expression, depth: usize) -> Result<i32, DecodeError> {
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
                Term::Nested {
                    pc,
                    tag,
                    expression,
                } if matches!(tag, 0x200 | 0x101 | 0x102) => {
                    let value = self.evaluate(expression, depth + 1)?;
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
                    self.write(cell, value, *pc, *op)?;
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
                        self.write(left_cell, right, *pc, *opcode)?;
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
                            self.write(left_cell, value, *pc, *opcode)?;
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
        self.set_integer(4096, value)
            .map_err(|why| error(expr.pc, expr.kind, why))?;
        self.result = Some(value as u32);
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

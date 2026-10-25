//! Conservative structural decoding, verified against one Amatsutsumi x64 build.
//! This is linear disassembly, not control-flow execution or a complete CMVS VM.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub pc: usize,
    pub opcode: Option<u16>,
    pub reason: &'static str,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code+0x{:x}", self.pc)?;
        if let Some(op) = self.opcode {
            write!(f, " opcode=0x{op:04x}")?;
        }
        write!(f, ": {}", self.reason)
    }
}
impl std::error::Error for DecodeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Scalar {
        pc: usize,
        tag: u16,
        raw: u32,
    },
    Nested {
        pc: usize,
        tag: u16,
        expression: Box<Expression>,
    },
    Indexed {
        pc: usize,
        tag: u16,
        base: u32,
        stride: u16,
        indices: Vec<Expression>,
    },
    Operator {
        pc: usize,
        opcode: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    pub pc: usize,
    /// 0x0200 integer, 0x0201 string reference, 0x0202 float.
    pub kind: u16,
    pub end: usize,
    pub terms: Vec<Term>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub pc: usize,
    pub end: usize,
    pub opcode: u16,
    pub kind: InstructionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionKind {
    Expression(Expression),
    Jump { target: u32 },
    ConditionalJump { target: u32 },
    CompareJump { value: u32, target: u32 },
    LongJump { reserved: u32, target: u32 },
    Label { index: u16 },
    StackAdjust { bytes: u16 },
    StackImmediate { bytes: u16, raw: u32 },
    Simple,
    System { command: u16 },
}

struct Reader<'a> {
    code: &'a [u8],
    pos: usize,
}
impl Reader<'_> {
    fn error(&self, pc: usize, opcode: Option<u16>, reason: &'static str) -> DecodeError {
        DecodeError { pc, opcode, reason }
    }
    fn word(&mut self) -> Result<u16, DecodeError> {
        let bytes = self
            .code
            .get(self.pos..self.pos + 2)
            .ok_or_else(|| self.error(self.pos, None, "truncated 16-bit operand"))?;
        self.pos += 2;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn dword(&mut self) -> Result<u32, DecodeError> {
        let bytes = self
            .code
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error(self.pos, None, "truncated 32-bit operand"))?;
        self.pos += 4;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn expression(&mut self, depth: usize) -> Result<Expression, DecodeError> {
        let pc = self.pos;
        if depth >= 64 {
            return Err(self.error(pc, None, "expression nesting limit"));
        }
        let kind = self.word()?;
        if !(0x200..=0x202).contains(&kind) {
            return Err(self.error(pc, Some(kind), "expected expression prefix"));
        }
        let mut terms = Vec::new();
        loop {
            let at = self.pos;
            let tag = self.word()?;
            if tag == 0x20f {
                if terms.is_empty() {
                    return Err(self.error(at, Some(tag), "empty expression"));
                }
                return Ok(Expression {
                    pc,
                    kind,
                    end: self.pos,
                    terms,
                });
            }
            if terms.len() >= 128 {
                return Err(self.error(at, Some(tag), "expression term limit"));
            }
            let term = match tag {
                0x200 if kind != 0x201 => {
                    self.pos = at;
                    Term::Nested {
                        pc: at,
                        tag,
                        expression: Box::new(self.expression(depth + 1)?),
                    }
                }
                0x101 | 0x102 | 0x12b if kind != 0x201 => {
                    let expression = self.expression(depth + 1)?;
                    if expression.kind != 0x200 {
                        return Err(self.error(at, Some(tag), "index must be integer expression"));
                    }
                    Term::Nested {
                        pc: at,
                        tag,
                        expression: Box::new(expression),
                    }
                }
                0x121 if kind == 0x201 => {
                    let expression = self.expression(depth + 1)?;
                    if expression.kind != 0x200 {
                        return Err(self.error(at, Some(tag), "string index must be integer"));
                    }
                    Term::Nested {
                        pc: at,
                        tag,
                        expression: Box::new(expression),
                    }
                }
                0x105 | 0x109 | 0x110 | 0x112 | 0x12d | 0x12f if kind != 0x201 => {
                    let base = self.dword()?;
                    let expression = self.expression(depth + 1)?;
                    if expression.kind != 0x200 {
                        return Err(self.error(at, Some(tag), "index must be integer expression"));
                    }
                    Term::Indexed {
                        pc: at,
                        tag,
                        base,
                        stride: 4,
                        indices: vec![expression],
                    }
                }
                0x107 | 0x10b | 0x111 | 0x113 if kind != 0x201 => {
                    let base = self.dword()?;
                    let stride = self.word()?;
                    let mut indices = vec![self.expression(depth + 1)?];
                    if matches!(tag, 0x111 | 0x113) {
                        indices.push(self.expression(depth + 1)?);
                    }
                    if indices.iter().any(|e| e.kind != 0x200) {
                        return Err(self.error(at, Some(tag), "index must be integer expression"));
                    }
                    Term::Indexed {
                        pc: at,
                        tag,
                        base,
                        stride,
                        indices,
                    }
                }
                0x160..=0x17e if kind != 0x201 || tag <= 0x172 => Term::Operator {
                    pc: at,
                    opcode: tag,
                },
                0x120 | 0x122 | 0x125 | 0x127 if kind == 0x201 => Term::Scalar {
                    pc: at,
                    tag,
                    raw: self.dword()?,
                },
                0x100 | 0x103 | 0x104 | 0x106 | 0x108 | 0x10a | 0x10e | 0x10f | 0x129 | 0x12a
                | 0x12c | 0x12e
                    if kind != 0x201 =>
                {
                    Term::Scalar {
                        pc: at,
                        tag,
                        raw: self.dword()?,
                    }
                }
                _ => return Err(self.error(at, Some(tag), "unsupported expression token")),
            };
            terms.push(term);
        }
    }
}

pub fn decode_at(code: &[u8], pc: usize) -> Result<Instruction, DecodeError> {
    // Bound the public offset before cursor arithmetic, including usize::MAX input.
    if pc >= code.len() {
        return Err(DecodeError {
            pc,
            opcode: None,
            reason: "PC outside code",
        });
    }
    let mut reader = Reader { code, pos: pc };
    let opcode = reader.word()?;
    let kind = match opcode {
        0x200..=0x202 => {
            reader.pos = pc;
            InstructionKind::Expression(reader.expression(0)?)
        }
        0x400 => InstructionKind::Jump {
            target: reader.dword()?,
        },
        0x401 | 0x402 => InstructionKind::ConditionalJump {
            target: reader.dword()?,
        },
        0x403 => InstructionKind::CompareJump {
            value: reader.dword()?,
            target: reader.dword()?,
        },
        0x407 => InstructionKind::LongJump {
            reserved: reader.dword()?,
            target: reader.dword()?,
        },
        0x405 | 0x410 => InstructionKind::Label {
            index: reader.word()?,
        },
        0x411 | 0x412 => InstructionKind::StackAdjust {
            bytes: reader.word()?,
        },
        0x413 | 0x414 | 0x416 | 0x430 => InstructionKind::Simple,
        0x440 | 0x442 => InstructionKind::StackImmediate {
            bytes: reader.word()?,
            raw: reader.dword()?,
        },
        0x2000..=0x27ff => InstructionKind::System {
            command: opcode & 0x7ff,
        },
        _ => return Err(reader.error(pc, Some(opcode), "unsupported instruction")),
    };
    Ok(Instruction {
        pc,
        end: reader.pos,
        opcode,
        kind,
    })
}

/// Decode every byte linearly, then check known entry/label/branch targets.
/// This does not establish reachability, stack correctness, or runtime compatibility.
pub fn decode_layout(
    code: &[u8],
    entry: u32,
    labels: &[u32],
) -> Result<Vec<Instruction>, DecodeError> {
    let mut instructions = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let instruction = decode_at(code, pc)?;
        pc = instruction.end;
        instructions.push(instruction);
    }
    let starts: std::collections::BTreeSet<usize> = instructions.iter().map(|i| i.pc).collect();
    let check = |source, op, target: u32| {
        if starts.contains(&(target as usize)) {
            Ok(())
        } else {
            Err(DecodeError {
                pc: source,
                opcode: op,
                reason: "target is not an instruction boundary",
            })
        }
    };
    check(entry as usize, None, entry)?;
    for &label in labels {
        check(label as usize, None, label)?;
    }
    for instruction in &instructions {
        match instruction.kind {
            InstructionKind::Jump { target }
            | InstructionKind::ConditionalJump { target }
            | InstructionKind::CompareJump { target, .. }
            | InstructionKind::LongJump { target, .. } => {
                check(instruction.pc, Some(instruction.opcode), target)?;
            }
            InstructionKind::Label { index } if index as usize >= labels.len() => {
                return Err(DecodeError {
                    pc: instruction.pc,
                    opcode: Some(instruction.opcode),
                    reason: "label index outside table",
                });
            }
            _ => {}
        }
    }
    Ok(instructions)
}

/// Evaluate only integer expressions consisting of constants and pure operators.
/// References and mutation explicitly fail: there is no guessed variable state.
pub fn eval_constant(expression: &Expression) -> Result<i32, DecodeError> {
    let error = |pc, opcode, reason| DecodeError {
        pc,
        opcode: Some(opcode),
        reason,
    };
    if expression.kind != 0x200 {
        return Err(error(
            expression.pc,
            expression.kind,
            "not an integer expression",
        ));
    }
    let mut values: Vec<i32> = Vec::new();
    for term in &expression.terms {
        match term {
            Term::Scalar {
                tag: 0x100, raw, ..
            } => values.push(*raw as i32),
            Term::Nested {
                tag: 0x200,
                expression,
                ..
            } => values.push(eval_constant(expression)?),
            Term::Operator { pc, opcode } if matches!(*opcode, 0x160..=0x16f | 0x171 | 0x172) => {
                let right = values
                    .pop()
                    .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                let left = values
                    .pop()
                    .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                let result = crate::evaluate::binary_value(*pc, *opcode, left, right)?;
                values.push(result);
            }
            Term::Scalar { pc, tag, .. }
            | Term::Nested { pc, tag, .. }
            | Term::Indexed { pc, tag, .. } => {
                return Err(error(
                    *pc,
                    *tag,
                    "operand needs VM state or another value type",
                ));
            }
            Term::Operator { pc, opcode } => {
                return Err(error(*pc, *opcode, "operator needs mutable VM state"));
            }
        }
    }
    if values.len() != 1 {
        return Err(error(
            expression.pc,
            expression.kind,
            "expression does not produce exactly one value",
        ));
    }
    Ok(values[0])
}

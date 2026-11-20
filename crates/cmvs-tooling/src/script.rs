//! Source string boundaries and bytecode references shared by table export and relocation.
use crate::Result;
use cmvs_core::{
    format::ps3::Script,
    vm::bytecode::{self, Instruction, InstructionKind, Term},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Segment {
    Text,
    Data,
}

pub type Key = (Segment, usize);

pub struct ScriptText<'a> {
    script: &'a Script,
    // Includes every text-table start and every referenced suffix or initial-data string.
    strings: BTreeMap<Key, &'a [u8]>,
    references: Vec<(usize, Key)>,
    // Literal operands the VM actually copies into each expression's accumulator.
    accumulators: Vec<(usize, Vec<Key>)>,
}

impl<'a> ScriptText<'a> {
    pub fn new(script: &'a Script) -> Result<Self> {
        let listing =
            bytecode::decode_layout(script.code(), script.header().entry_pc, &script.labels())?;
        Self::from_layout(script, &listing)
    }

    /// Reuse the decoded instruction layout when the caller also validates operands.
    pub fn from_layout(script: &'a Script, listing: &[Instruction]) -> Result<Self> {
        let mut strings = BTreeMap::new();
        let mut start = 0;
        for entry in script.text().split_inclusive(|&b| b == 0) {
            let value = entry.strip_suffix(&[0]).ok_or("unterminated text string")?;
            strings.insert((Segment::Text, start), value);
            start += entry.len();
        }
        let mut references = Vec::new();
        let mut accumulators = Vec::new();
        for ins in listing {
            let InstructionKind::Expression(expr) = &ins.kind else {
                continue;
            };
            // String expressions are top-level. Their nested slot indices are integer expressions.
            if expr.kind != 0x201 {
                continue;
            }
            let mut stack = Vec::new();
            let mut copies = Vec::new();
            for term in &expr.terms {
                match term {
                    Term::Scalar {
                        pc,
                        tag: tag @ (0x120 | 0x125),
                        raw,
                    } => {
                        let segment = if *tag == 0x120 {
                            Segment::Text
                        } else {
                            Segment::Data
                        };
                        let offset = *raw as usize;
                        let key = (segment, offset);
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            strings.entry(key)
                        {
                            let bytes = match segment {
                                Segment::Text => script.text(),
                                Segment::Data => script.data(),
                            };
                            let value = bytes
                                .get(offset..)
                                .and_then(|tail| {
                                    tail.iter().position(|&b| b == 0).map(|end| &tail[..end])
                                })
                                .ok_or_else(|| {
                                    format!(
                                        "code+{pc:#x}: invalid {segment:?} string at {offset:#x}"
                                    )
                                })?;
                            entry.insert(value);
                        }
                        references.push((*pc, key));
                        stack.push((segment == Segment::Text).then_some(key));
                    }
                    Term::Operator { pc, opcode } => {
                        let top = stack
                            .pop()
                            .ok_or_else(|| format!("string stack underflow at {pc:#x}"))?;
                        let second = stack
                            .last_mut()
                            .ok_or_else(|| format!("string stack underflow at {pc:#x}"))?;
                        if *opcode != 0x170 {
                            copies.extend(*second);
                        }
                        copies.extend(top);
                        *second = None;
                    }
                    _ => stack.push(None),
                }
            }
            if !copies.is_empty() {
                accumulators.push((expr.pc, copies));
            }
        }
        Ok(Self {
            script,
            strings,
            references,
            accumulators,
        })
    }

    pub fn source(&self) -> &'a Script {
        self.script
    }
    pub fn entries(&self) -> &BTreeMap<Key, &'a [u8]> {
        &self.strings
    }
    pub fn references(&self) -> &[(usize, Key)] {
        &self.references
    }
    pub fn accumulators(&self) -> &[(usize, Vec<Key>)] {
        &self.accumulators
    }

    pub fn strings(&self) -> impl Iterator<Item = (Segment, usize, &'a [u8])> + '_ {
        self.strings
            .iter()
            .map(|(&(seg, offset), &bytes)| (seg, offset, bytes))
    }
}

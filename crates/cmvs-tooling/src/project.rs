//! Shared conventions for the versioned, symbolic script listing.
use cmvs_core::{
    format::ps3::Container,
    profile::{EngineProfile, Registry},
    vm::bytecode::{Expression, Instruction, InstructionKind as Kind, Term},
};
use std::collections::BTreeSet;

pub const DATA_ROW: usize = 32;
pub const FORMAT: &str = "cmvs-script-v1";

/// Non-word fields in an instruction. Remaining fields are ordinary 16-bit words.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Number,
    Immediate,
    Pc,
    Label16,
    Label32,
    Text,
}

pub fn fields(ins: &Instruction, mut emit: impl FnMut(usize, Field)) {
    fn expression(expr: &Expression, emit: &mut impl FnMut(usize, Field)) {
        for term in &expr.terms {
            match term {
                Term::Scalar { pc, tag, .. } => emit(
                    pc + 2,
                    match tag {
                        0x100 if expr.kind == 0x200 => Field::Immediate,
                        0x10e => Field::Label32,
                        0x120 => Field::Text,
                        _ => Field::Number,
                    },
                ),
                Term::Nested {
                    expression: child, ..
                } => expression(child, emit),
                Term::Indexed { pc, indices, .. } => {
                    emit(pc + 2, Field::Number);
                    for child in indices {
                        expression(child, emit);
                    }
                }
                Term::Operator { .. } => {}
            }
        }
    }
    match &ins.kind {
        Kind::Jump { .. } | Kind::ConditionalJump { .. } => emit(ins.pc + 2, Field::Pc),
        Kind::CompareJump { .. } | Kind::LongJump { .. } => {
            emit(ins.pc + 2, Field::Number);
            emit(ins.pc + 6, Field::Pc);
        }
        Kind::Label { .. } => emit(ins.pc + 2, Field::Label16),
        Kind::StackImmediate { bytes, .. } => emit(
            ins.pc + 4,
            if *bytes == 4 {
                Field::Immediate
            } else {
                Field::Number
            },
        ),
        Kind::Expression(expr) => expression(expr, &mut emit),
        _ => {}
    }
}

pub fn runtime_command(profile: &EngineProfile, command: u16) -> Option<u16> {
    if profile.is_cvns() {
        profile.map(command)
    } else {
        Some(command)
    }
}

pub fn possible_callback(container: Container, command: u16) -> bool {
    match container {
        Container::Ps2a => command == 0x88,
        Container::Pss0 => Registry::builtin()
            .profiles()
            .iter()
            .any(|p| p.is_cvns() && p.map(command) == Some(0x88)),
    }
}

pub enum CallbackTarget {
    Literal { field: usize, value: u32 },
    Label,
    Dynamic,
}

/// The argument at SP-8 is the penultimate pushed word. Only infer a producer
/// along one straight-line path; a branch into the argument setup makes it ambiguous.
pub fn callback_target(
    code: &[Instruction],
    call: usize,
    entries: &BTreeSet<usize>,
) -> CallbackTarget {
    fn push_start(code: &[Instruction], end: usize) -> Option<usize> {
        let last = end.checked_sub(1)?;
        match &code[last].kind {
            Kind::StackImmediate { bytes: 4, .. } => Some(last),
            Kind::Simple if code[last].opcode == 0x430 => {
                let before = last.checked_sub(1)?;
                matches!(code[before].kind, Kind::Expression(_)).then_some(before)
            }
            _ => None,
        }
    }
    let Some(start) = push_start(code, call).and_then(|slot| push_start(code, slot)) else {
        return CallbackTarget::Dynamic;
    };
    if entries
        .range(code[start].end..=code[call].pc)
        .next()
        .is_some()
    {
        return CallbackTarget::Dynamic;
    }
    match &code[start].kind {
        Kind::StackImmediate { raw, .. } => CallbackTarget::Literal {
            field: code[start].pc + 4,
            value: *raw,
        },
        Kind::Expression(expr) if expr.kind == 0x200 => match expr.terms.as_slice() {
            [
                Term::Scalar {
                    tag: 0x100,
                    pc,
                    raw,
                },
            ] => CallbackTarget::Literal {
                field: pc + 2,
                value: *raw,
            },
            [Term::Scalar { tag: 0x10e, .. }] => CallbackTarget::Label,
            _ => CallbackTarget::Dynamic,
        },
        _ => CallbackTarget::Dynamic,
    }
}

pub fn entry_points(
    code: &[Instruction],
    entry: u32,
    labels: &[u32],
    profile: Option<&EngineProfile>,
    resolve_literal: impl Fn(usize, u32) -> Option<u32>,
) -> BTreeSet<usize> {
    let mut entries: BTreeSet<_> = labels.iter().map(|&pc| pc as usize).collect();
    entries.insert(entry as usize);
    for ins in code {
        if let Kind::Jump { target }
        | Kind::ConditionalJump { target }
        | Kind::CompareJump { target, .. }
        | Kind::LongJump { target, .. } = ins.kind
        {
            entries.insert(target as usize);
        }
    }
    // An invoked callback can also enter another registration's argument setup.
    if let Some(profile) = profile {
        let mut targets = Vec::new();
        for (i, ins) in code.iter().enumerate() {
            if let Kind::System { command } = ins.kind
                && runtime_command(profile, command) == Some(0x88)
                && let CallbackTarget::Literal { field, value } = callback_target(code, i, &entries)
                && value != u32::MAX
                && let Some(target) = resolve_literal(field, value)
            {
                targets.push(target as usize);
            }
        }
        entries.extend(targets);
    }
    entries
}

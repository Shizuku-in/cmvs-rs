use crate::bytecode::{Instruction, InstructionKind, Term, eval_constant};

// Recognize `if (integer != 0) call(argument)` at decoded instruction boundaries.
// Conflicting bindings are left to the game's original behavior.
pub(super) fn conditional_integer<'a>(
    mut instructions: impl Iterator<Item = &'a Instruction>,
    argument: i32,
    call_opcode: u16,
) -> Option<u32> {
    let mut previous = [
        instructions.next()?,
        instructions.next()?,
        instructions.next()?,
        instructions.next()?,
    ];
    let mut found = None;
    for call in instructions {
        let [guard, branch, value, push] = previous;
        previous = [branch, value, push, call];
        let contiguous = guard.end == branch.pc
            && branch.end == value.pc
            && value.end == push.pc
            && push.end == call.pc;
        let InstructionKind::Expression(guard) = &guard.kind else {
            continue;
        };
        let InstructionKind::ConditionalJump { target } = &branch.kind else {
            continue;
        };
        let InstructionKind::Expression(value) = &value.kind else {
            continue;
        };
        if !contiguous
            || *target < call.end as u32
            || guard.kind != 0x200
            || value.kind != 0x200
            || push.opcode != 0x430
            || call.opcode != call_opcode
            || eval_constant(value).ok() != Some(argument)
        {
            continue;
        }
        let [
            Term::Nested {
                tag: 0x101,
                expression,
                ..
            },
            Term::Scalar {
                tag: 0x100, raw: 0, ..
            },
            Term::Operator { opcode: 0x172, .. },
        ] = guard.terms.as_slice()
        else {
            continue;
        };
        let Some(integer) = eval_constant(expression)
            .ok()
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        if found.replace(integer).is_some_and(|old| old != integer) {
            return None;
        }
    }
    found
}

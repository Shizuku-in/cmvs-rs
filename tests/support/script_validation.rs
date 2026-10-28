//! Script structure assertions shared by synthetic tests and opt-in external checks.
use cmvs_rs::{
    bytecode::{self, DecodeError, Instruction, InstructionKind},
    ps3::Script,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct ScriptStructure {
    pub code_bytes: usize,
    pub label_count: u32,
    pub instructions: Vec<Instruction>,
    /// Full system opcodes, including 0x2000. Counts are static occurrences.
    pub system_commands: BTreeMap<u16, usize>,
}

/// Checks entry, labels and direct jump targets against decoded instruction boundaries.
/// Unknown runtime system semantics are not validated; this never executes a script.
pub fn validate(script: &Script) -> Result<ScriptStructure, DecodeError> {
    let instructions =
        bytecode::decode_layout(script.code(), script.header.entry_pc, &script.labels())?;
    let mut system_commands = BTreeMap::new();
    for instruction in &instructions {
        if let InstructionKind::System { command } = instruction.kind {
            *system_commands.entry(command + 0x2000).or_default() += 1;
        }
    }
    Ok(ScriptStructure {
        code_bytes: script.code().len(),
        label_count: script.header.label_count,
        instructions,
        system_commands,
    })
}

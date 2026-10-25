use cmvs_rs::{
    MAX_PAYLOAD,
    bytecode::{self, InstructionKind},
    ps3::Script,
};
use std::{collections::BTreeMap, env, error::Error, fs::File, io::Read};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args.next().ok_or("usage: cmvs-disasm <script>")?;
    if path == "--help" {
        println!(
            "Usage: cmvs-disasm <script>\nLinear structural decoding; does not execute or follow branches."
        );
        return Ok(());
    }
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let mut raw = Vec::new();
    File::open(&path)?
        .take(MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_PAYLOAD + 48 {
        return Err("input exceeds research allocation limit".into());
    }
    let script = Script::decode(&raw)?;
    let mut count = 0;
    let mut commands = BTreeMap::new();
    println!("# Linear decode: {:?}; not execution", path);
    let instructions =
        bytecode::decode_layout(script.code(), script.header.entry_pc, &script.labels())?;
    for insn in instructions {
        let pc = insn.pc;
        match &insn.kind {
            InstructionKind::Expression(expr) => {
                println!(
                    "{pc:04x}..{:04x}: expr 0x{:04x} {:?} constant={:?}",
                    insn.end,
                    expr.kind,
                    expr.terms,
                    bytecode::eval_constant(expr).ok()
                );
            }
            InstructionKind::System { command } => {
                *commands.entry(*command).or_insert(0usize) += 1;
                println!("{pc:04x}..{:04x}: system 0x{:04x}", insn.end, insn.opcode);
            }
            other => println!(
                "{pc:04x}..{:04x}: 0x{:04x} {other:?}",
                insn.end, insn.opcode
            ),
        }
        count += 1;
    }
    let pc = script.code().len();
    println!(
        "# Complete linear decode: {count} instructions, {pc} code bytes, {} system command IDs",
        commands.len()
    );
    for (command, uses) in commands {
        println!("# system 0x{:04x}: {uses}", command + 0x2000);
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

//! Script disassembly, project export, and translation table export.
mod project;
use cmvs_core::{
    format::ps3::Script,
    vm::bytecode::{self, InstructionKind},
};
use cmvs_tooling::{
    Result,
    cli::{Arguments, wants_help},
    script::{ScriptText, Segment},
    table::{Encoding, FORMAT, escape, fingerprint},
    write_new,
};
pub use project::export_project;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{BufWriter, Write},
    path::Path,
};

const USAGE: &str = "\
Usage: cmvs-disassembler <script> [--profile <id|game>]
       cmvs-disassembler <script> --export <new.project> [--profile <id|game>] [--source-encoding <encoding>]
       cmvs-disassembler <script> --export-text <new.tsv> --source-encoding <shift-jis|gbk|utf-8>

Print linear PS2A/PSS0 instructions and command counts, or export a translation table.
Add a fourth TSV column for edited rows. Import with cmvs-assembler script import-text.
Assemble an edited project with cmvs-assembler script assemble.";

pub fn run(args: &[OsString]) -> Result<()> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let args = Arguments::parse(
        args,
        &[
            "--profile",
            "--export",
            "--export-text",
            "--source-encoding",
        ],
        &[],
    )?;
    let [path] = args.paths.as_slice() else {
        return Err("expected one script path".into());
    };
    let script = args.read_script(Path::new(path))?;
    if let Some(out) = args.option("--export") {
        if args.option("--export-text").is_some() {
            return Err("choose one export format".into());
        }
        let encoding = if args.option("--source-encoding").is_some() {
            args.encoding("--source-encoding")?
        } else {
            Encoding::ShiftJis
        };
        return write_new(Path::new(out), |file| {
            export_project(&script, args.profile()?, encoding, BufWriter::new(file))
        });
    }
    if let Some(out) = args.option("--export-text") {
        let encoding = args.encoding("--source-encoding")?;
        return write_new(Path::new(out), |file| {
            export_text(&script, encoding, BufWriter::new(file))
        });
    }
    if args.option("--source-encoding").is_some() {
        return Err("--source-encoding requires --export or --export-text".into());
    }
    let mut count = 0;
    let mut commands = BTreeMap::new();
    println!("# Linear decode: {path:?}; not execution");
    let instructions =
        bytecode::decode_layout(script.code(), script.header().entry_pc, &script.labels())?;
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

pub fn export_text(script: &Script, source: Encoding, mut out: impl Write) -> Result<()> {
    let layout = ScriptText::new(script)?;
    writeln!(out, "{FORMAT}\t{}\t{}", source.name(), fingerprint(script)?)?;
    for (segment, offset, bytes) in layout.strings() {
        let name = match segment {
            Segment::Text => "text",
            Segment::Data => "data",
        };
        writeln!(out, "{name}\t{offset:x}\t{}", escape(bytes, source))?;
    }
    out.flush()?;
    Ok(())
}

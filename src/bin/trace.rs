use cmvs_rs::{
    MAX_PAYLOAD,
    bootstrap::Bootstrap,
    cpz::Cpz6Index,
    machine::{Machine, Step},
    ps3::Script,
};
use std::{env, error::Error, fs::File, io::Read};

fn number(raw: &std::ffi::OsStr) -> Result<u32, Box<dyn Error>> {
    let text = raw.to_str().ok_or("expected numeric argument")?;
    Ok(if let Some(hex) = text.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)?
    } else {
        text.parse()?
    })
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: cmvs-trace <script-or-archive> [--script <ASCII-basename>] [--entry <pc>] [--steps <budget>] [--bootstrap]")?;
    if path == "--help" {
        println!(
            "Usage: cmvs-trace <script-or-archive> [--script <ASCII-basename>] [--entry <pc>] [--steps <budget>] [--bootstrap]\n--script loads a script directly from CPZ6 mode 1. Bounded research VM; stops at unimplemented host commands. --bootstrap applies verified startup state/configuration. Isolated scripts do not inherit game startup state."
        );
        return Ok(());
    }
    let mut entry = None;
    let mut budget = 10000u32;
    let mut bootstrap = None;
    let mut archive_script = None;
    while let Some(flag) = args.next() {
        if flag == "--bootstrap" {
            bootstrap = Some(Bootstrap::default());
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        if flag == "--entry" {
            entry = Some(number(&value)?);
        } else if flag == "--steps" {
            budget = number(&value)?;
        } else if flag == "--script" {
            archive_script = Some(value);
        } else {
            return Err("unknown option".into());
        }
    }
    if budget == 0 || budget > 1000000 {
        return Err("step budget must be 1..1000000".into());
    }
    let mut file = File::open(&path)?;
    let raw = if let Some(name) = archive_script {
        let name = name
            .to_str()
            .filter(|s| s.is_ascii())
            .ok_or("archive script basename must be ASCII")?;
        let index = Cpz6Index::read(&mut file)?;
        index.read_entry(&mut file, index.find(name.as_bytes())?)?
    } else {
        let mut raw = Vec::new();
        file.take(MAX_PAYLOAD as u64 + 49).read_to_end(&mut raw)?;
        raw
    };
    if raw.len() > MAX_PAYLOAD + 48 {
        return Err("input exceeds research allocation limit".into());
    }
    let script = Script::decode(&raw)?;
    let mut machine = Machine::at(&script, entry.unwrap_or(script.header.entry_pc))?;
    println!(
        "# Research trace {:?}; initial variables unknown; entry=0x{:x}",
        path,
        machine.pc()
    );
    for step in 0..budget {
        let pc = machine.pc();
        let event = machine.step()?;
        println!(
            "{step}: pc=0x{pc:04x} -> 0x{:04x} result={:?} stack_bytes={} {event:?}",
            machine.pc(),
            machine.state().result,
            machine.stack().len_bytes()
        );
        if let Step::System { command, .. } = event {
            if let Some(host) = &mut bootstrap {
                if let Some(event) = host.try_handle(&mut machine)? {
                    println!(
                        "# bootstrap applied 0x{:04x} -> pc=0x{:04x} {event:?}",
                        command + 0x2000,
                        machine.pc()
                    );
                    continue;
                }
                println!(
                    "# bootstrap handled={} registered_paths={}",
                    host.handled_count(),
                    host.paths().len()
                );
                for ((op, slot), path) in host.paths() {
                    println!("# path 0x{op:04x}[{slot}]={}", path.escape_ascii());
                }
            }
            println!(
                "# STOP pending system=0x{:04x}; PC and parameters preserved; not game completion",
                command + 0x2000
            );
            if machine.stack().len_bytes() >= 4 {
                println!("# top_word=0x{:08x}", machine.stack().peek_u32(4)?);
            }
            println!(
                "# known integers={} known bits={}",
                machine.state().integers().len(),
                machine.state().bits().len()
            );
            return Ok(());
        }
    }
    Err("research step budget exhausted".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

//! Dispatch inventory: feeds every system command through the runner's full dispatch chain
//! with synthetic arguments and records whether any host recognizes it. The output is joined
//! with static script sites by `tools/command_inventory.py`; see the Amatsutsumi script audit.
#[path = "support/bytecode.rs"]
#[allow(dead_code)]
mod commands;
use cmvs_rs::{
    platform::RecordingBackend,
    profile::Registry,
    runner::{Startup, StartupEvent, load_start},
};
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

/// All 2048 cases count as base semantics so the gate never hides a host.
fn synthetic_profile() -> String {
    let mut text = format!(
        "profile synthetic-base\nfamily cmvs\nbase -\nbinary x64 file=cmvs64.exe sha256={} \
         timestamp=0x00000000 linker=10.0 image-base=0x140000000 table=0x1000 default=0x2000 \
         cases=2048\n",
        "0".repeat(64)
    );
    for command in 0..2048 {
        text.push_str(&format!("0x{:04x} base 0x3000\n", 0x2000 + command));
    }
    text
}

fn write_root(root: &Path, code: &[u8]) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("data/pack")).unwrap();
    fs::create_dir_all(root.join("save")).unwrap();
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        0,
        0,
        0,
        code.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(root.join("data/pack/start.ps3"), raw).unwrap();
    let bytes: Vec<_> = include_str!("fixtures/cpz6-script.hex")
        .trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    fs::write(root.join("data/pack/script.cpz"), bytes).unwrap();
}

/// `handled`: the command completed or waits; `error`: a host recognized it and rejected the
/// synthetic state; `unhandled`: no host claimed it. Errors keep the host's message.
fn probe(registry: &Registry, root: &Path, opcode: u16, args: &[u32]) -> String {
    let mut code = Vec::new();
    commands::command(&mut code, opcode, args);
    let site = code.len() - 2;
    code.extend(0x2000u16.to_le_bytes());
    write_root(root, &code);
    let game = registry
        .identify_game(root, Some("synthetic-base"))
        .unwrap();
    let script = load_start(root, &game.files).unwrap();
    let mut run = match Startup::new(&script, root, &game, 0) {
        Ok(run) => run,
        Err(e) => return format!("setup\t{e}"),
    };
    let mut backend = RecordingBackend::default();
    for _ in 0..256 {
        match run.tick(&mut backend) {
            Err(e) if run.machine().pc() <= site => {
                return format!("error\t{}", e.to_string().replace(['\t', '\n'], " "));
            }
            Err(_) => return "handled\tlater error".into(),
            Ok(StartupEvent::Unhandled { command, .. }) if command == opcode => {
                return "unhandled\t".into();
            }
            Ok(StartupEvent::Unhandled { .. }) => return "handled\tlater unhandled".into(),
            Ok(_) if run.machine().pc() > site && run.machine().pending_system().is_none() => {
                return "handled\t".into();
            }
            Ok(_) => {}
        }
    }
    "waits\t".into()
}

fn inventory(root: &Path, opcodes: impl Iterator<Item = u16>) -> String {
    let registry = Registry::parse(&[&synthetic_profile()], "").unwrap();
    let variants: [(&str, [u32; 8]); 3] = [
        ("zero", [0; 8]),
        ("one", [1; 8]),
        ("two", [2, 1, 1, 1, 1, 1, 1, 1]),
    ];
    let mut text = String::new();
    for opcode in opcodes {
        for (name, args) in &variants {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                probe(&registry, root, opcode, args)
            }))
            .unwrap_or_else(|_| "panic\t".into());
            writeln!(text, "0x{opcode:04x}\t{name}\t{result}").unwrap();
        }
    }
    let _ = fs::remove_dir_all(root);
    text
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cmvs-inventory-{}-{name}", std::process::id()))
}

#[test]
fn probe_separates_dispatched_and_unclaimed_commands() {
    let text = inventory(
        &temp_root("synthetic"),
        [0x20d0, 0x2022, 0x27fe].into_iter(),
    );
    // 0x20d0 is a pure math command; 0x2022 needs an object; no host claims 0x27fe.
    assert!(text.contains("0x20d0\tzero\thandled\t"));
    assert!(text.contains("0x2022\tzero\terror\t"));
    assert!(text.contains("0x27fe\tzero\tunhandled\t"));
}

#[test]
#[ignore = "research inventory; writes CMVS_COMMAND_INVENTORY"]
fn command_inventory() {
    let output = std::env::var("CMVS_COMMAND_INVENTORY")
        .expect("set CMVS_COMMAND_INVENTORY to a new output path");
    let text = inventory(&temp_root("all"), 0x2000u16..0x2400);
    fs::File::create_new(&output)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
        .unwrap();
}

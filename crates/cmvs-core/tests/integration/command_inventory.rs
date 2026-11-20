//! Dispatch inventory: feeds every system command through the runner's full dispatch chain
//! with synthetic arguments and records whether any host recognizes it.
use crate::support::{bytecode as commands, profile::synthetic_profile};
use cmvs_core::{
    host::platform::RecordingBackend, profile::Registry, runtime::Runtime, runtime::RuntimeEvent,
    runtime::load_start,
};
use std::{fmt::Write as _, fs, path::Path};

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
    let bytes: Vec<_> = include_str!("../fixtures/cpz6-script.hex")
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
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
    let mut run = match Runtime::new(&script, root, &game, 0) {
        Ok(run) => run,
        Err(e) => return format!("setup\t{e}"),
    };
    // Text services are opt-in for recording hosts; leaving them disabled hides
    // implemented message/choice commands from this inventory.
    run.set_record_text(true);
    if opcode == 0x2129 {
        run.set_user_data_directory(&root.with_extension("user"))
            .unwrap();
    }
    let mut backend = RecordingBackend::default();
    for _ in 0..256 {
        match run.tick(&mut backend) {
            Err(e) if run.machine().pc() <= site => {
                return format!("error\t{}", e.to_string().replace(['\t', '\n'], " "));
            }
            Err(_) => return "handled\tlater error".into(),
            Ok(RuntimeEvent::Unhandled { command, .. }) if command == opcode => {
                return "unhandled\t".into();
            }
            Ok(RuntimeEvent::Unhandled { .. }) => return "handled\tlater unhandled".into(),
            Ok(_) if run.machine().pc() > site && run.machine().pending_system().is_none() => {
                return "handled\t".into();
            }
            Ok(_) => {}
        }
    }
    "waits\t".into()
}

fn inventory(root: &Path, opcodes: impl Iterator<Item = u16>) -> String {
    let registry = Registry::new(vec![synthetic_profile("synthetic-base")], vec![]);
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
    let _ = fs::remove_dir_all(root.with_extension("user"));
    text
}

fn temp_root(name: &str) -> crate::support::temp_dir::TempPath {
    crate::support::temp_dir::TempPath::new(&format!("inventory-{name}"))
}

#[test]
fn probe_separates_dispatched_and_unclaimed_commands() {
    let text = inventory(
        &temp_root("synthetic"),
        [
            0x20d0, 0x2022, 0x210b, 0x2129, 0x2154, 0x2157, 0x2158, 0x238b, 0x27fe,
        ]
        .into_iter(),
    );
    // 0x20d0 is a pure math command; 0x2022 needs an object; no host claims 0x27fe.
    assert!(text.contains("0x20d0\tzero\thandled\t"));
    assert!(text.contains("0x2022\tzero\terror\t"));
    assert!(text.contains("0x27fe\tzero\tunhandled\t"));
    for op in [0x210b, 0x2129, 0x2154, 0x2157, 0x2158, 0x238b] {
        assert!(
            text.lines()
                .filter(|line| line.starts_with(&format!("0x{op:04x}\t")))
                .all(|line| line.contains("\thandled\t") || line.contains("\terror\t")),
            "implemented service 0x{op:04x} was hidden"
        );
    }
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

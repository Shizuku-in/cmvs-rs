use crate::support::TempDir;
use cmvs_core::{format::cpz::Cpz5Index, format::ps3::Script};
use std::{fs, io::Cursor, process::Command};

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn extract_cli_never_overwrites_an_existing_file() {
    let root = TempDir::new("cpz6-extract");
    let input = root.join("synthetic.bin");
    let output = root.join("existing.bin");
    fs::write(
        &input,
        hex(include_str!("../../../cmvs-core/tests/fixtures/cpz6-0.hex")),
    )
    .unwrap();
    fs::write(&output, b"keep me").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
        .arg("--extract")
        .arg(&input)
        .arg("fixture.bin")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&output).unwrap(), b"keep me");
}

#[test]
fn trace_cli_loads_and_steps_synthetic_ps3_directly_from_archive() {
    let root = TempDir::new("cpz6-script");
    let path = root.join("script.bin");
    fs::write(
        &path,
        hex(include_str!(
            "../../../cmvs-core/tests/fixtures/cpz6-script.hex"
        )),
    )
    .unwrap();
    let trace = |profile| {
        let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
            .arg(&path)
            .args(["--script", "fixture.ps3", "--profile", profile])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let stdout = trace("cmvs-20160201");
    assert!(stdout.contains("pending system=0x201e"));
    assert!(stdout.contains("top_word=0x00000007"));
    assert_eq!(trace("amatsutsumi"), stdout);
}

#[test]
fn cli_lists_extracts_and_rejects_inapplicable_settings() {
    let root = TempDir::new("cpz3-cli");
    let archive = root.join("fixture.cpz");
    let output = root.join("payload.bin");
    fs::write(
        &archive,
        hex(include_str!("../../../cmvs-core/tests/fixtures/cpz3-0.hex")),
    )
    .unwrap();
    // CPZ3 must not try to load adjacent startup settings.
    fs::write(root.join("start.ps2"), b"invalid").unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&[archive.as_os_str()]).status.success());
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let args = [
        "--extract".as_ref(),
        archive.as_os_str(),
        "fixture.bin".as_ref(),
        output.as_os_str(),
    ];
    assert!(run(&args).status.success());
    assert_eq!(fs::read(&output).unwrap(), b"synthetic CPZ3 payload");
    assert!(!run(&args).status.success());
    for option in [["--keys", "0,0,0,0"], ["--packing-mode", "5"]] {
        assert!(
            !run(&[
                option[0].as_ref(),
                option[1].as_ref(),
                "--list".as_ref(),
                archive.as_os_str()
            ])
            .status
            .success()
        );
    }
}

#[test]
fn plain_resources_and_startup_mode_selection_work_through_cli() {
    let mut reader = Cursor::new(hex(include_str!(
        "../../../cmvs-core/tests/fixtures/cpz5-plain.hex"
    )));
    let index = Cpz5Index::read(&mut reader).unwrap();
    assert_eq!(index.layout.encryption_mode, 0);
    assert_eq!(
        index.read_entry_verified(&mut reader, 0).unwrap(),
        b"synthetic CPZ5 payload 0"
    );
    let root = TempDir::new("cpz5-cli");
    let archive = root.join("fixture.cpz");
    let start = root.join("start.ps2");
    fs::write(
        &archive,
        hex(include_str!("../../../cmvs-core/tests/fixtures/cpz5-5.hex")),
    )
    .unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let mut raw = b"PS2A".to_vec();
    for field in [48u32, 0, 0, 0, 14, 0, 0, 0, 0, 14, 0] {
        raw.extend(field.to_le_bytes());
    }
    let mut code = Vec::new();
    for word in [0x200u16, 0x100] {
        code.extend(word.to_le_bytes());
    }
    code.extend(5u32.to_le_bytes());
    for word in [0x20fu16, 0x430, 0x201f] {
        code.extend(word.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(&start, raw).unwrap();
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    fs::write(&start, b"invalid").unwrap();
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    assert!(
        run(&[
            "--packing-mode".as_ref(),
            "5".as_ref(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
}

fn memory_script(code: Vec<u8>, text: &[u8]) -> Script {
    let mut bytes = b"PS2A".to_vec();
    for field in [
        48,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        bytes.extend(field.to_le_bytes());
    }
    bytes.extend(code);
    bytes.extend(text);
    Script::from_memory_image(&bytes).unwrap()
}

fn push(code: &mut Vec<u8>, string: bool, value: u32) {
    code.extend(if string { 0x201u16 } else { 0x200 }.to_le_bytes());
    code.extend(if string { 0x120u16 } else { 0x100 }.to_le_bytes());
    code.extend(value.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    code.extend(0x430u16.to_le_bytes());
}

fn registration(code: &mut Vec<u8>, keys: [u32; 4]) {
    for key in keys.into_iter().rev() {
        push(code, false, key);
    }
    push(code, true, 1);
    push(code, false, 0);
    code.extend(0x2268u16.to_le_bytes());
}

#[test]
fn cli_discovers_startup_beside_cpz7_and_respects_explicit_overrides() {
    let root = TempDir::new("cpz7-discovery");
    let archive = root.join("script.cpz");
    let start = root.join("start.ps3");
    let explicit_start = root.join("custom.ps3");
    let cpz6 = root.join("cpz6.cpz");
    fs::write(
        &archive,
        hex(include_str!(
            "../../../cmvs-core/tests/fixtures/cpz7-keyed.hex"
        )),
    )
    .unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let mut code = Vec::new();
    push(&mut code, false, 13);
    code.extend(0x201fu16.to_le_bytes());
    registration(&mut code, [0x10203040, 0x89abcdef, 0xfedcba98, 0x76543210]);
    let mut encoded = memory_script(code, b"\0data\\pack\\script.cpz\0")
        .memory_image(&[])
        .unwrap();
    for byte in &mut encoded[48..] {
        *byte = byte.rotate_left(1).wrapping_add(124);
    }
    fs::write(&start, &encoded).unwrap();
    let result = run(&["--list".as_ref(), archive.as_os_str()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("2 files"));
    fs::write(&explicit_start, encoded).unwrap();
    fs::write(&start, b"invalid startup").unwrap();
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    assert!(
        run(&[
            "--start-script".as_ref(),
            explicit_start.as_os_str(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
    assert!(
        run(&[
            "--keys".as_ref(),
            "0x10203040,0x89abcdef,0xfedcba98,0x76543210".as_ref(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
    fs::write(
        &cpz6,
        hex(include_str!("../../../cmvs-core/tests/fixtures/cpz6-0.hex")),
    )
    .unwrap();
    assert!(run(&["--list".as_ref(), cpz6.as_os_str()]).status.success());
}

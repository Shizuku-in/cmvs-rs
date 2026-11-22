use std::{fs, process::Command};

#[test]
fn disassembler_decodes_synthetic_ps2a_and_counts_system_commands() {
    let dir = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_cmvs-disassembler");
    let mut code = Vec::new();
    for value in [42u32, 99] {
        code.extend(0x0440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
        code.extend(0x201eu16.to_le_bytes());
    }
    code.extend(0x2003u16.to_le_bytes());
    let size = code.len() as u32;
    let mut script = b"PS2A".to_vec();
    for word in [48, 0, 0, 0, size, 0, 0, 0, 0, size, 0] {
        script.extend(word.to_le_bytes());
    }
    script.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(dir.path().join("--help"), script).unwrap();

    let run = |args: &[&str]| {
        Command::new(exe)
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let help = run(&["--unknown", "-h"]);
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: cmvs-disassembler"));
    assert!(help.stderr.is_empty());

    let out = run(&["--", "--help"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    for line in [
        "0x0440 StackImmediate { bytes: 4, raw: 42 }",
        "# Complete linear decode: 5 instructions, 22 code bytes, 2 system command IDs",
        "# system 0x2003: 1",
        "# system 0x201e: 2",
    ] {
        assert!(stdout.contains(line), "{stdout}");
    }
    assert!(!run(&["--", "--help", "extra.ps3"]).status.success());
}

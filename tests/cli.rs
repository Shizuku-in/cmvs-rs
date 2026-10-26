use std::{fs, process::Command};

#[test]
fn help_and_unknown_options() {
    let exe = env!("CARGO_BIN_EXE_cmvs-inspect");
    assert!(
        Command::new(exe)
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !Command::new(exe)
            .args(["dummy", "--bad"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn existing_input_is_never_overwritten() {
    let dir = std::env::temp_dir().join(format!("cmvs-cli-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("fixture.ps3");
    // An uncompressed, key-zero script: one 2-byte code segment, no labels or text.
    let mut raw = b"PS2A".to_vec();
    for value in [48u32, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0] {
        raw.extend_from_slice(&value.to_le_bytes());
    }
    raw.extend_from_slice(&[124, 188]);
    fs::write(&input, &raw).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_cmvs-inspect"))
        .arg(&input)
        .arg("--payload")
        .arg(&input)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&input).unwrap(), raw);
    fs::remove_file(&input).unwrap();
    fs::remove_dir(&dir).unwrap();
}

#[test]
fn trace_fade_time_is_frozen_unless_a_frame_delta_is_explicit() {
    let root = std::env::temp_dir().join(format!("cmvs-cli-fade-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let input = root.join("synthetic.ps3");
    let mut code = vec![];
    for value in [9u32, 20, 0, 1] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(0x2220u16.to_le_bytes());
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0u32.to_le_bytes());
    code.extend(0x2224u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let mut raw = b"PS2A".to_vec();
    for value in [
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
        raw.extend(value.to_le_bytes());
    }
    raw.extend(code.iter().map(|v| v.rotate_left(1).wrapping_add(124)));
    fs::write(&input, &raw).unwrap();
    for (extra, needle) in [
        (vec![], "# WAIT fade"),
        (vec!["--frame-ms", "10"], "# STOP pending system=0x27ff"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
            .arg(&input)
            .arg("--record-platform")
            .arg(&root)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(needle));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&input)
        .args(["--frame-ms", "0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&input).unwrap(), raw);
    fs::remove_file(input).unwrap();
    fs::remove_dir(root).unwrap();
}

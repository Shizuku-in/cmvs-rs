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

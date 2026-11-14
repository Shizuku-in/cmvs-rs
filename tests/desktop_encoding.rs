#![cfg(feature = "desktop")]
use std::process::Command;

#[test]
fn font_and_record_text_are_mutually_exclusive() {
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([
            ".",
            "--encoding",
            "gbk",
            "--font",
            "unused.ttf",
            "--record-text",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--font cannot be combined with --record-text")
    );
}

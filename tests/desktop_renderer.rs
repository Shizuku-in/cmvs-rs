#![cfg(feature = "desktop")]
use std::process::Command;

#[test]
fn rejects_unknown_renderer_before_opening_a_window() {
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--renderer", "unknown"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("renderer must be cpu or wgpu"));
}

#[cfg(not(feature = "wgpu-renderer"))]
#[test]
fn cpu_only_build_rejects_gpu_selection_explicitly() {
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--renderer", "wgpu"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("requires the wgpu-renderer build feature")
    );
}

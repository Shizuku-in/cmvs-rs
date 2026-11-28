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

#[test]
fn rejects_invalid_or_cpu_anti_aliasing_before_opening_a_window() {
    let invalid = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--anti-aliasing", "maybe"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("anti-aliasing must be off or on"));

    let cpu = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--anti-aliasing", "on"])
        .output()
        .unwrap();
    assert!(!cpu.status.success());
    assert!(
        String::from_utf8_lossy(&cpu.stderr)
            .contains("--anti-aliasing on requires --renderer wgpu")
    );

    let bad_scale = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--render-scale", "5x"])
        .output()
        .unwrap();
    assert!(!bad_scale.status.success());
    assert!(
        String::from_utf8_lossy(&bad_scale.stderr)
            .contains("render-scale must be 1x, auto, 1.5x, 2x, or 3x")
    );

    let cpu_scale = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .args([".", "--encoding", "shift-jis", "--render-scale", "2x"])
        .output()
        .unwrap();
    assert!(!cpu_scale.status.success());
    assert!(
        String::from_utf8_lossy(&cpu_scale.stderr)
            .contains("--render-scale requires --renderer wgpu")
    );
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

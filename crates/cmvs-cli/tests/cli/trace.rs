use crate::support::{TempDir, bytecode::command};
use cmvs_core::vm::dispatch::Capability;
use std::{fs, process::Command};

#[test]
fn trace_dispatch_respects_optional_capabilities() {
    let root = TempDir::new("shared-state");
    let input = root.join("synthetic.ps3");
    for (opcode, args, log, extra_flag) in [
        (0x2008u16, vec![], Some("# timer 0x2008: values="), None),
        (
            0x22eb,
            vec![1u32, 1],
            Some("# message flag 0x22eb: skip=true auto_advance=false bypass=false;"),
            None,
        ),
        (
            0x20d4,
            vec![10],
            Some("# rand 0x20d4: system_integer[0]=Some(8) (explicit research seed)"),
            Some("--rand-seed"),
        ),
        (0x2020, vec![3], Some("# scene state 0x2020:"), None),
        (0x20a1, vec![], Some("# audio 0x20a1 BGM"), None),
        (0x2226, vec![], Some("# quake 0x2226:"), None),
        (0x22cd, vec![0], Some("# popup menu 0x22cd:"), None),
        (
            0x201e,
            vec![77],
            Some("# bootstrap applied 0x201e"),
            Some("--bootstrap"),
        ),
        (0x2128, vec![], Some("# system save"), Some("--system-save")),
        (
            0x22b3,
            vec![2],
            Some("# save menu applied 0x22b3"),
            Some("--save-dir"),
        ),
        // Recording does not provide text, save metadata or writable storage.
        (0x2100, vec![0, 0, 0], None, None),
        (0x22b9, vec![1], None, None),
        (0x2129, vec![], None, None),
    ] {
        let mut code = vec![];
        for value in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend(opcode.to_le_bytes());
        code.extend(0x2003u16.to_le_bytes());
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
        fs::write(&input, &raw).unwrap();
        for record in [false, true] {
            let options: &[bool] = if extra_flag.is_some() {
                &[false, true]
            } else {
                &[false]
            };
            for &enabled in options {
                let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
                cli.arg(&input).args(["--profile", "cmvs-20160201"]);
                if record {
                    cli.arg("--record-platform").arg(&root);
                }
                if enabled {
                    let flag = extra_flag.unwrap();
                    cli.arg(flag);
                    match flag {
                        "--rand-seed" => {
                            cli.arg("0");
                        }
                        "--save-dir" => {
                            cli.arg(&root);
                        }
                        "--system-save" => {
                            cli.arg(root.join("absent.dat"));
                        }
                        _ => {}
                    }
                }
                let output = cli.output().unwrap();
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(
                    output.status.success(),
                    "{stdout}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let available = if extra_flag.is_some() {
                    enabled
                } else {
                    record && log.is_some()
                };
                if let Some(log) = log {
                    assert_eq!(stdout.contains(log), available, "{stdout}");
                }
                let stop = if available { 0x2003 } else { opcode };
                assert!(
                    stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                    "{stdout}"
                );
            }
        }
        assert_eq!(fs::read(&input).unwrap(), raw);
        assert!(!root.join("absent.dat").exists());
    }
}

#[test]
fn trace_contract_sources_and_legacy_research_policies_are_distinct() {
    let root = TempDir::new("contract");
    let input = root.join("synthetic.ps3");
    let mut code = Vec::new();
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(1u32.to_le_bytes());
    for opcode in [0x2087u16, 0x2008, 0x2009, 0x200e, 0x2003] {
        code.extend(opcode.to_le_bytes());
    }
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
    fs::write(&input, &raw).unwrap();
    for (flags, presumed, unreviewed) in [
        (vec![], true, true),
        (vec!["--block-unreviewed"], true, false),
        (vec!["--block-presumed"], false, false),
        (vec!["--block-presumed", "--block-unreviewed"], false, false),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
            .arg(&input)
            .args(["--profile", "cmvs-20150405", "--record-platform"])
            .arg(&root)
            .args(flags)
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stdout.contains("contract-equivalent=3"),
            "{stdout}\n{stderr}"
        );
        assert!(stdout.contains("# contract-equivalent commands attempted: 0x2008 0x2009 0x2087"));
        assert!(stdout.contains(if presumed { "presumed=1" } else { "presumed=0" }));
        if !presumed {
            assert!(!out.status.success());
            assert!(stderr.contains("legacy presumed"));
        } else if !unreviewed {
            assert!(!out.status.success());
            assert!(stderr.contains("0x2003") && stderr.contains("is unreviewed"));
        } else {
            assert!(stdout.contains("# STOP pending system=0x2003"));
            assert!(stdout.contains("unreviewed=1"));
        }
    }
    assert_eq!(fs::read(&input).unwrap(), raw);
}

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
    let dir = TempDir::new("inspect");
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
}

#[test]
fn trace_fade_time_is_frozen_unless_a_frame_delta_is_explicit() {
    let root = TempDir::new("fade");
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
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&input)
        .args(["--profile", "cmvs-20160201", "--record-platform"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("# WAIT fade"));
    // Once the fade finishes, 0x27ff lies beyond the base build's switch: the gate stops it.
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&input)
        .args(["--profile", "cmvs-20160201", "--record-platform"])
        .arg(&root)
        .args(["--frame-ms", "10"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("# frame delta_ms=10"));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("system command 0x27ff at synthetic.ps3 / 0x")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&input)
        .args(["--profile", "cmvs-20160201", "--frame-ms", "0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&input).unwrap(), raw);
}

#[test]
fn trace_timer_queries_observe_one_delta_after_yield_and_zero_without_frame_clock() {
    let root = TempDir::new("timers");
    let input = root.join("synthetic.ps3");
    let mut code = vec![];
    for query in 0..2 {
        if query == 1 {
            code.extend(0x2000u16.to_le_bytes());
        }
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(0u32.to_le_bytes());
        code.extend(0x200cu16.to_le_bytes());
    }
    code.extend(0x27ffu16.to_le_bytes());
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
    fs::write(&input, &raw).unwrap();
    for (args, expected) in [(vec![], 0), (vec!["--frame-ms", "17"], 17)] {
        let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
            .arg(&input)
            .args(["--profile", "cmvs-20160201", "--record-platform"])
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let queries = stdout
            .lines()
            .filter(|s| s.starts_with("# timer 0x200c:"))
            .collect::<Vec<_>>();
        assert_eq!(queries.len(), 2);
        assert!(queries[0].ends_with("system_integer[0]=Some(0)"));
        assert!(queries[1].ends_with(&format!("system_integer[0]=Some({expected})")));
        // 0x27ff lies beyond the base build's switch, so the profile gate ends the trace.
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("0x27ff at synthetic.ps3 / 0x16 does not exist")
        );
    }
    assert_eq!(fs::read(&input).unwrap(), raw);
}

#[rustfmt::skip]
fn runtime_cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x2001, vec![], Capability::Runtime),
        (0x2003, vec![], Capability::Runtime),
        (0x228c, vec![1, 2], Capability::Runtime),
        (0x228e, vec![1, 2], Capability::Runtime),
        (0x22b5, vec![1000], Capability::Runtime),
        (0x22b6, vec![1000], Capability::Runtime),
        (0x22b8, vec![], Capability::Runtime),
        (0x22be, vec![3, 0x12345678, 0xabcdef01], Capability::Runtime),
        (0x22b7, vec![0, 2], Capability::Runtime),
        (0x22bd, vec![1000, 0, 4294967295, 2147483648, 2147483649], Capability::Runtime),
        (0x22b2, vec![], Capability::Snapshot),
        (0x2153, vec![0, 0, 4294967295], Capability::MessageWait),
        (0x2158, vec![0, 0, 4294967295], Capability::MessageWait),
    ]
}

#[test]
fn trace_has_no_runtime_or_message_context_and_requires_explicit_snapshot_policy() {
    let root = TempDir::new("runtime");
    let path = root.join("runtime.ps3");
    for (op, args, _) in runtime_cases() {
        let mut code = vec![];
        command(&mut code, op, &args);
        command(&mut code, 0x2003, &[]);
        let mut raw = b"PS2A".to_vec();
        for word in [
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
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        fs::write(&path, &raw).unwrap();
        for record in [false, true] {
            for without in [false, true] {
                let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
                cli.arg(&path).args(["--profile", "cmvs-20160201"]);
                if record {
                    cli.arg("--record-platform").arg(&root);
                }
                if without {
                    cli.arg("--continue-without-snapshot");
                }
                let out = cli.output().unwrap();
                let stdout = String::from_utf8(out.stdout).unwrap();
                assert!(
                    out.status.success(),
                    "{stdout}\n{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let stop = if op == 0x22b2 && without { 0x2003 } else { op };
                assert!(
                    stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                    "{stdout}"
                );
            }
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
}

#[test]
fn trace_wait_capabilities_do_not_enable_camera_or_change_waits() {
    let root = TempDir::new("waits");
    let input = root.join("synthetic.ps3");
    for opcode in [0x2224u16, 0x222b, 0x2235, 0x2228, 0x2233, 0x2069, 0x22c7] {
        let mut code = vec![];
        command(
            &mut code,
            opcode,
            if opcode == 0x22c7 { &[0, 0] } else { &[0] },
        );
        command(&mut code, 0x2003, &[]);
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
        fs::write(&input, &raw).unwrap();
        for record in [false, true] {
            let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
            cli.arg(&input)
                .args(["--profile", "cmvs-20160201", "--frame-ms", "17"]);
            if record {
                cli.arg("--record-platform").arg(&root);
            }
            let output = cli.output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let supported = record && !matches!(opcode, 0x2069 | 0x22c7);
            let stop = if supported { 0x2003 } else { opcode };
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                "{stdout}"
            );
            assert!(
                !stdout.contains("# WAIT") && !stdout.contains("# frame delta_ms="),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&input).unwrap(), raw);
    }
}

#[test]
fn cli_step_budget_stops_a_real_control_flow_loop() {
    let dir = TempDir::new("loop-test");
    let path = dir.join("loop.ps3");
    let mut code = 0x400u16.to_le_bytes().to_vec();
    code.extend(0u32.to_le_bytes());
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48u32,
        0,
        0,
        1,
        code.len() as u32,
        0,
        0,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(value.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(&path, raw).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&path)
        .args(["--steps", "3", "--profile", "cmvs-20160201"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("budget exhausted"));
    // Without --profile an unrecorded script is rejected before it runs.
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("matches no recorded game"), "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn trace_reports_the_selected_particle_entry_and_requires_an_explicit_random_source() {
    let root = TempDir::new("particle-trace");
    let path = root.join("synthetic.ps3");
    let arguments = [
        99,
        0,
        0,
        40.0f32.to_bits(),
        20.0f32.to_bits(),
        (-20.0f32).to_bits(),
        2000.0f32.to_bits(),
        1,
    ];
    let mut code = Vec::new();
    command(&mut code, 0x22a5, &arguments);
    command(&mut code, 0x22a0, &[99]);
    command(&mut code, 0x2003, &[]);
    let text = b"particle.pb3\0";
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(&path, &raw).unwrap();
    for seeded in [false, true] {
        let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
        cli.arg(&path)
            .args(["--profile", "cmvs-20171114", "--record-platform"])
            .arg(&root);
        if seeded {
            cli.args(["--rand-seed", "0"]);
        }
        let output = cli.output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(stdout.contains("# particle pools=0"), seeded, "{stdout}");
        assert_eq!(
            stdout.contains("# effect object release 0x22a0:"),
            seeded,
            "{stdout}"
        );
        let stop = if seeded { "0x2003" } else { "0x22a5" };
        assert!(
            stdout.contains(&format!("# STOP pending system={stop};")),
            "{stdout}"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), raw);
}

use cmvs_rs::{
    audio::RecordingAudio,
    dispatch::{self, Capability, Flow, Hosts, MessageWaitHosts, RuntimeOperation, SnapshotHosts},
    input::ActionInput,
    machine::Machine,
    message::{AutoAdvanceContext, MessageHost},
    profile::Semantics,
    snapshot::SaveImage,
};
#[path = "support/runtime_cases.rs"]
mod runtime_cases;
#[allow(dead_code)]
mod support;
use runtime_cases::cases;
use support::bytecode::{command, pending, script};
#[test]
fn runtime_entries_defer_without_completing_and_acknowledge_exactly_once() {
    for (op, args, cap) in cases()
        .into_iter()
        .filter(|(_, _, c)| *c != Capability::MessageWait)
    {
        let mut code = vec![];
        command(&mut code, op, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        let mut image = SaveImage::Absent;
        let request = match dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                runtime: true,
                snapshot: Some(SnapshotHosts {
                    image: &mut image,
                    capture: true,
                    continue_without_image: false,
                }),
                ..Default::default()
            },
        )
        .unwrap()
        {
            Some(Flow::Runtime(request)) => request,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(image, SaveImage::Absent);
        assert_eq!((request.pc(), request.command()), (before.0, op - 0x2000));
        request.validate(&vm).unwrap();
        if cap == Capability::Snapshot {
            assert_eq!(request.operation(), RuntimeOperation::CaptureSnapshot);
        }
        let flags = 0x4000 | ((args.len() * 4) as u16);
        assert_eq!(request.complete(&mut vm, flags).unwrap(), Flow::Done);
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        assert!(request.complete(&mut vm, flags).is_err());
    }
}
#[test]
fn deferred_calls_reject_changed_arguments_and_wrong_cleanup() {
    let mut code = vec![];
    command(&mut code, 0x22b6, &[9]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let request = match dispatch::run(
        Semantics::Root,
        &mut vm,
        &mut Hosts {
            runtime: true,
            ..Default::default()
        },
    )
    .unwrap()
    {
        Some(Flow::Runtime(r)) => r,
        _ => unreachable!(),
    };
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    assert!(request.complete(&mut vm, 0x4008).is_err());
    assert_eq!(
        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
        before
    );
    let mut other = vec![];
    command(&mut other, 0x22b6, &[8]);
    let other = script_code(&other);
    let mut changed = Machine::new(&other).unwrap();
    pending(&mut changed);
    assert_eq!(changed.pending_system(), before.1);
    assert!(request.validate(&changed).is_err());
    assert_eq!(changed.stack().len_bytes(), 4);
}
fn script_code(code: &[u8]) -> cmvs_rs::ps3::Script {
    support::bytecode::script(code)
}
#[test]
fn runtime_short_arguments_preserve_pending_command_and_snapshot() {
    for (op, args, _) in cases()
        .into_iter()
        .filter(|(_, args, cap)| !args.is_empty() && *cap == Capability::Runtime)
    {
        for len in [0, args.len() - 1] {
            let mut code = vec![];
            command(&mut code, op, &args[..len]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let e = dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    runtime: true,
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!((e.pc, e.command), (before.0, op - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
        }
    }
}
#[test]
fn snapshot_policy_selects_capture_barrier_or_explicit_unavailable_image() {
    for capture in [false, true] {
        for without in [false, true] {
            for runtime in [false, true] {
                let script = script(&0x22b2u16.to_le_bytes());
                let mut vm = Machine::new(&script).unwrap();
                pending(&mut vm);
                let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
                let mut image = SaveImage::Absent;
                let flow = dispatch::run(
                    Semantics::Root,
                    &mut vm,
                    &mut Hosts {
                        runtime,
                        snapshot: Some(SnapshotHosts {
                            image: &mut image,
                            capture,
                            continue_without_image: without,
                        }),
                        ..Default::default()
                    },
                )
                .unwrap();
                if capture && runtime {
                    assert!(matches!(flow, Some(Flow::Runtime(_))));
                } else if !capture && without {
                    assert_eq!(flow, Some(Flow::Done));
                    assert!(matches!(image, SaveImage::Unavailable { .. }));
                    assert_eq!(vm.state().bit(0), Some(false));
                } else {
                    assert_eq!(flow, Some(Flow::Unavailable));
                }
                if flow != Some(Flow::Done) {
                    assert_eq!(
                        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                        before
                    );
                    assert_eq!(image, SaveImage::Absent);
                }
            }
        }
    }
}
fn wait_context(audio: &RecordingAudio) -> AutoAdvanceContext<'_> {
    AutoAdvanceContext {
        settings: Default::default(),
        delta_ms: 0,
        gesture_result: false,
        read_eligible: true,
        audio,
    }
}
#[test]
fn message_waits_preserve_pending_and_timed_wait_ignores_unknown_read_word() {
    for op in [0x2153, 0x2158] {
        let mut messages = MessageHost::default();
        let mut input = ActionInput::default();
        let audio = RecordingAudio::default();
        let mut code = vec![];
        command(&mut code, 0x2140, &[0, 0, 0, 0, 1280, 720]);
        // Reserve the ignored word above voice, then push the window slot.
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend(u32::MAX.to_le_bytes());
        command(&mut code, op, &[0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(messages.try_handle(&mut vm).unwrap());
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let result = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                message_wait: Some(MessageWaitHosts {
                    messages: &mut messages,
                    input: &mut input,
                    flags: Default::default(),
                    context: wait_context(&audio),
                }),
                ..Default::default()
            },
        );
        if op == 0x2153 {
            assert_eq!(
                result.unwrap_err().as_decode().unwrap().reason,
                "read identifier"
            );
        } else {
            assert_eq!(result.unwrap(), Some(Flow::Wait));
        }
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
    }
}

#[test]
fn trace_has_no_runtime_or_message_context_and_requires_explicit_snapshot_policy() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-runtime-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("runtime.ps3");
    for (op, args, _) in cases() {
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
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

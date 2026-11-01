use cmvs_rs::{
    camera::CameraHost,
    dispatch::{self, Capability, Flow, Hosts, WaitHosts},
    fade::SkipGate,
    input::ActionInput,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
mod support;
use support::bytecode::{command, pending, script};

fn run(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    camera: &mut CameraHost,
    input: &mut ActionInput,
    gate: SkipGate,
) -> Result<Option<Flow>, cmvs_rs::bytecode::DecodeError> {
    dispatch::run(
        Semantics::Root,
        vm,
        &mut Hosts {
            waits: Some(WaitHosts {
                scene: Some(scene),
                camera: Some(camera),
                input,
                gate,
                change_images: true,
            }),
            ..Default::default()
        },
    )
}

fn object(scene: &mut SceneHost) {
    let mut code = vec![];
    command(&mut code, 0x2020, &[0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    assert!(scene.try_handle(&mut vm).unwrap());
}

#[test]
fn inactive_waits_check_capabilities_then_complete_without_yield_or_frame_advance() {
    let entries: Vec<_> = dispatch::ROOT
        .iter()
        .filter(|entry| dispatch::is_wait(entry.command))
        .collect();
    assert_eq!(entries.len(), 7);
    for entry in entries {
        let owner =
            dispatch::ownership::candidates(entry.command, dispatch::ownership::Chain::Runner);
        assert_eq!(owner.len(), 1);
        assert_eq!(
            owner[0].order_for(entry.command, dispatch::ownership::Chain::Runner),
            Some(40)
        );
        let args: &[u32] = if entry.capability == Capability::ChangeWait {
            &[0, 0]
        } else {
            &[0]
        };
        let mut code = vec![];
        command(&mut code, entry.command + 0x2000, args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        let mut scene = SceneHost::default();
        object(&mut scene);
        let mut camera = CameraHost::default();
        let mut input = ActionInput::default();
        let mut hosts = Hosts {
            waits: Some(WaitHosts {
                scene: (entry.capability != Capability::CameraWait).then_some(&mut scene),
                camera: (entry.capability == Capability::CameraWait).then_some(&mut camera),
                input: &mut input,
                gate: SkipGate::default(),
                change_images: true,
            }),
            ..Default::default()
        };
        assert_eq!(
            dispatch::run(Semantics::ResearchFontCells, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.pc(), code.len());
        assert_eq!(vm.pending_system(), None);
        assert_eq!((vm.stack().len_bytes(), vm.repeat_count()), (0, 0));
        assert_eq!(
            vm.state().system_integer(0),
            Some(u32::from(entry.capability == Capability::QuakeWait))
        );
    }
}

#[test]
fn active_wait_retries_are_frozen_and_completed_skips_yield_with_correct_input_consumption() {
    for (setup, args, wait) in [
        (0x2220u16, vec![0, 0, 100, 1], 0x2224),
        (0x2225, vec![0, 3, 10, 0], 0x2228),
        (0x222f, vec![0, 0, 20, 100, 1], 0x2233),
        (
            0x206c,
            vec![0, 10f32.to_bits(), 20f32.to_bits(), 30f32.to_bits(), 100],
            0x2069,
        ),
    ] {
        for trigger in 0..5 {
            let quake = wait == 0x2228;
            let cancel = if quake {
                trigger == 1
            } else {
                (1..=3).contains(&trigger)
            };
            let parameter = if quake {
                u32::from(!cancel)
            } else {
                u32::from(cancel)
            };
            let mut code = vec![];
            command(&mut code, setup, &args);
            command(&mut code, wait, &[parameter]);
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            pending(&mut vm);
            let mut scene = SceneHost::default();
            let mut camera = CameraHost::default();
            assert!(
                match setup {
                    0x206c => camera.try_handle(&mut vm),
                    0x2225 => scene.try_handle_quake(&mut vm),
                    0x222f => scene.try_handle_strip(&mut vm),
                    _ => scene.try_handle(&mut vm),
                }
                .unwrap()
            );
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let frozen = format!("{scene:?} {camera:?}");
            let mut input = ActionInput::default();
            if trigger != 0 {
                input.set(0, true, true, true).unwrap();
            }
            let original_input = input.clone();
            let gate = SkipGate {
                effects: trigger != 3 && trigger != 4,
                skip: trigger == 2 || trigger == 4,
            };
            if cancel {
                assert_eq!(
                    run(&mut vm, &mut scene, &mut camera, &mut input, gate).unwrap(),
                    Some(Flow::Yield)
                );
                assert_eq!(vm.pending_system(), None);
                assert_eq!(vm.pc(), before.0 + 2);
                assert_eq!((vm.stack().len_bytes(), vm.repeat_count()), (0, 0));
                assert_eq!(vm.state().system_integer(0), Some(1));
                if !quake && trigger == 1 {
                    assert!(!input.released(0));
                } else {
                    assert_eq!(input, original_input);
                }
            } else {
                for repeat in 1..=2 {
                    assert_eq!(
                        run(&mut vm, &mut scene, &mut camera, &mut input, gate).unwrap(),
                        Some(Flow::Wait)
                    );
                    assert_eq!(
                        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                        before
                    );
                    assert_eq!(vm.repeat_count(), repeat);
                    assert_eq!(vm.state().system_integer(0), Some(0));
                    assert_eq!(format!("{scene:?} {camera:?}"), frozen);
                    assert_eq!(input, original_input);
                }
                scene.advance_frame(200);
                camera.advance_frame(200);
                assert_eq!(
                    run(&mut vm, &mut scene, &mut camera, &mut input, gate).unwrap(),
                    Some(Flow::Done)
                );
                assert_eq!(vm.pending_system(), None);
                assert_eq!(vm.pc(), before.0 + 2);
                assert_eq!((vm.stack().len_bytes(), vm.repeat_count()), (0, 0));
                assert_eq!(vm.state().system_integer(0), Some(u32::from(quake)));
                assert_eq!(input, original_input);
            }
        }
    }
}

#[test]
fn wait_parameter_errors_preserve_the_command_and_do_not_consume_input() {
    for (opcode, args, reason) in [
        (0x2224u16, vec![], "skippable"),
        (0x222b, vec![], "skippable"),
        (0x2235, vec![], "skippable"),
        (0x2228, vec![], "wait"),
        (0x2233, vec![], "skippable"),
        (0x2069, vec![], "skippable"),
        (0x22c7, vec![], "object slot"),
        (0x22c7, vec![0], "skippable"),
        (
            0x22c7,
            vec![256, 1],
            "replacement control needs an existing object in slots 0..255",
        ),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 99).unwrap();
        let before = (
            vm.pc(),
            vm.pending_system(),
            vm.stack().len_bytes(),
            vm.repeat_count(),
        );
        let mut scene = SceneHost::default();
        object(&mut scene);
        let mut camera = CameraHost::default();
        let frozen = format!("{scene:?} {camera:?}");
        let mut input = ActionInput::default();
        input.set(0, true, true, true).unwrap();
        let original_input = input.clone();
        assert_eq!(
            run(
                &mut vm,
                &mut scene,
                &mut camera,
                &mut input,
                SkipGate::default()
            )
            .unwrap_err()
            .reason,
            reason
        );
        assert_eq!(
            (
                vm.pc(),
                vm.pending_system(),
                vm.stack().len_bytes(),
                vm.repeat_count()
            ),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert_eq!(input, original_input);
        assert_eq!(format!("{scene:?} {camera:?}"), frozen);
    }
}

#[test]
fn trace_wait_capabilities_do_not_enable_camera_or_change_waits() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-waits-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
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
    fs::remove_file(input).unwrap();
    fs::remove_dir(root).unwrap();
}

use cmvs_rs::{
    camera::CameraHost,
    dispatch::{self, Capability, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
#[path = "support/state_cases.rs"]
mod state_cases;
mod support;
use state_cases::cases;
use support::bytecode::{command, pending, script};

fn run(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    camera: &mut CameraHost,
    capability: Capability,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    dispatch::run(
        Semantics::Root,
        vm,
        &mut Hosts {
            scene: (capability == Capability::Scene).then_some(scene),
            camera: (capability == Capability::Camera).then_some(camera),
            ..Default::default()
        },
    )
}

fn setup(scene: &mut SceneHost) {
    let mut code = vec![];
    for (opcode, args) in [
        (0x2020, vec![3]),
        (0x2022, vec![3, 7]),
        (0x2040, vec![3]),
        (0x2050, vec![3, 7]),
    ] {
        command(&mut code, opcode, &args);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..4 {
        pending(&mut vm);
        assert_eq!(
            run(
                &mut vm,
                scene,
                &mut CameraHost::default(),
                Capability::Scene
            )
            .unwrap(),
            Some(Flow::Done)
        );
    }
}

#[test]
fn batch_has_exact_owners_and_each_command_completes_with_only_its_own_host() {
    let cases = cases();
    for (capability, count) in [(Capability::Scene, 74), (Capability::Camera, 29)] {
        let expected: std::collections::BTreeSet<_> = cases
            .iter()
            .filter(|(_, _, cap)| *cap == capability)
            .map(|(op, _, _)| op - 0x2000)
            .collect();
        let actual = dispatch::ROOT
            .iter()
            .filter(|entry| entry.capability == capability)
            .map(|entry| entry.command)
            .collect();
        assert_eq!(expected.len(), count);
        assert_eq!(expected, actual);
    }
    for (opcode, args, capability) in cases {
        let mut scene = SceneHost::default();
        let mut camera = CameraHost::default();
        setup(&mut scene);
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let objects = scene.objects().clone();
        let camera_words = [
            camera.camera(0).unwrap().words,
            camera.camera(1).unwrap().words,
        ];
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        let wrong = if capability == Capability::Scene {
            Capability::Camera
        } else {
            Capability::Scene
        };
        assert_eq!(
            run(&mut vm, &mut scene, &mut camera, wrong).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(
                Semantics::ResearchFontCells,
                &mut vm,
                &mut Hosts {
                    scene: Some(&mut scene),
                    camera: Some(&mut camera),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(scene.objects(), &objects);
        assert_eq!(
            [
                camera.camera(0).unwrap().words,
                camera.camera(1).unwrap().words
            ],
            camera_words
        );
        assert_eq!(
            run(&mut vm, &mut scene, &mut camera, capability).unwrap(),
            Some(Flow::Done),
            "{opcode:04x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let objects = scene.objects().clone();
        let camera_words = [
            camera.camera(0).unwrap().words,
            camera.camera(1).unwrap().words,
        ];
        assert_eq!(
            run(&mut vm, &mut scene, &mut camera, capability).unwrap(),
            None
        );
        assert_eq!(scene.objects(), &objects);
        assert_eq!(
            [
                camera.camera(0).unwrap().words,
                camera.camera(1).unwrap().words
            ],
            camera_words
        );
    }
}

#[test]
fn every_parameterized_command_rejects_short_stack_without_changing_hosts() {
    for (opcode, args, capability) in cases() {
        if args.is_empty() {
            continue;
        }
        for supplied in [0, args.len() - 1] {
            let mut code = vec![];
            command(&mut code, opcode, &args[..supplied]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let mut scene = SceneHost::default();
            setup(&mut scene);
            let objects = scene.objects().clone();
            let mut camera = CameraHost::default();
            let camera_words = [
                camera.camera(0).unwrap().words,
                camera.camera(1).unwrap().words,
            ];
            let error = run(&mut vm, &mut scene, &mut camera, capability).unwrap_err();
            let decode = error.as_decode().unwrap();
            assert_eq!(decode.opcode, Some(opcode));
            assert_eq!((error.pc, error.command), (before.0, opcode - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(scene.objects(), &objects);
            assert!(scene.fade().current().is_none());
            assert_eq!(
                [
                    camera.camera(0).unwrap().words,
                    camera.camera(1).unwrap().words
                ],
                camera_words
            );
        }
    }
}

#[test]
fn trace_keeps_recording_capability_for_all_batch_commands() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-dispatch-state-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("state.ps3");
    for (opcode, args, _) in cases() {
        for record in [false, true] {
            let mut code = vec![];
            if record {
                for (op, args) in [
                    (0x2020, vec![3]),
                    (0x2022, vec![3, 7]),
                    (0x2040, vec![3]),
                    (0x2050, vec![3, 7]),
                ] {
                    command(&mut code, op, &args);
                }
            }
            command(&mut code, opcode, &args);
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
            let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
            cli.arg(&path).args(["--profile", "cmvs-20160201"]);
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
            let stop = if record { 0x2003 } else { opcode };
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                "{stdout}"
            );
            assert_eq!(fs::read(&path).unwrap(), raw);
        }
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

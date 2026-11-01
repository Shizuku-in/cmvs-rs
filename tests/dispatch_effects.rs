use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
#[allow(dead_code)]
mod support;
use support::bytecode::{command, pending, script};
fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x220c, vec![], Capability::Quake),
        (0x2225, vec![0, 7, 10, 4], Capability::Quake),
        (0x2226, vec![], Capability::Quake),
        (0x2227, vec![1], Capability::Quake),
        (0x2229, vec![], Capability::Quake),
        (0x222f, vec![0, 0x123456, 64, 40, 9], Capability::Strip),
        (0x2230, vec![], Capability::Strip),
        (0x2231, vec![], Capability::Strip),
        (0x2232, vec![], Capability::Strip),
    ]
}
fn run(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    dispatch::run(
        Semantics::Root,
        vm,
        &mut Hosts {
            scene: Some(scene),
            ..Default::default()
        },
    )
}
fn active() -> SceneHost {
    let mut code = vec![];
    command(&mut code, 0x2225, &[0, 7, 10, 4]);
    command(&mut code, 0x222f, &[0, 0x123456, 64, 40, 9]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..2 {
        pending(&mut vm);
        assert_eq!(run(&mut vm, &mut scene).unwrap(), Some(Flow::Done));
    }
    scene
}
#[test]
fn effects_have_unique_owners_and_preserve_missing_capabilities() {
    for (opcode, args, capability) in cases() {
        let entries: Vec<_> = dispatch::ROOT
            .iter()
            .filter(|e| e.command == opcode - 0x2000)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].capability, capability);
        for active_state in [false, true] {
            let mut scene = if active_state {
                active()
            } else {
                SceneHost::default()
            };
            let before_scene = format!("{scene:?}");
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                dispatch::run(
                    Semantics::ResearchFontCells,
                    &mut vm,
                    &mut Hosts {
                        scene: Some(&mut scene),
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
            assert_eq!(format!("{scene:?}"), before_scene);
            assert_eq!(run(&mut vm, &mut scene).unwrap(), Some(Flow::Done));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                (before.0 + 2, None, 0)
            );
            let before_scene = format!("{scene:?}");
            assert_eq!(run(&mut vm, &mut scene).unwrap(), None);
            assert_eq!(format!("{scene:?}"), before_scene);
        }
    }
}
#[test]
fn effect_arguments_and_rejected_modes_leave_active_effects_intact() {
    let mut failures = vec![];
    for (op, args, _) in cases().into_iter().filter(|(_, a, _)| !a.is_empty()) {
        for len in [0, args.len() - 1] {
            failures.push((op, args[..len].to_vec()));
        }
    }
    failures.extend([
        (0x2225, vec![0x20, 7, 10, 4]),
        (0x2225, vec![0x100, 7, 10, 4]),
        (0x222f, vec![3, 0, 64, 40, 9]),
        (0x222f, vec![0, 0, 0, 40, 9]),
    ]);
    for (opcode, args) in failures {
        let mut scene = active();
        let before_scene = format!("{scene:?}");
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let e = run(&mut vm, &mut scene).unwrap_err();
        assert_eq!((e.pc, e.command), (before.0, opcode - 0x2000));
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(format!("{scene:?}"), before_scene);
    }
}

#[test]
fn trace_keeps_effect_recording_capability() {
    use std::{fs, process::Command};
    let root =
        std::env::temp_dir().join(format!("cmvs-cli-dispatch-effects-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("windows.ps3");
    for (opcode, args, _) in cases() {
        let mut code = vec![];
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
        for record in [false, true] {
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
            assert!(
                stdout.contains(&format!(
                    "# STOP pending system=0x{:04x};",
                    if record { 0x2003 } else { opcode }
                )),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

use cmvs_rs::{
    buttons::ButtonHost,
    dispatch::{self, Capability, Flow, Hosts, InputHosts},
    gesture::PointerGesture,
    input::ActionInput,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
#[path = "support/control_cases.rs"]
mod control_cases;
mod support;
use control_cases::cases;
use support::bytecode::{command, pending, script};

#[derive(Default)]
struct Controls {
    scene: SceneHost,
    buttons: ButtonHost,
    input: ActionInput,
    gesture: PointerGesture,
}
impl Controls {
    fn hosts(&mut self, capability: Capability) -> Hosts<'_> {
        let button = matches!(
            capability,
            Capability::Buttons | Capability::ButtonInput | Capability::ListInput
        );
        let input = capability != Capability::Buttons;
        let gesture = matches!(capability, Capability::ListInput | Capability::DragPress);
        Hosts {
            scene: button.then_some(&mut self.scene),
            buttons: button.then_some(&mut self.buttons),
            input: input.then_some(InputHosts {
                input: &mut self.input,
                skip: false,
                clock: (capability == Capability::DragPress).then_some(123),
            }),
            gesture: gesture.then_some(&mut self.gesture),
            list_input: capability == Capability::ListInput,
            ..Default::default()
        }
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?}{:?}{:?}{:?}",
            self.scene, self.buttons, self.input, self.gesture
        )
    }
    fn setup(&mut self) {
        let mut code = vec![];
        for (op, args) in [
            (0x2020, vec![3]),
            (0x2040, vec![3]),
            (0x2022, vec![3, 0]),
            (0x2050, vec![3, 0]),
            (0x2210, vec![0, 3]),
            (0x2370, vec![0, 3]),
            (0x2377, vec![0]),
        ] {
            command(&mut code, op, &args);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        for i in 0..7 {
            pending(&mut vm);
            let capability = if i < 4 {
                Capability::Scene
            } else if i < 6 {
                Capability::Buttons
            } else {
                Capability::ListInput
            };
            if capability == Capability::Scene {
                assert!(self.scene.try_handle(&mut vm).unwrap());
            } else {
                assert_eq!(
                    dispatch::run(Semantics::Root, &mut vm, &mut self.hosts(capability)).unwrap(),
                    Some(Flow::Done)
                );
            }
        }
    }
}

#[test]
fn all_control_entries_require_their_own_context_and_complete_once() {
    for (capability, count) in [
        (Capability::Input, 47),
        (Capability::Buttons, 21),
        (Capability::ButtonInput, 3),
        (Capability::ListInput, 1),
        (Capability::DragPress, 1),
    ] {
        let expected: std::collections::BTreeSet<_> = cases()
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
    assert!(
        dispatch::ROOT
            .iter()
            .any(|entry| entry.command == 0x34d && entry.capability == Capability::FunctionKeys)
    );
    for (opcode, args, capability) in cases() {
        let mut controls = Controls::default();
        controls.setup();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let state = controls.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(
                Semantics::ResearchFontCells,
                &mut vm,
                &mut controls.hosts(capability)
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(controls.snapshot(), state);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut controls.hosts(capability)).unwrap(),
            Some(Flow::Done),
            "{opcode:04x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let state = controls.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut controls.hosts(capability)).unwrap(),
            None
        );
        assert_eq!(controls.snapshot(), state);
    }
}

#[test]
fn partial_button_and_gesture_contexts_cannot_run_or_mutate_input() {
    for (opcode, capability, parts) in [
        (0x2210, Capability::Buttons, 2),
        (0x2217, Capability::ButtonInput, 3),
        (0x2377, Capability::ListInput, 5),
        (0x2369, Capability::DragPress, 3),
    ] {
        let args = cases()
            .into_iter()
            .find(|(op, _, _)| *op == opcode)
            .unwrap()
            .1;
        for missing in 0..parts {
            let mut controls = Controls::default();
            controls.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let state = controls.snapshot();
            let mut hosts = controls.hosts(capability);
            if capability == Capability::DragPress {
                match missing {
                    0 => hosts.gesture = None,
                    1 => hosts.input = None,
                    _ => hosts.input.as_mut().unwrap().clock = None,
                }
            } else {
                match missing {
                    0 => hosts.buttons = None,
                    1 => hosts.scene = None,
                    2 => hosts.input = None,
                    3 => hosts.gesture = None,
                    _ => hosts.list_input = false,
                }
            }
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(controls.snapshot(), state);
        }
    }
}

#[test]
fn short_arguments_preserve_buttons_scene_and_input_edges() {
    for (opcode, args, capability) in cases() {
        if args.is_empty() {
            continue;
        }
        for supplied in [0, args.len() - 1] {
            let mut controls = Controls::default();
            controls.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args[..supplied]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let state = controls.snapshot();
            let error = dispatch::run(Semantics::Root, &mut vm, &mut controls.hosts(capability))
                .unwrap_err();
            assert_eq!(error.as_decode().unwrap().opcode, Some(opcode));
            assert_eq!((error.pc, error.command), (before.0, opcode - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(controls.snapshot(), state);
        }
    }
}

#[test]
fn trace_keeps_control_capabilities_and_uninitialized_list_query_barrier() {
    use std::{fs, process::Command};
    let root =
        std::env::temp_dir().join(format!("cmvs-cli-dispatch-controls-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("controls.ps3");
    for (opcode, args, _) in cases() {
        for record in [false, true] {
            let mut code = vec![];
            if record {
                for (op, args) in [
                    (0x2020, vec![3]),
                    (0x2040, vec![3]),
                    (0x2022, vec![3, 0]),
                    (0x2050, vec![3, 0]),
                    (0x2210, vec![0, 3]),
                    (0x2370, vec![0, 3]),
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
            if record && opcode == 0x237f {
                assert!(!output.status.success());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("auxiliary list hit point is not initialized")
                );
            } else {
                assert!(
                    output.status.success(),
                    "{stdout}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let stop = if record && !matches!(opcode, 0x2377 | 0x2369) {
                    0x2003
                } else {
                    opcode
                };
                assert!(
                    stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                    "{stdout}"
                );
            }
            assert_eq!(fs::read(&path).unwrap(), raw);
        }
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

use cmvs_rs::{
    dispatch::{self, Flow, Hosts},
    machine::Machine,
    message::MessageHost,
    profile::Semantics,
    ps3::Script,
    scene::SceneHost,
};
#[allow(dead_code)]
mod support;
#[path = "support/text_state_cases.rs"]
mod text_state_cases;
use support::bytecode::{command, pending};
use text_state_cases::{cases, setup};
fn bytes(code: &[u8]) -> Vec<u8> {
    let text = b"\0abc\0";
    let mut raw = b"PS2A".to_vec();
    for word in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        raw.extend(word.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(text)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&bytes(code)).unwrap()
}

#[derive(Default)]
struct State {
    messages: MessageHost,
    scene: SceneHost,
}
impl State {
    fn hosts(&mut self) -> Hosts<'_> {
        Hosts {
            messages: Some(&mut self.messages),
            scene: Some(&mut self.scene),
            ..Default::default()
        }
    }
    fn snapshot(&self) -> String {
        format!("{:?} {:?}", self.messages, self.scene)
    }
    fn setup(&mut self) {
        let setup = setup();
        let mut code = vec![];
        for (op, args) in &setup {
            command(&mut code, *op, args);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        for _ in setup {
            pending(&mut vm);
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut self.hosts()).unwrap(),
                Some(Flow::Done)
            );
        }
    }
}
#[test]
fn all_text_state_commands_have_one_owner_and_complete_once() {
    assert_eq!(cases().len(), 45);
    for (opcode, args, capability) in cases() {
        let entries: Vec<_> = dispatch::ROOT
            .iter()
            .filter(|e| e.command == opcode - 0x2000)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].capability, capability);
        assert!(dispatch::uses_message_state(opcode - 0x2000));
        let mut state = State::default();
        state.setup();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(Semantics::ResearchFontCells, &mut vm, &mut state.hosts()).unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(state.snapshot(), snapshot);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts()).unwrap(),
            Some(Flow::Done),
            "{opcode:x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts()).unwrap(),
            None
        );
        assert_eq!(state.snapshot(), snapshot);
    }
}
#[test]
fn short_text_arguments_preserve_messages_scene_and_vm() {
    for (opcode, args, _) in cases() {
        for len in [0, args.len() - 1] {
            let mut state = State::default();
            state.setup();
            let snapshot = state.snapshot();
            let mut code = vec![];
            command(&mut code, opcode, &args[..len]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let err = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts()).unwrap_err();
            assert_eq!((err.pc, err.command), (before.0, opcode - 0x2000));
            assert_eq!(err.as_decode().unwrap().opcode, Some(opcode));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
        }
    }
}
#[test]
fn auxiliary_scene_context_is_conditional_and_retryable() {
    let mut state = State::default();
    state.setup();
    let mut code = vec![];
    for (op, args) in [
        (0x2383, vec![0, 1, 2]),
        (0x239d, vec![0, 11]),
        (0x2383, vec![0, 16777217, i32::MAX as u32]),
        (0x239d, vec![0, u32::MAX]),
        (0x2383, vec![0, 3, 4]),
    ] {
        command(&mut code, op, &args);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    for i in 0..5 {
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let snapshot = state.snapshot();
        let flow = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                messages: Some(&mut state.messages),
                ..Default::default()
            },
        )
        .unwrap();
        if matches!(i, 1..=3) {
            assert_eq!(flow, Some(Flow::Unavailable));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut state.hosts()).unwrap(),
                Some(Flow::Done)
            );
        } else {
            assert_eq!(flow, Some(Flow::Done));
        }
        if i == 2 {
            assert_eq!(
                &state.scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
                &[16777216, i32::MIN as u32]
            );
        }
    }
    assert_eq!(
        state.messages.auxiliary_text(0).unwrap().associated_scene,
        None
    );
}

#[test]
fn trace_keeps_text_state_capabilities_unavailable() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!(
        "cmvs-cli-dispatch-text-state-{}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("windows.ps3");
    for (opcode, args, _) in cases() {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        command(&mut code, 0x2003, &[]);
        let raw = bytes(&code);
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
                stdout.contains(&format!("# STOP pending system=0x{opcode:04x};")),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

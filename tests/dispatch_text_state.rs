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
use support::bytecode::{command, pending};
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
        let setup = [
            (0x2020, vec![11]),
            (0x2040, vec![11]),
            (0x2100, vec![0, 0, 0]),
            (0x2105, vec![0, 25, 0, 0]),
            (0x2103, vec![0, 0, 0, 200, 100]),
            (0x2380, vec![0, 0, 25, 32]),
            (0x2385, vec![0, 0, 0, 200, 100]),
        ];
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
fn natsuuta_logical_text_creation_stores_the_scripted_size() {
    let profile = cmvs_rs::profile::Registry::builtin()
        .profile("cmvs-20100314")
        .unwrap();
    let semantics = profile.semantics(0x11c);
    assert!(matches!(semantics, Semantics::Variant(_)));
    assert!(dispatch::uses_message_state(0x11c));
    let mut state = State::default();
    let mut code = vec![];
    command(&mut code, 0x211c, &[1, 0, 0, 1280, 960]);
    let created = script(&code);
    let mut vm = Machine::new(&created).unwrap();
    pending(&mut vm);
    assert_eq!(
        dispatch::run(semantics, &mut vm, &mut state.hosts()).unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(state.messages.text(1).unwrap().logical_size, [1280, 960]);

    for size in [[0, 960], [1280, u32::MAX]] {
        let mut state = State::default();
        let mut code = vec![];
        command(&mut code, 0x211c, &[1, 0, 0, size[0], size[1]]);
        let invalid = script(&code);
        let mut vm = Machine::new(&invalid).unwrap();
        pending(&mut vm);
        let before = state.snapshot();
        assert!(dispatch::run(semantics, &mut vm, &mut state.hosts()).is_err());
        assert_eq!(state.snapshot(), before);
        assert_eq!(vm.stack().len_bytes(), 20);
    }
    // Without the binding the root table has no 0x211c entry.
    let mut vm = Machine::new(&created).unwrap();
    pending(&mut vm);
    assert_eq!(
        dispatch::run(Semantics::Root, &mut vm, &mut State::default().hosts()).unwrap(),
        None
    );
}

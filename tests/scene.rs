use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    scene::{ClearState, SceneHost, SceneObject},
};

fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
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
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn push_then(value: u32, ops: &[u16]) -> Vec<u8> {
    let mut code = 0x440u16.to_le_bytes().to_vec();
    code.extend(4u16.to_le_bytes());
    code.extend(value.to_le_bytes());
    for op in ops {
        code.extend(op.to_le_bytes());
    }
    code
}

#[test]
fn defaults_match_device_constructor() {
    let clear = SceneHost::default().clear();
    assert_eq!(clear, ClearState::default());
    assert_eq!((clear.color, clear.depth), (0, false));
    assert_eq!(clear.rgba(), [0, 0, 0, 255]);
}

#[test]
fn clear_color_keeps_raw_word_and_renders_low_24_bits_opaque() {
    let script = script(&push_then(0x80_12_34_56, &[0x22cb, 0x2250]));
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    vm.step().unwrap();
    assert_eq!(
        vm.step().unwrap(),
        Step::System {
            pc: 8,
            command: 0x2cb
        }
    );
    assert!(scene.try_handle(&mut vm).unwrap());
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (10, 0));
    assert_eq!(scene.clear().color, 0x80_12_34_56);
    assert_eq!(scene.clear().rgba(), [0x12, 0x34, 0x56, 255]);
    assert!(!scene.clear().depth);
}

#[test]
fn depth_flag_is_normalized_and_missing_argument_keeps_pending_state() {
    let script = script(&push_then(7, &[0x2250, 0x2250]));
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    vm.step().unwrap();
    vm.step().unwrap();
    assert!(scene.try_handle(&mut vm).unwrap());
    assert!(scene.clear().depth);
    assert_eq!(
        vm.step().unwrap(),
        Step::System {
            pc: 10,
            command: 0x250
        }
    );
    assert!(scene.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system(), Some((10, 0x250)));
    assert_eq!(vm.pc(), 10);
    assert!(scene.clear().depth);
}

#[test]
fn other_commands_are_left_pending() {
    let script = script(&push_then(1, &[0x22cc]));
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    assert!(!scene.try_handle(&mut vm).unwrap());
    vm.step().unwrap();
    vm.step().unwrap();
    assert!(!scene.try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system(), Some((8, 0x2cc)));
    assert_eq!(scene.clear(), ClearState::default());
}

#[test]
fn object_slots_are_rebuilt_destroyed_and_bounded() {
    let mut code = vec![];
    for op in [0x2020, 0x2020, 0x2021, 0x2021] {
        code.extend(push_then(10, &[op]));
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for expected in [true, true, false, false] {
        vm.step().unwrap();
        vm.step().unwrap();
        assert!(scene.try_handle(&mut vm).unwrap());
        assert_eq!(scene.object(10).is_some(), expected);
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    for slot in [256u32, u32::MAX] {
        let program = script(&push_then(slot, &[0x2020]));
        let mut vm = Machine::new(&program).unwrap();
        vm.step().unwrap();
        vm.step().unwrap();
        assert!(scene.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((8, 0x20)));
    }
    assert!(scene.objects().is_empty());
    let program = script(&push_then(3, &[0x2020]));
    let mut vm = Machine::new(&program).unwrap();
    vm.step().unwrap();
    vm.step().unwrap();
    scene.try_handle(&mut vm).unwrap();
    assert_eq!(scene.object(3), Some(&SceneObject::default()));
}

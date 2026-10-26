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

fn command(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(push_then(arg, &[]));
    }
    code.extend(op.to_le_bytes());
}
fn next(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
) -> Result<bool, cmvs_rs::bytecode::DecodeError> {
    loop {
        if matches!(vm.step()?, Step::System { .. }) {
            return scene.try_handle(vm);
        }
    }
}

#[test]
fn draw_initialization_properties_and_reinitialization_preserve_other_objects() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![10]),
        (0x2040, vec![10]),
        (0x2044, vec![10, u32::MAX, 5, 6, 7, 8]),
        (0x2046, vec![10, u32::MAX, 2, 3]),
        (0x2045, vec![10, u32::MAX, 100, 200]),
        (0x2047, vec![10, u32::MAX, 32]),
        (0x2048, vec![10, u32::MAX, 999]),
        (0x2048, vec![10, u32::MAX, u32::MAX]),
        (0x204b, vec![10, u32::MAX, 2]),
        (0x204b, vec![10, u32::MAX, 4]),
        (0x2040, vec![10]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..7 {
        assert!(next(&mut vm, &mut scene).unwrap());
    }
    let d = &scene.object(10).unwrap().draw.as_ref().unwrap().words;
    assert_eq!(&d[2..11], &[5, 6, 7, 8, 2, 3, 100, 200, 32]);
    assert_eq!(d[17], 255);
    for _ in 0..3 {
        next(&mut vm, &mut scene).unwrap();
    }
    let d = &scene.object(10).unwrap().draw.as_ref().unwrap().words;
    assert_eq!(d[17], u32::MAX); // Original only clamps the upper end, signed.
    assert_eq!(d[1], 2); // Unsupported blend selector is a verified no-op.
    next(&mut vm, &mut scene).unwrap();
    assert_eq!(
        scene.object(10).unwrap().draw,
        Some(cmvs_rs::scene::DrawState::default())
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn children_rebuild_and_inherit_constructor_flags_at_creation_time() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![1]),
        (0x2022, vec![1, 767]),
        (0x2050, vec![1, 767]),
        (0x2045, vec![1, 767, 9, 10]),
        (0x2040, vec![1]),
        (0x2022, vec![1, 767]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..4 {
        next(&mut vm, &mut scene).unwrap();
    }
    let parent = scene.object(1).unwrap();
    assert_eq!(parent.flags, u32::MAX);
    assert!(parent.draw.is_none());
    assert_eq!(parent.children[&767].flags, 0);
    assert_eq!(
        &parent.children[&767].draw.as_ref().unwrap().words[8..10],
        &[9, 10]
    );
    for _ in 0..2 {
        next(&mut vm, &mut scene).unwrap();
    }
    assert_eq!(
        scene.object(1).unwrap().children[&767],
        SceneObject::default()
    );
}

#[test]
fn invalid_property_targets_and_truncated_arguments_are_transactional() {
    for (op, args) in [
        (0x2040, vec![2]),
        (0x2040, vec![256]),
        (0x2022, vec![1, 768]),
        (0x2050, vec![1, 0]),
        (0x2045, vec![1, u32::MAX, 5, 6]), // no draw state
        (0x2044, vec![1, u32::MAX, 5]),    // truncated stack
    ] {
        let mut code = vec![];
        command(&mut code, 0x2020, &[1]);
        command(&mut code, op, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        next(&mut vm, &mut scene).unwrap();
        let before = scene.objects().clone();
        assert!(next(&mut vm, &mut scene).is_err());
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.pending_system().unwrap().1, op - 0x2000);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

#[test]
fn destroy_all_clears_every_slot_but_keeps_stack_fade_and_clear_state() {
    let mut code = vec![];
    for (op, args) in [
        (0x22cb, vec![0x123456]),
        (0x2020, vec![0]),
        (0x2020, vec![255]),
        (0x2040, vec![255]),
        (0x2022, vec![255, 3]),
        (0x2220, vec![0, 0, 2000, 9]),
    ] {
        command(&mut code, op, &args);
    }
    // A leftover argument shows that 0x202f pops nothing.
    code.extend(push_then(0xdead, &[0x202f, 0x202f]));
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..6 {
        assert!(next(&mut vm, &mut scene).unwrap());
    }
    vm.state_mut().set_system_integer(0, 5).unwrap();
    let fade = scene.fade().current().cloned();
    assert!(fade.is_some());
    assert_eq!(scene.objects().len(), 2);
    for _ in 0..2 {
        assert!(next(&mut vm, &mut scene).unwrap());
        assert!(scene.objects().is_empty());
        assert_eq!(vm.stack().len_bytes(), 4);
    }
    assert_eq!(vm.pc(), code.len());
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdead);
    assert_eq!(scene.fade().current().cloned(), fade);
    assert_eq!(scene.clear().color, 0x123456);
    assert_eq!(vm.state().system_integer(0), Some(5));
}

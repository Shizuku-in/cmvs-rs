use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    scene::{ClearState, DrawState, SceneHost, SceneObject},
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
fn coordinate_mode_two_preserves_parent_child_properties_and_system_results() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![255]),
        (0x2040, vec![255]),
        (0x2045, vec![255, u32::MAX, 100, 200]),
        (0x21ea, vec![255, u32::MAX, 23]),
        (0x2022, vec![255, 767]),
        (0x2050, vec![255, 767]),
        (0x2048, vec![255, 767, 42]),
        (0x2042, vec![255, 0x8000_0000]), // Any signed negative child selects the parent.
        (0x2042, vec![255, 767]),
        (0x2042, vec![255, 767]), // Idempotent.
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..7 {
        assert!(next(&mut vm, &mut scene).unwrap());
    }
    let mut expected = scene.object(255).unwrap().clone();
    vm.state_mut().set_system_integer(0, 0xdead_beef).unwrap();
    assert!(next(&mut vm, &mut scene).unwrap());
    expected.draw.as_mut().unwrap().words[0] = 2;
    assert_eq!(scene.object(255), Some(&expected));
    expected
        .children
        .get_mut(&767)
        .unwrap()
        .draw
        .as_mut()
        .unwrap()
        .words[0] = 2;
    for _ in 0..2 {
        assert!(next(&mut vm, &mut scene).unwrap());
        assert_eq!(scene.object(255), Some(&expected));
    }
    assert_eq!(vm.pc(), code.len());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(0xdead_beef));
    // The mode is retained, but its presentation is not yet modeled.
    assert!(cmvs_rs::scene_render::render(&scene, 1, 1).is_err());
}

#[test]
fn camera_coordinate_mode_is_three_for_nonzero_values_and_two_for_zero() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![255]),
        (0x2040, vec![255]),
        (0x2045, vec![255, u32::MAX, 100, 200]),
        (0x2022, vec![255, 767]),
        (0x2050, vec![255, 767]),
        (0x2048, vec![255, 767, 42]),
    ] {
        command(&mut code, op, &args);
    }
    // (child, value, mode): any signed negative child selects the parent.
    let cases = [
        (0x8000_0000, 1, 3),
        (767, 0x8000_0000, 3),
        (767, 0, 2),
        (u32::MAX, 0, 2),
        (u32::MAX, u32::MAX, 3),
    ];
    for (child, value, _) in cases {
        command(&mut code, 0x204c, &[255, child, value]);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..6 {
        assert!(next(&mut vm, &mut scene).unwrap());
    }
    vm.state_mut().set_system_integer(0, 0xdead_beef).unwrap();
    let mut expected = scene.object(255).unwrap().clone();
    for (child, _, mode) in cases {
        assert!(next(&mut vm, &mut scene).unwrap());
        let target = if child == 767 {
            expected.children.get_mut(&767).unwrap()
        } else {
            &mut expected
        };
        target.draw.as_mut().unwrap().words[0] = mode;
        assert_eq!(scene.object(255), Some(&expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(vm.pc(), code.len());
    assert_eq!(vm.state().system_integer(0), Some(0xdead_beef));
    // Camera projection is not presented yet.
    assert!(cmvs_rs::scene_render::render(&scene, 1, 1).is_err());
}

#[test]
fn projection_parameters_preserve_raw_float_bits_and_each_other() {
    let values = [
        70.0f32.to_bits(),
        (-12.5f32).to_bits(),
        0,
        0x8000_0000,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_1234,
        0x7f80_0001,
    ];
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![255]),
        (0x2040, vec![255]),
        (0x2042, vec![255, u32::MAX]),
        (0x2045, vec![255, u32::MAX, 100, 200]),
        (0x21ea, vec![255, u32::MAX, 23]),
        (0x2022, vec![255, 767]),
        (0x2050, vec![255, 767]),
    ] {
        command(&mut code, op, &args);
    }
    for value in values {
        for op in [0x2070, 0x2071, 0x2072, 0x2073, 0x2074, 0x2075] {
            command(&mut code, op, &[255, 0x8000_0000, value]);
            command(&mut code, op, &[255, 767, value]);
        }
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..7 {
        assert!(next(&mut vm, &mut scene).unwrap());
    }
    vm.state_mut().set_system_integer(0, 123).unwrap();
    vm.state_mut().set_system_integer(4, 456).unwrap();
    let mut expected = scene.object(255).unwrap().clone();
    // Index 27 stands for +108, the depth bias kept outside the first 88 bytes.
    let set = |draw: &mut DrawState, index: usize, value: u32| match index {
        27 => draw.depth_bias = value,
        _ => draw.words[index] = value,
    };
    for value in values {
        for index in [15, 16, 11, 12, 13, 27] {
            assert!(next(&mut vm, &mut scene).unwrap());
            set(expected.draw.as_mut().unwrap(), index, value);
            assert_eq!(scene.object(255), Some(&expected));
            assert!(next(&mut vm, &mut scene).unwrap());
            let child = expected.children.get_mut(&767).unwrap();
            set(child.draw.as_mut().unwrap(), index, value);
            assert_eq!(scene.object(255), Some(&expected));
            assert_eq!(vm.stack().len_bytes(), 0);
            assert_eq!(vm.state().system_integer(0), Some(123));
            assert_eq!(vm.state().system_integer(4), Some(456));
        }
    }
    assert_eq!(vm.pc(), code.len());
    // Preserving the projection input is not an implementation of camera presentation.
    assert!(cmvs_rs::scene_render::render(&scene, 1, 1).is_err());
}

#[test]
fn projection_setter_errors_preserve_targets_stack_and_pending_command() {
    for op in [0x204c, 0x2071, 0x2072, 0x2073, 0x2074, 0x2075] {
        for args in [
            vec![256, u32::MAX, 0],
            vec![u32::MAX, u32::MAX, 0],
            vec![2, u32::MAX, 0],
            vec![1, 768, 0],
            vec![1, 0, 0],        // Missing child.
            vec![1, 767, 0],      // Existing child without a draw state.
            vec![1, u32::MAX, 0], // Parent without a draw state.
            vec![1, u32::MAX],    // Missing float word.
        ] {
            let mut code = vec![];
            command(&mut code, 0x2020, &[1]);
            command(&mut code, 0x2022, &[1, 767]);
            command(&mut code, op, &args);
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            let mut scene = SceneHost::default();
            for _ in 0..2 {
                next(&mut vm, &mut scene).unwrap();
            }
            vm.state_mut().set_system_integer(0, 123).unwrap();
            let before = scene.objects().clone();
            assert!(next(&mut vm, &mut scene).is_err());
            assert_eq!(scene.objects(), &before);
            assert_eq!(vm.pending_system(), Some((code.len() - 2, op - 0x2000)));
            assert_eq!(vm.pc(), code.len() - 2);
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            assert_eq!(vm.state().system_integer(0), Some(123));
        }
    }
}

#[test]
fn coordinate_mode_two_errors_preserve_pending_command_stack_and_objects() {
    for args in [
        vec![256, u32::MAX],
        vec![u32::MAX, u32::MAX],
        vec![2, u32::MAX], // Missing parent.
        vec![1, 768],
        vec![1, 0],        // Missing child.
        vec![1, 767],      // Child without a draw state.
        vec![1, u32::MAX], // Parent without a draw state.
        vec![1],           // Missing argument.
    ] {
        let mut code = vec![];
        command(&mut code, 0x2020, &[1]);
        command(&mut code, 0x2022, &[1, 767]);
        command(&mut code, 0x2042, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        for _ in 0..2 {
            assert!(next(&mut vm, &mut scene).unwrap());
        }
        let before = scene.objects().clone();
        assert!(next(&mut vm, &mut scene).is_err());
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x42)));
        assert_eq!(vm.pc(), code.len() - 2);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
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
        (0x2045, vec![1, u32::MAX, 5, 6]),        // no draw state
        (0x2044, vec![1, u32::MAX, 5]),           // truncated stack
        (0x2070, vec![1, u32::MAX, 0x428c_0000]), // Parent without draw state.
        (0x2070, vec![2, u32::MAX, 0]),           // Missing parent.
        (0x2070, vec![256, u32::MAX, 0]),
        (0x2070, vec![u32::MAX, u32::MAX, 0]),
        (0x2070, vec![1, 768, 0]),
        (0x2070, vec![1, 0, 0]),     // Missing child.
        (0x2070, vec![1, u32::MAX]), // Missing float parameter.
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

/// Pushes `words` bottom first, then issues `command`.
fn invoke(code: &mut Vec<u8>, command: u16, words: &[u32]) {
    for &word in words {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(word.to_le_bytes());
    }
    code.extend(command.to_le_bytes());
}

#[test]
fn objects_move_between_slots_and_existence_queries_report_them() {
    let mut code = vec![];
    invoke(&mut code, 0x2020, &[29]);
    invoke(&mut code, 0x2022, &[5, 29]);
    invoke(&mut code, 0x2020, &[30]);
    // 0x2028: SP-4 target 30, SP-8 source 29.
    invoke(&mut code, 0x2028, &[29, 30]);
    invoke(&mut code, 0x2027, &[u32::MAX, 30]);
    invoke(&mut code, 0x2027, &[5, 30]);
    invoke(&mut code, 0x2027, &[6, 30]);
    invoke(&mut code, 0x2027, &[u32::MAX, 29]);
    invoke(&mut code, 0x2027, &[768, 30]);
    // Moving from an empty slot empties the target.
    invoke(&mut code, 0x2028, &[29, 30]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let step = |vm: &mut Machine<'_>, scene: &mut SceneHost| {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert!(scene.try_handle(vm).unwrap());
    };
    for _ in 0..4 {
        step(&mut vm, &mut scene);
    }
    assert!(scene.object(29).is_none());
    assert!(scene.object(30).unwrap().children.contains_key(&5));
    for expected in [1, 1, 0, 0, 0] {
        step(&mut vm, &mut scene);
        assert_eq!(vm.state().system_integer(0), Some(expected));
    }
    step(&mut vm, &mut scene);
    assert!(scene.objects().is_empty());
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn object_moves_and_queries_reject_bad_slots_and_self_moves() {
    for (setup, command, words) in [
        (true, 0x2028u16, vec![3, 3]),
        (false, 0x2028, vec![256, 3]),
        (false, 0x2028, vec![3, u32::MAX]),
        (false, 0x2027, vec![0, 256]),
        (false, 0x2027, vec![0]),
    ] {
        let mut code = vec![];
        if setup {
            invoke(&mut code, 0x2020, &[3]);
        }
        invoke(&mut code, command, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        let mut scene = SceneHost::default();
        if setup {
            while !matches!(vm.step().unwrap(), Step::System { .. }) {}
            assert!(scene.try_handle(&mut vm).unwrap());
        }
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let bytes = vm.stack().len_bytes();
        assert!(scene.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), Some(9));
        assert_eq!(scene.object(3).is_some(), setup);
    }
}

/// Slot `slot` with a draw state, an opacity track starting at `base`, and keys
/// (next, hold, duration, value) played from key 0.
fn tracked(code: &mut Vec<u8>, slot: u32, base: u32, keys: &[(u32, u32, u32, u32)]) {
    invoke(code, 0x2020, &[slot]);
    invoke(code, 0x2040, &[slot]);
    invoke(code, 0x21ea, &[base, u32::MAX, slot]);
    for (index, &(next, hold, duration, value)) in keys.iter().enumerate() {
        invoke(
            code,
            0x21eb,
            &[value, duration, hold, next, index as u32, u32::MAX, slot],
        );
    }
    invoke(code, 0x21ec, &[0, u32::MAX, slot]);
}
fn run_all(vm: &mut Machine<'_>, scene: &mut SceneHost, commands: usize) {
    for _ in 0..commands {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert!(scene.try_handle(vm).unwrap(), "{:?}", vm.pending_system());
    }
}
fn opacity(scene: &SceneHost, slot: u8) -> u32 {
    scene.object(slot).unwrap().draw.as_ref().unwrap().words[17]
}

#[test]
fn opacity_tracks_interpolate_with_clamped_frame_time_and_free_at_the_end() {
    let mut code = vec![];
    tracked(&mut code, 3, 0, &[(u32::MAX, 0, 1000, 255)]);
    invoke(&mut code, 0x21ee, &[u32::MAX, 3]);
    invoke(&mut code, 0x21ee, &[u32::MAX, 3]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 5);
    assert_eq!(opacity(&scene, 3), 255, "starting does not change opacity");
    run_all(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(0));
    let mut seen = vec![];
    for delta in [250, 300, 100, 200, 200, 200] {
        scene.advance_frame(delta);
        seen.push(opacity(&scene, 3));
    }
    assert_eq!(seen, [51, 102, 127, 178, 229, 255]);
    let draw = scene.object(3).unwrap().draw.as_ref().unwrap();
    assert!(draw.opacity_track.is_none());
    run_all(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn opacity_keys_chain_hold_reverse_and_dispose() {
    let mut code = vec![];
    // Slot 1: two keys; slot 2: 255 down to 0; slot 4: held key; slot 5 ends with -2.
    tracked(
        &mut code,
        1,
        0,
        &[(1, 0, 100, 100), (u32::MAX, 0, 100, 200)],
    );
    tracked(&mut code, 2, 255, &[(u32::MAX, 0, 100, 0)]);
    tracked(&mut code, 4, 10, &[(u32::MAX, 1, 100, 90)]);
    tracked(&mut code, 5, 0, &[((-2i32) as u32, 0, 100, 0)]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 6 + 5 + 5 + 5);
    scene.advance_frame(150);
    // Slot 1 reached key 0 (value 100) and moved on without interpolating key 1 yet.
    assert_eq!(opacity(&scene, 1), 100);
    assert!(
        scene.object(5).is_none(),
        "the -2 end disposes the slot object"
    );
    scene.advance_frame(25);
    assert_eq!(opacity(&scene, 1), 175);
    // After 175 ms: slot 2 ended at 0; slot 4 held its old opacity until the end.
    assert_eq!(opacity(&scene, 2), 0);
    assert_eq!(opacity(&scene, 4), 90);
    let mut code = vec![];
    tracked(&mut code, 2, 255, &[(u32::MAX, 0, 100, 0)]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 5);
    scene.advance_frame(50);
    // (0 - 255) as u32 * 50 is -12750 as i32; divided by 100 that is -127.
    assert_eq!(opacity(&scene, 2), 128);
    let mut code = vec![];
    tracked(&mut code, 4, 10, &[(u32::MAX, 1, 100, 90)]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 5);
    scene.advance_frame(50);
    assert_eq!(opacity(&scene, 4), 255, "a held key does not interpolate");
}

#[test]
fn stopping_and_querying_opacity_tracks() {
    let mut code = vec![];
    tracked(&mut code, 3, 0, &[(u32::MAX, 0, 1000, 77)]);
    invoke(&mut code, 0x21ed, &[u32::MAX, 3]);
    invoke(&mut code, 0x21ea, &[0x8000_0000, u32::MAX, 3]);
    invoke(&mut code, 0x21ee, &[u32::MAX, 3]);
    invoke(&mut code, 0x21ed, &[u32::MAX, 3]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 5);
    scene.advance_frame(100);
    assert_eq!(opacity(&scene, 3), 7);
    // Stopping a playing track writes its current key's value.
    run_all(&mut vm, &mut scene, 1);
    assert_eq!(opacity(&scene, 3), 77);
    run_all(&mut vm, &mut scene, 1);
    let track = scene
        .object(3)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .opacity_track
        .clone()
        .unwrap();
    assert_eq!(track.base, 0, "|i32::MIN| % 256 is 0");
    run_all(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    // Stopping a stopped track only frees it.
    run_all(&mut vm, &mut scene, 1);
    let draw = scene.object(3).unwrap().draw.as_ref().unwrap();
    assert!(draw.opacity_track.is_none());
    assert_eq!(draw.words[17], 77);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn opacity_commands_reject_bad_targets_and_keys() {
    for (draw, command, words) in [
        (false, 0x21eau16, vec![0, u32::MAX, 3]),
        (true, 0x21ea, vec![0, 5, 3]),
        (true, 0x21ea, vec![0, 768, 3]),
        (true, 0x21ea, vec![0, u32::MAX, 4]),
        (true, 0x21eb, vec![0, 0, 0, u32::MAX, 8, u32::MAX, 3]),
        (true, 0x21eb, vec![0, 0, 0, 8, 0, u32::MAX, 3]),
        (true, 0x21ec, vec![8, u32::MAX, 3]),
        (true, 0x21ee, vec![3]),
    ] {
        let mut code = vec![];
        invoke(&mut code, 0x2020, &[3]);
        if draw {
            invoke(&mut code, 0x2040, &[3]);
        }
        invoke(&mut code, command, &words);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        let mut scene = SceneHost::default();
        run_all(&mut vm, &mut scene, 1 + usize::from(draw));
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let bytes = vm.stack().len_bytes();
        assert!(scene.try_handle(&mut vm).is_err(), "{command:x} {words:?}");
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), Some(9));
    }
}

fn motion_setup(code: &mut Vec<u8>, slot: u32, base: [f32; 3]) {
    command(code, 0x2020, &[slot]);
    command(code, 0x2040, &[slot]);
    for (i, value) in base.into_iter().enumerate() {
        command(code, 0x2072 + i as u16, &[slot, u32::MAX, value.to_bits()]);
    }
    command(code, 0x21f9, &[slot, u32::MAX]);
}
fn motion_xyz(scene: &SceneHost, slot: u8) -> [f32; 3] {
    scene.object(slot).unwrap().draw.as_ref().unwrap().words[11..14]
        .try_into()
        .map(|v: [u32; 3]| v.map(f32::from_bits))
        .unwrap()
}

#[test]
fn world_motion_reset_query_stop_and_parent_child_isolation() {
    let mut code = vec![];
    motion_setup(&mut code, 255, [1.0, 2.0, 3.0]);
    command(&mut code, 0x2022, &[255, 767]);
    command(&mut code, 0x2050, &[255, 767]);
    command(&mut code, 0x21fd, &[255, 767]);
    command(&mut code, 0x21f9, &[255, 767]);
    command(&mut code, 0x21fd, &[255, 767]);
    command(
        &mut code,
        0x21fa,
        &[
            255,
            767,
            7,
            u32::MAX,
            0,
            400,
            0x7fc01234,
            0x80000000,
            0x7f800000,
        ],
    );
    command(&mut code, 0x21fb, &[255, 767, 7]);
    command(&mut code, 0x21fd, &[255, 767]);
    command(&mut code, 0x21fc, &[255, 767]);
    command(&mut code, 0x21f9, &[255, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 8);
    vm.state_mut().set_system_integer(6, 123).unwrap();
    run_all(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    run_all(&mut vm, &mut scene, 2);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    run_all(&mut vm, &mut scene, 3);
    assert_eq!(vm.state().system_integer(0), Some(7));
    let child = &scene.object(255).unwrap().children[&767];
    assert_eq!(
        &child.draw.as_ref().unwrap().words[11..14],
        &[0, 0, 0],
        "play does not snap"
    );
    run_all(&mut vm, &mut scene, 1);
    let child = &scene.object(255).unwrap().children[&767];
    assert_eq!(
        &child.draw.as_ref().unwrap().words[11..14],
        &[0x7fc01234, 0x80000000, 0x7f800000]
    );
    assert!(child.draw.as_ref().unwrap().world_motion.is_none());
    assert_eq!(motion_xyz(&scene, 255), [1.0, 2.0, 3.0]);
    run_all(&mut vm, &mut scene, 1);
    let motion = scene
        .object(255)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .world_motion
        .as_ref()
        .unwrap();
    assert!(motion.playback.is_none());
    assert_eq!(motion.keys, [cmvs_rs::scene::MotionKey::default(); 8]);
    assert_eq!(vm.state().system_integer(6), Some(123));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn world_motion_linear_relative_and_quadratic_ease_out_use_hundredths_and_clamped_time() {
    for (mode, halfway) in [
        (0, [10.0, 10.0, 0.0]),
        (0x8000, [15.0, 20.0, 15.0]),
        (1, [10.0, 5.0, -15.0]),
        (0x103e0038, [10.0, 19.38, 30.0]),
        (0x103e0039, [10.0, 20.62, 30.0]),
        (0x203e8038, [10.0, 20.0, 30.0]),
        (0x003e0038, [10.0, 20.0, 30.0]),
    ] {
        let mut code = vec![];
        motion_setup(&mut code, 3, [10.0, 20.0, 30.0]);
        command(
            &mut code,
            0x21fa,
            &[
                3,
                u32::MAX,
                0,
                u32::MAX,
                mode,
                400,
                10f32.to_bits(),
                0,
                (-30f32).to_bits(),
            ],
        );
        command(&mut code, 0x21fb, &[3, u32::MAX, 0]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run_all(&mut vm, &mut scene, 8);
        assert_eq!(motion_xyz(&scene, 3), [10.0, 20.0, 30.0]);
        scene.advance_frame(900);
        assert_eq!(motion_xyz(&scene, 3), halfway, "mode={mode:x}");
        scene.advance_frame(200);
        // Even relative interpolation snaps to the stored target, not base + target.
        assert_eq!(motion_xyz(&scene, 3), [10.0, 0.0, -30.0]);
        assert!(
            scene
                .object(3)
                .unwrap()
                .draw
                .as_ref()
                .unwrap()
                .world_motion
                .is_none()
        );
    }
}

#[test]
fn world_motion_crossing_a_key_snaps_until_the_next_frame_and_disposes_on_minus_two() {
    let mut code = vec![];
    motion_setup(&mut code, 1, [0.0; 3]);
    for (index, next_key, target) in [(0, 1, 10f32), (1, (-2i32) as u32, 20f32)] {
        command(
            &mut code,
            0x21fa,
            &[
                1,
                u32::MAX,
                index,
                next_key,
                0,
                100,
                target.to_bits(),
                target.to_bits(),
                target.to_bits(),
            ],
        );
    }
    command(&mut code, 0x21fb, &[1, u32::MAX, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 9);
    scene.advance_frame(150);
    assert_eq!(motion_xyz(&scene, 1), [10.0; 3]);
    let motion = scene
        .object(1)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .world_motion
        .as_ref()
        .unwrap();
    assert_eq!(motion.playback.as_ref().unwrap().current, 1);
    assert_eq!(motion.playback.as_ref().unwrap().elapsed, 50);
    scene.advance_frame(1);
    assert_eq!(motion_xyz(&scene, 1), [15.1; 3]);
    scene.advance_frame(49);
    assert!(scene.object(1).is_none());
}

#[test]
fn world_motion_without_a_track_is_a_noop_and_reset_discards_a_playing_track() {
    let mut code = vec![];
    command(&mut code, 0x2020, &[1]);
    command(&mut code, 0x2040, &[1]);
    command(&mut code, 0x21fa, &[1, u32::MAX, 99, 99, 99, 100, 0, 0, 0]);
    command(&mut code, 0x21fb, &[1, u32::MAX, 99]);
    command(&mut code, 0x21fc, &[1, u32::MAX]);
    command(&mut code, 0x21f9, &[1, u32::MAX]);
    command(
        &mut code,
        0x21fa,
        &[1, u32::MAX, 0, u32::MAX, 0, 100, 100f32.to_bits(), 0, 0],
    );
    command(&mut code, 0x21fb, &[1, u32::MAX, 0]);
    command(&mut code, 0x21f9, &[1, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_all(&mut vm, &mut scene, 5);
    assert!(
        scene
            .object(1)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .world_motion
            .is_none()
    );
    run_all(&mut vm, &mut scene, 3);
    scene.advance_frame(50);
    assert_eq!(motion_xyz(&scene, 1), [50.0, 0.0, 0.0]);
    run_all(&mut vm, &mut scene, 1);
    scene.advance_frame(100);
    assert_eq!(motion_xyz(&scene, 1), [50.0, 0.0, 0.0]);
    assert!(
        scene
            .object(1)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .world_motion
            .as_ref()
            .unwrap()
            .playback
            .is_none()
    );
}

#[test]
fn world_motion_invalid_targets_indices_modes_and_missing_args_are_transactional() {
    for (op, args) in [
        (0x21f9, vec![256, u32::MAX]),
        (0x21f9, vec![u32::MAX, u32::MAX]),
        (0x21f9, vec![2, u32::MAX]),
        (0x21f9, vec![1, 768]),
        (0x21f9, vec![1, 0]),
        (0x21fa, vec![1, u32::MAX, 8, 0, 0, 0, 0, 0, 0]),
        (0x21fa, vec![1, u32::MAX, 0, 8, 0, 0, 0, 0, 0]),
        (0x21fa, vec![1, u32::MAX, 0, 0, 0x8001, 0, 0, 0, 0]),
        (0x21fa, vec![1, u32::MAX, 0, 0, 16, 0, 0, 0, 0]),
        (0x21fb, vec![1, u32::MAX, u32::MAX]),
        (0x21fc, vec![1]),
        (0x21fd, vec![]),
    ] {
        let mut code = vec![];
        motion_setup(&mut code, 1, [1.0, 2.0, 3.0]);
        command(&mut code, op, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run_all(&mut vm, &mut scene, 6);
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let before = scene.object(1).unwrap().clone();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        for _ in 0..2 {
            assert!(scene.try_handle(&mut vm).is_err(), "{op:x} {args:?}");
            assert_eq!(scene.object(1), Some(&before));
            assert_eq!(vm.pending_system(), Some((code.len() - 2, op - 0x2000)));
            assert_eq!(vm.pc(), code.len() - 2);
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            for (i, arg) in args.iter().enumerate() {
                assert_eq!(vm.stack().peek_u32((i + 1) * 4).unwrap(), *arg);
            }
            assert_eq!(vm.state().system_integer(0), Some(123));
        }
    }
}

#[test]
fn scale_setter_preserves_raw_float_bits_other_properties_and_child_isolation() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![7]),
        (0x2040, vec![7]),
        (0x2045, vec![7, u32::MAX, 11, 22]),
        (0x21ea, vec![7, u32::MAX, 2]),
        (0x2022, vec![7, 767]),
        (0x2050, vec![7, 767]),
    ] {
        command(&mut code, op, &args);
    }
    let pairs = [
        [0x3fc00000, 0xbf800000],
        [0x80000000, 0x7fc01234],
        [0x7f800000, 0xff800000],
    ];
    for pair in pairs {
        command(&mut code, 0x2049, &[7, 0x80000000, pair[0], pair[1]]);
        command(&mut code, 0x2049, &[7, 767, pair[1], pair[0]]);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..6 {
        next(&mut vm, &mut scene).unwrap();
    }
    vm.state_mut().set_system_integer(0, 123).unwrap();
    let mut expected = scene.object(7).unwrap().clone();
    for pair in pairs {
        next(&mut vm, &mut scene).unwrap();
        expected.draw.as_mut().unwrap().words[18..20].copy_from_slice(&pair);
        assert_eq!(scene.object(7), Some(&expected));
        next(&mut vm, &mut scene).unwrap();
        expected
            .children
            .get_mut(&767)
            .unwrap()
            .draw
            .as_mut()
            .unwrap()
            .words[18..20]
            .copy_from_slice(&[pair[1], pair[0]]);
        assert_eq!(scene.object(7), Some(&expected));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
    }
}

#[test]
fn scale_setter_invalid_targets_or_short_stack_preserve_the_pending_command() {
    for args in [
        vec![1, u32::MAX, 0, 0],
        vec![2, u32::MAX, 0, 0],
        vec![256, u32::MAX, 0, 0],
        vec![u32::MAX, u32::MAX, 0, 0],
        vec![1, 768, 0, 0],
        vec![1, 0, 0, 0],
        vec![1, u32::MAX, 0],
    ] {
        let mut code = vec![];
        command(&mut code, 0x2020, &[1]);
        command(&mut code, 0x2049, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        next(&mut vm, &mut scene).unwrap();
        let old = scene.objects().clone();
        assert!(next(&mut vm, &mut scene).is_err());
        assert_eq!(scene.objects(), &old);
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x49));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

#[test]
fn solid_setter_preserves_other_properties_tracks_and_child_isolation() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![7]),
        (0x2040, vec![7]),
        (0x2044, vec![7, u32::MAX, 3, 4, 5, 6]),
        (0x2046, vec![7, u32::MAX, 9, 10]),
        (0x2045, vec![7, u32::MAX, 11, 22]),
        (0x21ea, vec![7, u32::MAX, 2]),
        (0x2022, vec![7, 767]),
        (0x2050, vec![7, 767]),
    ] {
        command(&mut code, op, &args);
    }
    let pairs = [
        [0x3fc00000, 0xbf800000],
        [0x80000000, 0x7fc01234],
        [0x7f800000, 0xff800000],
    ];
    for pair in pairs {
        command(
            &mut code,
            0x2043,
            &[7, 0x80000000, pair[0], pair[1], 0x12345678],
        );
        command(&mut code, 0x2043, &[7, 767, pair[1], pair[0], 0xabcdef]);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..8 {
        next(&mut vm, &mut scene).unwrap();
    }
    vm.state_mut().set_system_integer(0, 123).unwrap();
    let mut expected = scene.object(7).unwrap().clone();
    for pair in pairs {
        next(&mut vm, &mut scene).unwrap();
        let words = &mut expected.draw.as_mut().unwrap().words;
        words[2..4].fill(0);
        words[4..6].copy_from_slice(&pair);
        words[14] = 0xff345678;
        assert_eq!(scene.object(7), Some(&expected));
        next(&mut vm, &mut scene).unwrap();
        let words = &mut expected
            .children
            .get_mut(&767)
            .unwrap()
            .draw
            .as_mut()
            .unwrap()
            .words;
        words[2..4].fill(0);
        words[4..6].copy_from_slice(&[pair[1], pair[0]]);
        words[14] = 0xffabcdef;
        assert_eq!(scene.object(7), Some(&expected));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
    }
}

#[test]
fn solid_setter_invalid_targets_or_short_stack_preserve_the_pending_command() {
    for args in [
        vec![1, u32::MAX, 0, 0, 0],
        vec![2, u32::MAX, 0, 0, 0],
        vec![256, u32::MAX, 0, 0, 0],
        vec![u32::MAX, u32::MAX, 0, 0, 0],
        vec![1, 768, 0, 0, 0],
        vec![1, 0, 0, 0, 0],
        vec![1, u32::MAX, 0, 0],
    ] {
        let mut code = vec![];
        command(&mut code, 0x2020, &[1]);
        command(&mut code, 0x2043, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        next(&mut vm, &mut scene).unwrap();
        let old = scene.objects().clone();
        assert!(next(&mut vm, &mut scene).is_err());
        assert_eq!(scene.objects(), &old);
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x43));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

use cmvs_rs::{
    machine::{Machine, Step},
    message::MessageHost,
    ps3::Script,
};

fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
fn script(code: &[u8], text: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(value.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
fn run(vm: &mut Machine<'_>, messages: &mut MessageHost) {
    pending(vm);
    assert!(messages.try_handle(vm).unwrap());
}

fn stack_words(vm: &Machine<'_>) -> Vec<u32> {
    (0..vm.stack().len_bytes())
        .step_by(4)
        .map(|at| vm.stack().read_u32_at(at).unwrap())
        .collect()
}

#[test]
fn construction_owns_a_separate_atlas_and_preserves_constructor_defaults() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 10 + i).unwrap();
    }
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    for i in 1..7 {
        assert_eq!(vm.state().system_integer(i), Some(10 + i));
    }
    assert_eq!(messages.auxiliary_font_present(7), Some(true));
    assert!(messages.text(7).is_none());
    assert!(!messages.font_present(7));
    let group = messages.auxiliary_text(7).unwrap();
    for (at, value) in [
        (48, 25),
        (124, 0),
        (128, 1),
        (132, 0xffffff),
        (136, 0xffffff),
        (140, 2),
        (144, 0),
        (148, 0),
        (168, 1),
        (172, 1),
        (196, 148),
        (204, 12),
        (340, 32),
        (344, 1280),
        (348, 720),
        (352, 1280),
        (356, 720),
        (360, 1280),
        (364, 720),
        (368, 0),
        (404, 16),
        (408, 16),
        (412, 256),
        (1440, 512),
    ] {
        assert_eq!(group.word(at), Some(value), "offset {at}");
    }
    assert_eq!(group.word(40), Some(1280.0f32.to_bits()));
    assert_eq!(group.word(44), Some(720.0f32.to_bits()));
    assert_eq!(group.word(0), None); // native pointer is not a serialized address
    assert_eq!(group.atlas.texture_size, [512, 512]);
    assert_eq!(group.atlas.cell_side, 31);
    assert_eq!(group.atlas.initial_clear_tile.width(), 31);
    assert!(
        group
            .atlas
            .initial_clear_tile
            .pixels()
            .iter()
            .all(|b| *b == 0)
    );
    let draw = group.object.draw.as_ref().unwrap();
    assert_eq!(draw.words[0], 5);
    assert_eq!(&draw.words[2..6], &[0, 0, 4, 4]);
    assert_eq!(group.object.logical_size, [1280, 720]);
    assert!(group.object.children.is_empty());
    assert!(group.object.image.is_none()); // unknown atlas pixels are not fabricated
}

#[test]
fn capacity_boundaries_default_and_multiple_pages_follow_native_geometry() {
    for (capacity, side, height, normalized) in [
        (0, 1024, 1024, 512),
        (64, 512, 512, 64),
        (65, 1024, 1024, 65),
        (2000, 1024, 2048, 2000),
    ] {
        let mut code = vec![];
        call(&mut code, 0x2380, &[0, 0, 25, capacity]);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages);
        let atlas = &messages.auxiliary_text(0).unwrap().atlas;
        assert_eq!(atlas.capacity, normalized);
        assert_eq!(atlas.texture_size, [side, height]);
        assert_eq!(atlas.grid_side, side / 31);
        assert_eq!(atlas.cells_per_page, (side / 31) * (side / 31));
    }
}

#[test]
fn replacement_and_nonempty_destruction_preserve_font_and_ordinary_window() {
    let mut code = vec![];
    call(&mut code, 0x215d, &[0, 0, 20, 30, 100, 50, 1280, 720]);
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    call(&mut code, 0x2380, &[0, 0, 18, 65]);
    call(&mut code, 0x2382, &[0]);
    call(&mut code, 0x2382, &[0]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let window = messages.window(0).unwrap().clone();
    let text = messages.text(0).unwrap().clone();
    run(&mut vm, &mut messages);
    run(&mut vm, &mut messages);
    assert_eq!(messages.auxiliary_text(0).unwrap().atlas.cell_side, 24);
    assert_eq!(messages.auxiliary_text(0).unwrap().word(48), Some(18));
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 70 + i).unwrap();
    }
    for _ in 0..2 {
        run(&mut vm, &mut messages);
        assert!(messages.auxiliary_text(0).is_none());
        assert_eq!(messages.auxiliary_font_present(0), Some(true));
        assert_eq!(messages.window(0), Some(&window));
        assert_eq!(messages.text(0), Some(&text));
        assert!(messages.font_present(0));
        assert_eq!(vm.stack().len_bytes(), 0);
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(70 + i));
        }
    }
}

#[test]
fn invalid_creation_preserves_existing_owned_state_font_stack_pc_and_results() {
    for args in [
        [8, 0, 25, 32],
        [u32::MAX, 0, 25, 32],
        [2, u32::MAX, 25, 32],
        [2, 1, 25, 32],
        [2, 0, u32::MAX, 32],
        [2, 0, 1024, 32],
        [2, 0, 25, u32::MAX],
    ] {
        let mut code = vec![];
        call(&mut code, 0x2380, &[2, 0, 25, 32]);
        call(&mut code, 0x2380, &args);
        let script = script(&code, b"\0Named\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages);
        let atlas = messages.auxiliary_text(2).unwrap().atlas.clone();
        pending(&mut vm);
        let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        vm.state_mut().set_system_integer(0, 123).unwrap();
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err(), "{args:?}");
            assert_eq!(
                (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
                before
            );
            assert_eq!(vm.state().system_integer(0), Some(123));
            assert_eq!(messages.auxiliary_text(2).unwrap().atlas, atlas);
            assert_eq!(messages.auxiliary_font_present(2), Some(true));
        }
    }
}

#[test]
fn incomplete_creation_never_installs_a_font_or_object() {
    for count in 0..4 {
        let mut code = vec![];
        call(&mut code, 0x2380, &[0, 0, 25, 32][..count]);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        pending(&mut vm);
        let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            before
        );
        assert!(messages.auxiliary_text(0).is_none());
        assert_eq!(messages.auxiliary_font_present(0), Some(false));
    }
}

#[test]
fn positioning_updates_only_auxiliary_coordinates_and_preserves_results() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    let positions = [
        [0, 0],
        [(-180i32) as u32, 72],
        [16_777_217, (-16_777_217i32) as u32],
        [i32::MAX as u32, i32::MIN as u32],
    ];
    for [x, y] in positions {
        call(&mut code, 0x2383, &[7, x, y]);
    }
    call(&mut code, 0x2384, &[7]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let atlas = messages.auxiliary_text(7).unwrap().atlas.clone();
    let before_draw = messages
        .auxiliary_text(7)
        .unwrap()
        .object
        .draw
        .clone()
        .unwrap();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 80 + i).unwrap();
    }
    // IEEE-754 reference words include signed and beyond-exact-integer cases.
    let expected = [
        [0, 0],
        [0xc3340000, 0x42900000],
        [0x4b800000, 0xcb800000],
        [0x4f000000, 0xcf000000],
    ];
    for bits in expected {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 12);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert!(vm.pending_system().is_none());
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.word(8), Some(bits[0]));
        assert_eq!(group.word(12), Some(bits[1]));
        let mut draw = before_draw.clone();
        draw.words[11] = bits[0];
        draw.words[12] = bits[1];
        assert_eq!(group.object.draw.as_ref(), Some(&draw));
        assert_eq!(group.atlas, atlas);
        assert_eq!(group.word(40), Some(1280.0f32.to_bits()));
        assert_eq!(group.word(44), Some(720.0f32.to_bits()));
        assert_eq!(messages.auxiliary_font_present(7), Some(true));
        assert!(messages.auxiliary_text(0).is_none());
        assert!(messages.text(7).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(80 + i));
        }
    }
    pending(&mut vm);
    let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
    assert!(!messages.try_handle(&mut vm).unwrap());
    assert_eq!(
        (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
        before
    );
}

#[test]
fn invalid_auxiliary_setter_target_keeps_pending_command_and_owned_state() {
    for (opcode, slot, count) in [
        (0x2383, 3),
        (0x2385, 5),
        (0x2387, 4),
        (0x2389, 3),
        (0x238e, 2),
        (0x2392, 2),
        (0x238f, 2),
    ]
    .into_iter()
    .flat_map(|(op, count)| [0, 8, u32::MAX].map(|slot| (op, slot, count)))
    {
        let mut code = vec![];
        call(&mut code, 0x2380, &[7, 0, 25, 32]);
        let args = [slot, 180, 72, 1060, 720];
        call(&mut code, opcode, &args[..count]);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages);
        let group = messages.auxiliary_text(7).unwrap().clone();
        pending(&mut vm);
        let before = (vm.pc(), stack_words(&vm), vm.pending_system());
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err());
            assert_eq!((vm.pc(), stack_words(&vm), vm.pending_system()), before);
            let after = messages.auxiliary_text(7).unwrap();
            assert_eq!(after.word(8), group.word(8));
            assert_eq!(after.word(12), group.word(12));
            for at in [16, 20, 24, 28, 48, 52, 56, 132, 136, 148, 176, 180] {
                assert_eq!(after.word(at), group.word(at));
            }
            assert_eq!(after.object, group.object);
            assert_eq!(after.atlas, group.atlas);
            assert_eq!(vm.state().system_integer(0), Some(1));
        }
    }
}

#[test]
fn incomplete_auxiliary_setter_arguments_leave_group_and_stack_unchanged() {
    for (opcode, count) in [
        (0x2383, 3),
        (0x2385, 5),
        (0x2387, 4),
        (0x2389, 3),
        (0x238e, 2),
        (0x2392, 2),
        (0x238f, 2),
    ]
    .into_iter()
    .flat_map(|(op, count)| (0..count).map(move |count| (op, count)))
    {
        let mut code = vec![];
        call(&mut code, 0x2380, &[0, 0, 25, 32]);
        call(&mut code, opcode, &[0, 180, 72, 1060, 720][..count]);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages);
        let group = messages.auxiliary_text(0).unwrap().clone();
        pending(&mut vm);
        let before = (vm.pc(), stack_words(&vm), vm.pending_system());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!((vm.pc(), stack_words(&vm), vm.pending_system()), before);
        let after = messages.auxiliary_text(0).unwrap();
        assert_eq!(after.word(8), group.word(8));
        assert_eq!(after.word(12), group.word(12));
        for at in [16, 20, 24, 28, 48, 52, 56, 132, 136, 148, 176, 180] {
            assert_eq!(after.word(at), group.word(at));
        }
        assert_eq!(after.object, group.object);
        assert_eq!(after.atlas, group.atlas);
    }
}

#[test]
fn colors_update_independently_only_for_zero_high_bytes() {
    let cases = [
        ([0x123456, 0xabcdef], [0x123456, 0xabcdef]),
        ([0x01000000, 0], [0x123456, 0]),
        ([0xffffff, 0x80000000], [0xffffff, 0]),
        ([u32::MAX, 0xff123456], [0xffffff, 0]),
        ([0, 0xffffff], [0, 0xffffff]),
    ];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for (colors, _) in cases {
        call(&mut code, 0x2389, &[7, colors[0], colors[1]]);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(7).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 110 + i).unwrap();
    }
    for (_, expected) in cases {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 12);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!([group.word(132), group.word(136)], expected.map(Some));
        for at in (0..1476).step_by(4).filter(|at| ![132, 136].contains(at)) {
            assert_eq!(group.word(at), before.word(at), "offset {at}");
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(messages.auxiliary_font_present(7), Some(true));
        assert!(messages.text(7).is_none());
        assert!(messages.auxiliary_text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(110 + i));
        }
    }
}

#[test]
fn text_parameters_keep_sentinel_fields_and_never_reallocate_the_atlas() {
    let cases = [
        ([0, 30, 4, 5], [30, 0x40800000, 0x40a00000]),
        (
            [0, 0, u32::MAX, (-2i32) as u32],
            [30, 0x40800000, 0x40a00000],
        ),
        ([0, u32::MAX, 0, u32::MAX], [30, 0, 0x40a00000]),
        ([0, i32::MIN as u32, u32::MAX, 0], [30, 0, 0]),
        (
            [0, i32::MAX as u32, i32::MAX as u32, 16_777_217],
            [i32::MAX as u32, 0x4f000000, 0x4b800000],
        ),
    ];
    let mut code = vec![];
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    for (args, _) in cases {
        call(&mut code, 0x2387, &args);
    }
    call(&mut code, 0x238a, &[0]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(0).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 100 + i).unwrap();
    }
    for (_, words) in cases {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 16);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert!(vm.pending_system().is_none());
        let group = messages.auxiliary_text(0).unwrap();
        for (offset, value) in [48, 52, 56].into_iter().zip(words) {
            assert_eq!(group.word(offset), Some(value));
        }
        for at in [8, 12, 16, 20, 24, 28, 124, 132, 136, 140, 176, 180, 340] {
            assert_eq!(group.word(at), before.word(at));
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(group.atlas.cell_side, 31);
        assert_eq!(messages.auxiliary_font_present(0), Some(true));
        assert!(messages.auxiliary_text(1).is_none());
        assert!(messages.text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(100 + i));
        }
    }
    pending(&mut vm);
    let before = (vm.pc(), stack_words(&vm), vm.pending_system());
    assert!(!messages.try_handle(&mut vm).unwrap());
    assert_eq!((vm.pc(), stack_words(&vm), vm.pending_system()), before);
}

#[test]
fn layout_stores_signed_float_fields_without_moving_or_resizing_the_atlas() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    call(&mut code, 0x2383, &[0, 180, 72]);
    call(&mut code, 0x2385, &[0, 180, 0, 1060, 720]);
    call(
        &mut code,
        0x2385,
        &[
            0,
            (-180i32) as u32,
            16_777_217,
            i32::MIN as u32,
            i32::MAX as u32,
        ],
    );
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(0).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 90 + i).unwrap();
    }
    // Reference words retain signed values and f32 rounding without clamping.
    for bits in [
        [0x43340000, 0, 0x44848000, 0x44340000],
        [0xc3340000, 0x4b800000, 0xcf000000, 0x4f000000],
    ] {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 20);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert!(vm.pending_system().is_none());
        let group = messages.auxiliary_text(0).unwrap();
        for (at, value) in [
            (16, bits[0]),
            (20, bits[1]),
            (24, bits[2]),
            (28, bits[3]),
            (176, bits[0]),
            (180, bits[1]),
        ] {
            assert_eq!(group.word(at), Some(value));
        }
        for at in [8, 12, 40, 44, 344, 348, 352, 356, 360, 364] {
            assert_eq!(group.word(at), before.word(at));
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(messages.auxiliary_font_present(0), Some(true));
        assert!(messages.auxiliary_text(1).is_none());
        assert!(messages.text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(90 + i));
        }
    }
}

#[test]
fn field_148_preserves_all_parameter_bits_without_affecting_other_state() {
    let values = [1, 0, 0x01000000, i32::MAX as u32, i32::MIN as u32, u32::MAX];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for value in values {
        call(&mut code, 0x238e, &[7, value]);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(7).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 120 + i).unwrap();
    }
    for value in values {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 8);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.word(148), Some(value));
        for at in (0..1476).step_by(4).filter(|at| *at != 148) {
            assert_eq!(group.word(at), before.word(at), "offset {at}");
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(messages.auxiliary_font_present(7), Some(true));
        assert!(messages.text(7).is_none());
        assert!(messages.auxiliary_text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(120 + i));
        }
    }
}
#[test]
fn field_140_preserves_all_parameter_bits_without_affecting_other_state() {
    let values = [1, 0, 0x01000000, i32::MAX as u32, i32::MIN as u32, u32::MAX];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for value in values {
        call(&mut code, 0x2392, &[7, value]);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(7).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 120 + i).unwrap();
    }
    for value in values {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 8);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.word(140), Some(value));
        for at in (0..1476).step_by(4).filter(|at| *at != 140) {
            assert_eq!(group.word(at), before.word(at), "offset {at}");
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(messages.auxiliary_font_present(7), Some(true));
        assert!(messages.text(7).is_none());
        assert!(messages.auxiliary_text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(120 + i));
        }
    }
}
#[test]
fn layer_updates_owned_draw_and_preserves_other_state() {
    let values = [1, 0, 0x01000000, i32::MAX as u32, i32::MIN as u32, u32::MAX];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for value in values {
        call(&mut code, 0x238f, &[7, value]);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(7).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 120 + i).unwrap();
    }
    for value in values {
        pending(&mut vm);
        let pc = vm.pc();
        assert_eq!(vm.stack().len_bytes(), 8);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.word(172), Some(value));
        for at in (0..1476).step_by(4).filter(|at| *at != 172) {
            assert_eq!(group.word(at), before.word(at), "offset {at}");
        }
        let mut expected_object = before.object.clone();
        expected_object.draw.as_mut().unwrap().words[10] = value;
        assert_eq!(group.object, expected_object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(messages.auxiliary_font_present(7), Some(true));
        assert!(messages.text(7).is_none());
        assert!(messages.auxiliary_text(0).is_none());
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(120 + i));
        }
    }
}

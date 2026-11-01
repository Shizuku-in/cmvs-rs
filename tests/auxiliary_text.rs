#[path = "support/shared_windows.rs"]
mod shared_windows;
use cmvs_rs::{
    machine::{Machine, Step},
    message::MessageHost,
    ps3::Script,
};
use shared_windows::SharedWindows;

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
    assert!(messages.shared_message(vm).unwrap());
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
            assert!(messages.shared_message(&mut vm).is_err(), "{args:?}");
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
        assert!(messages.shared_message(&mut vm).is_err());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
    assert!(!messages.shared_message(&mut vm).unwrap());
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
        (0x2396, 5),
        (0x2386, 1),
        (0x2387, 4),
        (0x2389, 3),
        (0x238e, 2),
        (0x2392, 2),
        (0x2393, 2),
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
            assert!(messages.shared_message(&mut vm).is_err());
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
        (0x2396, 5),
        (0x2386, 1),
        (0x2387, 4),
        (0x2389, 3),
        (0x238e, 2),
        (0x2392, 2),
        (0x2393, 2),
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
        assert!(messages.shared_message(&mut vm).is_err());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
    assert!(!messages.shared_message(&mut vm).unwrap());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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
        assert!(messages.shared_message(&mut vm).unwrap());
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

#[test]
fn clear_empty_content_preserves_atlas_and_configured_object() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    call(&mut code, 0x2385, &[7, 180, 72, 1060, 720]);
    call(&mut code, 0x2383, &[7, 31, 52]);
    call(&mut code, 0x238f, &[7, 10]);
    call(&mut code, 0x2386, &[7]);
    call(&mut code, 0x2386, &[7]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    for _ in 0..4 {
        run(&mut vm, &mut messages);
    }
    let before = messages.auxiliary_text(7).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 120 + i).unwrap();
    }
    for _ in 0..2 {
        run(&mut vm, &mut messages);
        let group = messages.auxiliary_text(7).unwrap();
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        for at in (0..1476).step_by(4) {
            assert_eq!(group.word(at), before.word(at));
        }
        assert_eq!(group.object, before.object);
        assert_eq!(group.atlas, before.atlas);
        assert_eq!(group.word(176), Some(180f32.to_bits()));
        assert_eq!(group.word(180), Some(72f32.to_bits()));
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(120 + i));
        }
    }
}
#[test]
fn field_144_preserves_all_parameter_bits_without_affecting_other_state() {
    let values = [1, 0, 0x01000000, i32::MAX as u32, i32::MIN as u32, u32::MAX];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for value in values {
        call(&mut code, 0x2393, &[7, value]);
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
        assert!(messages.shared_message(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.word(144), Some(value));
        for at in (0..1476).step_by(4).filter(|at| *at != 144) {
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
fn bounds_convert_signed_parameters_without_affecting_other_state() {
    let values = [1, 0, 0x01000000, i32::MAX as u32, i32::MIN as u32, u32::MAX];
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    for value in values {
        call(
            &mut code,
            0x2396,
            &[7, value, value.wrapping_add(1), 1280, 720],
        );
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
        assert_eq!(vm.stack().len_bytes(), 20);
        assert!(messages.shared_message(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        let group = messages.auxiliary_text(7).unwrap();
        for (at, v) in [32, 36, 40, 44]
            .into_iter()
            .zip([value, value.wrapping_add(1), 1280, 720])
        {
            assert_eq!(group.word(at), Some(((v as i32) as f32).to_bits()));
        }
        for at in (0..1476)
            .step_by(4)
            .filter(|at| !matches!(*at, 32 | 36 | 40 | 44))
        {
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
fn bound_auxiliary_position_moves_scene_with_float_roundtrip_and_detaches() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![11]),
        (0x2040, vec![11]),
        (0x2380, vec![0, 0, 25, 32]),
        (0x239d, vec![0, 11]),
        (0x2383, vec![0, 16777217, i32::MAX as u32]),
        (0x239d, vec![0, u32::MAX]),
        (0x2383, vec![0, 180, 72]),
    ] {
        call(&mut code, op, &args);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    for _ in 0..4 {
        pending(&mut vm);
        assert!(
            scene.try_handle(&mut vm).unwrap()
                || messages
                    .shared_auxiliary_scene(&mut vm, &mut scene)
                    .unwrap()
                || messages.shared_message(&mut vm).unwrap()
        );
    }
    assert_eq!(
        messages.auxiliary_text(0).unwrap().associated_scene,
        Some(11)
    );
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 99 + i).unwrap();
    }
    pending(&mut vm);
    assert!(messages.shared_message(&mut vm).is_err());
    assert!(
        messages
            .shared_auxiliary_scene(&mut vm, &mut scene)
            .unwrap()
    );
    assert_eq!(
        &scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
        &[16777216, i32::MIN as u32]
    );
    for _ in 0..2 {
        pending(&mut vm);
        assert!(
            messages
                .shared_auxiliary_scene(&mut vm, &mut scene)
                .unwrap()
                || messages.shared_message(&mut vm).unwrap()
        );
    }
    assert_eq!(messages.auxiliary_text(0).unwrap().associated_scene, None);
    assert_eq!(
        messages.auxiliary_text(0).unwrap().word(8),
        Some(180f32.to_bits())
    );
    assert_eq!(
        &scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
        &[16777216, i32::MIN as u32]
    );
    for i in 0..7 {
        assert_eq!(vm.state().system_integer(i), Some(99 + i));
    }
}

#[test]
fn auxiliary_association_errors_keep_target_and_pending_command() {
    for args in [vec![0, 256], vec![0, 11], vec![0], vec![8, 0]] {
        let mut code = vec![];
        call(&mut code, 0x2380, &[0, 0, 25, 32]);
        call(&mut code, 0x239d, &args);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        run(&mut vm, &mut messages);
        let before = messages.auxiliary_text(0).unwrap().clone();
        pending(&mut vm);
        let p = vm.pending_system();
        let sp = vm.stack().len_bytes();
        assert!(
            messages
                .shared_auxiliary_scene(&mut vm, &mut scene)
                .is_err()
        );
        assert_eq!(vm.pending_system(), p);
        assert_eq!(vm.stack().len_bytes(), sp);
        assert_eq!(messages.auxiliary_text(0).unwrap().object, before.object);
        assert_eq!(messages.auxiliary_text(0).unwrap().associated_scene, None);
    }
}

#[test]
fn auxiliary_position_bounds_keep_complete_signed_words_and_other_configuration() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[7, 0, 25, 32]);
    call(&mut code, 0x239a, &[7, 180, 180, i32::MIN as u32, 72]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages);
    let before = messages.auxiliary_text(7).unwrap().clone();
    run(&mut vm, &mut messages);
    let group = messages.auxiliary_text(7).unwrap();
    for (offset, value) in [152, 156, 160, 164]
        .into_iter()
        .zip([180, 180, i32::MIN as u32, 72])
    {
        assert_eq!(group.word(offset), Some(value));
    }
    for at in (0..1476)
        .step_by(4)
        .filter(|at| !matches!(*at, 152 | 156 | 160 | 164))
    {
        assert_eq!(group.word(at), before.word(at));
    }
    assert_eq!(group.object, before.object);
    assert_eq!(group.atlas, before.atlas);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn auxiliary_position_queries_preserve_result_zero_and_clamp_inverted_bounds_sequentially() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    call(&mut code, 0x2383, &[0, i32::MAX as u32, (-17i32) as u32]);
    call(&mut code, 0x2398, &[0]);
    call(&mut code, 0x239a, &[0, 100, 50, (-100i32) as u32, 100]);
    call(&mut code, 0x239e, &[0, 0, (-200i32) as u32]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    for _ in 0..2 {
        run(&mut vm, &mut messages);
    }
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 120 + i).unwrap();
    }
    run(&mut vm, &mut messages);
    assert_eq!(vm.state().system_integer(1), Some(i32::MIN as u32));
    assert_eq!(vm.state().system_integer(2), Some((-17i32) as u32));
    run(&mut vm, &mut messages);
    run(&mut vm, &mut messages);
    assert_eq!(vm.state().system_integer(1), Some(50));
    assert_eq!(vm.state().system_integer(2), Some((-100i32) as u32));
    for i in [0, 3, 4, 5, 6] {
        assert_eq!(vm.state().system_integer(i), Some(120 + i));
    }
    assert_eq!(
        messages.auxiliary_text(0).unwrap().word(8),
        Some((i32::MAX as f32).to_bits())
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn auxiliary_motion_linear_sine_wrap_clock_and_bound_scene_advance_only_per_frame() {
    for (opcode, args, half) in [
        (
            0x239c,
            vec![7, 100, (-100i32) as u32, 100],
            [50, (-50i32) as u32],
        ),
        (
            0x239f,
            vec![7, 1, 100, (-100i32) as u32, 77, 100],
            [70, (-70i32) as u32],
        ),
    ] {
        let mut code = vec![];
        for (op, args) in [
            (0x2020, vec![11]),
            (0x2040, vec![11]),
            (0x2380, vec![7, 0, 25, 32]),
            (0x239d, vec![7, 11]),
            (
                0x239a,
                vec![7, (-1000i32) as u32, 1000, (-1000i32) as u32, 1000],
            ),
        ] {
            call(&mut code, op, &args);
        }
        call(&mut code, opcode, &args);
        call(&mut code, 0x239b, &[7]);
        let script = script(&code, b"\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        messages
            .advance_auxiliary_motion(u32::MAX, &mut scene)
            .unwrap();
        for _ in 0..6 {
            pending(&mut vm);
            assert!(
                scene.try_handle(&mut vm).unwrap()
                    || messages
                        .shared_auxiliary_scene(&mut vm, &mut scene)
                        .unwrap()
                    || messages.shared_message(&mut vm).unwrap()
            );
        }
        assert_eq!(
            messages.auxiliary_text(7).unwrap().word(400),
            Some(u32::MAX)
        );
        assert_eq!(messages.auxiliary_text(7).unwrap().word(368), Some(1));
        assert_eq!(messages.auxiliary_text(7).unwrap().word(8), Some(0));
        messages.advance_auxiliary_motion(50, &mut scene).unwrap();
        assert_eq!(
            &scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
            &half
        );
        run(&mut vm, &mut messages);
        assert_eq!(vm.state().system_integer(0), Some(1));
        messages.advance_auxiliary_motion(50, &mut scene).unwrap();
        assert_eq!(
            &scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
            &[100, (-100i32) as u32]
        );
        assert_eq!(messages.auxiliary_text(7).unwrap().word(368), Some(0));
        messages.advance_auxiliary_motion(1000, &mut scene).unwrap();
        assert_eq!(
            &scene.object(11).unwrap().draw.as_ref().unwrap().words[8..10],
            &[100, (-100i32) as u32]
        );
    }
}

#[test]
fn auxiliary_motion_clamps_stops_and_rejects_unknown_mode_without_changing_command() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    call(&mut code, 0x239a, &[0, 0, 10, 0, 0]);
    call(&mut code, 0x239c, &[0, 100, 0, 100]);
    call(&mut code, 0x239c, &[0, 1, 0, 100]);
    call(&mut code, 0x239f, &[0, 2, 0, 0, 0, 100]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    for _ in 0..3 {
        run(&mut vm, &mut messages);
    }
    messages.advance_auxiliary_motion(50, &mut scene).unwrap();
    assert_eq!(
        messages.auxiliary_text(0).unwrap().word(8),
        Some(10f32.to_bits())
    );
    assert_eq!(messages.auxiliary_text(0).unwrap().word(368), Some(0));
    let before = messages.auxiliary_text(0).unwrap().clone();
    run(&mut vm, &mut messages);
    assert_eq!(
        messages.auxiliary_text(0).unwrap().word(400),
        before.word(400)
    );
    pending(&mut vm);
    let pc = vm.pc();
    let sp = vm.stack().len_bytes();
    assert!(messages.shared_message(&mut vm).is_err());
    assert_eq!(vm.pc(), pc);
    assert_eq!(vm.stack().len_bytes(), sp);
    assert_eq!(messages.auxiliary_text(0).unwrap().word(368), Some(0));
}

#[test]
fn inverted_motion_bounds_stop_even_when_sequential_clamping_restores_same_position() {
    let mut code = vec![];
    call(&mut code, 0x2380, &[0, 0, 25, 32]);
    call(&mut code, 0x2383, &[0, 50, 0]);
    call(&mut code, 0x239a, &[0, 100, 50, 0, 0]);
    call(&mut code, 0x239c, &[0, 0, 0, 100]);
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    for _ in 0..4 {
        run(&mut vm, &mut messages);
    }
    messages
        .advance_auxiliary_motion(10, &mut cmvs_rs::scene::SceneHost::default())
        .unwrap();
    assert_eq!(
        messages.auxiliary_text(0).unwrap().word(8),
        Some(50f32.to_bits())
    );
    assert_eq!(messages.auxiliary_text(0).unwrap().word(368), Some(0));
}

/// Text segment offsets: 0 is the empty font name, 1 the text under test.
fn content_script(calls: &[(u16, &[u32])], text: &[u8]) -> Script {
    let mut code = vec![];
    for (op, args) in calls {
        call(&mut code, *op, args);
    }
    let mut segment = b"\0".to_vec();
    segment.extend(text);
    segment.push(0);
    script(&code, &segment)
}
fn run_calls(vm: &mut Machine<'_>, messages: &mut MessageHost, count: usize) {
    for _ in 0..count {
        run(vm, messages);
    }
}

#[test]
fn proportional_auxiliary_text_uses_advances_for_single_and_double_byte_glyphs() {
    #[derive(Debug)]
    struct Metrics;
    impl cmvs_rs::message::GlyphMetrics for Metrics {
        fn ink_width(&self, _: &[u8], _: u16, _: u32) -> Result<u32, &'static str> {
            Ok(99)
        }
        fn advance_width(&self, _: &[u8], code: u16, _: u32, _: bool) -> Result<u32, &'static str> {
            Ok(if code == 0x82a0 { 13 } else { 3 })
        }
    }
    let script = content_script(
        &[
            (0x2380, &[7, 0, 24, 64]),
            (0x2385, &[7, 0, 0, 100, 100]),
            (0x23ac, &[1]),
            (0x238b, &[7, 1]),
        ],
        b"W\x82\xa0",
    );
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    messages.set_record_output(true);
    messages.set_glyph_metrics(Box::new(Metrics));
    for _ in 0..4 {
        pending(&mut vm);
        let mut input = cmvs_rs::input::ActionInput::default();
        let profile = cmvs_rs::profile::Registry::builtin()
            .profile("cmvs-20171114")
            .unwrap();
        let command = vm.pending_system().unwrap().1;
        let mut hosts = cmvs_rs::dispatch::Hosts::default();
        if cmvs_rs::dispatch::uses_message_state(command) {
            hosts.messages = Some(&mut messages);
        } else {
            hosts.text = Some(cmvs_rs::dispatch::TextHosts {
                messages: &mut messages,
                input: &mut input,
                flags: Default::default(),
            });
        }
        match cmvs_rs::dispatch::run(profile.semantics(command), &mut vm, &mut hosts).unwrap() {
            Some(cmvs_rs::dispatch::Flow::Done) => {}
            None => assert!(messages.shared_message(&mut vm).unwrap()),
            other => panic!("unexpected dispatch {other:?}"),
        }
    }
    let group = messages.auxiliary_text(7).unwrap();
    assert_eq!(
        group.glyphs.iter().map(|g| g.position).collect::<Vec<_>>(),
        [[0, 0], [3, 0]]
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}
/// Group 7 with 24-pixel glyphs: area at (100, 50), 110 wide, glyph spacing 2, line spacing 6,
/// so the wrap test fires once the pen reaches 210 - 26 = 184.
const AREA: [(u16, &[u32]); 3] = [
    (0x2380, &[7, 0, 24, 64]),
    (0x2385, &[7, 100, 50, 110, 200]),
    (0x2387, &[7, 0, 2, 6]),
];
/// あいうえ。か
const KANA: &[u8] = b"\x82\xa0\x82\xa2\x82\xa4\x82\xa6\x81\x42\x82\xa9";

#[test]
fn text_layout_wraps_keeps_closing_marks_and_shifts_voiced_marks() {
    // あいうえ。か \n ゛あ \c\w5 {漢/かん}
    let mut text = KANA.to_vec();
    text.extend(b"\\n\x81\x4a\x82\xa0\\c\\w5{\x8a\xbf/\x82\xa9\x82\xf1}");
    let mut calls = AREA.to_vec();
    calls.extend([
        (0x2389, &[7, 0x123456, 0x654321][..]),
        (0x238e, &[7, 2]),
        (0x2393, &[7, 0x111111]),
        (0x238b, &[7, 1]),
    ]);
    let script = content_script(&calls, &text);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run_calls(&mut vm, &mut messages, 6);
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 10 + i).unwrap();
    }
    // Text output follows the explicit recording policy of 0x2152.
    pending(&mut vm);
    assert!(!messages.shared_message(&mut vm).unwrap());
    messages.set_record_output(true);
    assert!(messages.shared_message(&mut vm).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
    for i in 0..7 {
        assert_eq!(vm.state().system_integer(i), Some(10 + i));
    }
    let group = messages.auxiliary_text(7).unwrap();
    let placed: Vec<(u16, [i32; 2])> = group.glyphs.iter().map(|g| (g.code, g.position)).collect();
    assert_eq!(
        placed,
        [
            (0x82a0, [100, 50]),
            (0x82a2, [126, 50]),
            (0x82a4, [152, 50]),
            (0x82a6, [178, 50]),
            (0x8142, [204, 50]), // 。 stays on the full line
            (0x82a9, [100, 80]),
            (0x814a, [96, 110]), // ゛ a fifth of the size to the left
            (0x82a0, [109, 110]),
            (0x8abf, [135, 110]), // ruby base only
        ]
    );
    for glyph in &group.glyphs {
        assert_eq!(glyph.size, 24);
        assert_eq!(glyph.colors, [0x123456, 0x654321]);
        assert_eq!(glyph.gradient, 2);
        assert_eq!(glyph.outline, [2, 0x111111]);
        assert_eq!(glyph.cell, 0);
    }
    assert_eq!(group.attached_glyphs().count(), 0);
    let x = ((100.0f32 + ((12.0f32 - 24.0 / 5.0) + 2.0)) + 26.0) + 26.0;
    assert_eq!(group.word(176), Some(x.to_bits()));
    assert_eq!(group.word(180), Some(110.0f32.to_bits()));
}

#[test]
fn text_layout_errors_keep_the_group_pen_and_pending_command() {
    let thirty_three = b"\x82\xa0".repeat(33);
    for (layout, text) in [
        (&[7, 100, 50, 110, 200][..], &b"\x82\xa0A"[..]), // uninitialized half-width advance
        (&[7, 100, 50, 110, 200], b"\\x"),                // loops forever in the original
        (&[7, 100, 50, 110, 200], b"\\"),
        (&[7, 100, 50, 110, 200], b"\\w"),
        (&[7, 100, 50, 110, 200], b"{\x8a\xbf/\x82\xa9"),
        (&[7, 0, 0, 0, 0], &thirty_three), // every glyph wraps: 32 cleared line flags
    ] {
        let script = content_script(
            &[
                (0x2380, &[7, 0, 24, 64]),
                (0x2385, layout),
                (0x238b, &[7, 1]),
            ],
            text,
        );
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        run_calls(&mut vm, &mut messages, 2);
        let pen = messages.auxiliary_text(7).unwrap().word(176);
        pending(&mut vm);
        let sp = vm.stack().len_bytes();
        assert!(messages.shared_message(&mut vm).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x38b));
        assert_eq!(vm.stack().len_bytes(), sp);
        let group = messages.auxiliary_text(7).unwrap();
        assert!(group.glyphs.is_empty());
        assert_eq!(group.word(176), pen);
    }
}

#[test]
fn measurement_counts_lines_and_byte_columns_from_the_pen() {
    for (text, expected) in [
        // あいうえ。 fit (。 stays), か wraps, \n starts line 3 with two half-width bytes.
        ([KANA, b"\\nAB"].concat(), [3, 10, 2]),
        // An unknown escape ends the measurement.
        (b"\x82\xa0\\q\x82\xa2".to_vec(), [1, 2, 2]),
        // Ruby base characters count, ruby text does not.
        (b"{\x8a\xbf\x8a\xbf/xyz}".to_vec(), [1, 4, 4]),
    ] {
        let mut calls = AREA.to_vec();
        calls.push((0x2390, &[7, 1]));
        let script = content_script(&calls, &text);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run_calls(&mut vm, &mut messages, 3);
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 70 + i).unwrap();
        }
        run(&mut vm, &mut messages);
        assert_eq!(vm.stack().len_bytes(), 0);
        for (i, value) in [(0, expected[0]), (1, expected[1]), (4, expected[2])] {
            assert_eq!(vm.state().system_integer(i), Some(value));
        }
        for i in [2, 3, 5, 6] {
            assert_eq!(vm.state().system_integer(i), Some(70 + i));
        }
        let group = messages.auxiliary_text(7).unwrap();
        assert!(group.glyphs.is_empty());
        assert_eq!(group.word(176), Some(100.0f32.to_bits()));
    }
}

#[test]
fn pen_and_motion_stop_change_only_their_fields() {
    let script = content_script(
        &[
            (0x2380, &[7, 0, 24, 64]),
            (0x238c, &[7, (-5i32) as u32, 40]),
            (
                0x239a,
                &[7, (-1000i32) as u32, 1000, (-1000i32) as u32, 1000],
            ),
            (0x239c, &[7, 100, 0, 100]),
            (0x23a0, &[7]),
        ],
        b"",
    );
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    run_calls(&mut vm, &mut messages, 2);
    let group = messages.auxiliary_text(7).unwrap();
    assert_eq!(group.word(176), Some((-5.0f32).to_bits()));
    assert_eq!(group.word(180), Some(40.0f32.to_bits()));
    run_calls(&mut vm, &mut messages, 2);
    assert_eq!(messages.auxiliary_text(7).unwrap().word(368), Some(1));
    run(&mut vm, &mut messages);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(messages.auxiliary_text(7).unwrap().word(368), Some(0));
    messages.advance_auxiliary_motion(50, &mut scene).unwrap();
    assert_eq!(messages.auxiliary_text(7).unwrap().word(8), Some(0));
}

fn attached(messages: &MessageHost) -> Vec<(u16, u16, u32)> {
    messages
        .auxiliary_text(7)
        .unwrap()
        .attached_glyphs()
        .map(|g| (g.code, g.cell, g.layer))
        .collect()
}

#[test]
fn glyphs_attach_inside_the_bounds_and_cells_leak_after_a_clear() {
    let up: &[u32] = &[7, 0, (-40i32) as u32];
    let up2: &[u32] = &[7, 0, (-80i32) as u32];
    let mut calls = AREA.to_vec();
    calls.extend([
        (0x2396, &[7, 0, 0, 300, 60][..]),
        (0x238f, &[7, 9]),
        (0x238b, &[7, 1]),
        (0x238f, &[7, 3]),
        (0x2383, up),
        (0x2383, up2),
        (0x2386, &[7]),
        (0x2383, &[7, 0, 0]),
        (0x238b, &[7, 1]),
        (0x2383, up),
    ]);
    let script = content_script(&calls, KANA);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    messages.set_record_output(true);
    let mut scene = cmvs_rs::scene::SceneHost::default();
    let mut frame = |messages: &mut MessageHost| {
        messages.advance_auxiliary_motion(16, &mut scene).unwrap();
        attached(messages)
    };
    run_calls(&mut vm, &mut messages, 6);
    // Line 0 (y 50) is inside; line 1 (y 80, padded top 78) lies below the bottom 60.
    let line0: Vec<(u16, u16, u32)> = [0x82a0, 0x82a2, 0x82a4, 0x82a6, 0x8142]
        .into_iter()
        .zip(1..)
        .map(|(code, cell)| (code, cell, 9))
        .collect();
    assert_eq!(frame(&mut messages), line0);
    // Attached glyphs keep the layer they were attached with.
    run_calls(&mut vm, &mut messages, 2);
    let mut both = line0.clone();
    both.push((0x82a9, 6, 3));
    assert_eq!(frame(&mut messages), both);
    // At y -80 line 0 ends above the top (padded bottom -4) and releases its cells.
    run(&mut vm, &mut messages);
    assert_eq!(frame(&mut messages), [(0x82a9, 6, 3)]);
    // 0x2386 drops the records but not cell 6.
    run_calls(&mut vm, &mut messages, 3);
    let group = messages.auxiliary_text(7).unwrap();
    assert_eq!(group.glyphs.len(), 6);
    assert_eq!(group.word(176), Some(126.0f32.to_bits()));
    let line0: Vec<_> = line0
        .iter()
        .map(|&(code, cell, _)| (code, cell, 3))
        .collect();
    assert_eq!(frame(&mut messages), line0);
    run(&mut vm, &mut messages);
    let mut both = line0;
    both.push((0x82a9, 7, 3));
    assert_eq!(frame(&mut messages), both);
}

#[test]
fn exhausted_cells_leave_glyphs_detached_and_cells_must_fit_the_atlas() {
    let page = (0x2385, &[7, 0, 0, 1280, 720][..]);
    // 4-pixel glyphs: 1024-pixel pages hold 10404 cells, so the 1023-cell table runs out.
    let script = content_script(
        &[(0x2380, &[7, 0, 4, 0]), page, (0x238b, &[7, 1])],
        &b"\x82\xa0".repeat(1100),
    );
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    messages.set_record_output(true);
    let mut scene = cmvs_rs::scene::SceneHost::default();
    run_calls(&mut vm, &mut messages, 3);
    for _ in 0..2 {
        messages.advance_auxiliary_motion(16, &mut scene).unwrap();
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.glyphs.len(), 1100);
        assert_eq!(group.attached_glyphs().count(), 1023);
        assert!(group.glyphs[1023..].iter().all(|g| g.cell == 0));
    }
    // 24-pixel glyphs in a 64-glyph atlas: one 512-pixel page of 289 cells; and a glyph
    // size changed after creation no longer matches the atlas cells.
    for (calls, glyphs) in [
        (
            vec![(0x2380, &[7, 0, 24, 64][..]), page, (0x238b, &[7, 1])],
            290,
        ),
        (
            vec![
                (0x2380, &[7, 0, 24, 64][..]),
                page,
                (0x2387, &[7, 30, u32::MAX, u32::MAX]),
                (0x238b, &[7, 1]),
            ],
            1,
        ),
    ] {
        let script = content_script(&calls, &b"\x82\xa0".repeat(glyphs));
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        run_calls(&mut vm, &mut messages, calls.len());
        assert!(messages.advance_auxiliary_motion(16, &mut scene).is_err());
    }
}

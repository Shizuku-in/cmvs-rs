use cmvs_rs::{
    input::ActionInput,
    machine::{Machine, Step},
    message::{
        LineBreakRules, MESSAGE_WINDOWS, MessageHost, MessageWindow, TEXT_OBJECTS, TextObject,
        WINDOW_BUTTONS, WindowButton,
    },
    ps3::Script,
    scene::{Animation, AnimationFrame, DrawState, SceneObject},
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
/// Pushes `words` bottom first, then issues `command`.
fn call(code: &mut Vec<u8>, command: u16, words: &[u32]) {
    for &word in words {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(word.to_le_bytes());
    }
    code.extend(command.to_le_bytes());
}
fn destroy(slots: &[u32]) -> Vec<u8> {
    let mut code = vec![];
    for &slot in slots {
        call(&mut code, 0x2141, &[slot]);
    }
    code
}
/// 0x215d arguments in push order: logical height/width, height/width, y/x, text, slot.
fn create(code: &mut Vec<u8>, slot: u32, text: u32, rect: [u32; 4]) {
    let [x, y, width, height] = rect;
    call(code, 0x215d, &[720, 1280, height, width, y, x, text, slot]);
}
/// The text object 0x215d leaves after 0x14005d3b0, for a 1280x720 logical size.
fn created_text(position: [i32; 2], size: [u32; 2]) -> TextObject {
    TextObject {
        font_face: Vec::new(),
        scale_bits: 1.0f32.to_bits(),
        logical_size: [1280, 720],
        position,
        area: [0, 0, size[0] as i32, size[1] as i32],
        cursor: [0, 0],
        ruby_lines: [0; 32],
        layout_scratch: [0; 2],
        glyphs: Vec::new(),
        glyph_size: 27,
        spacing: [0, 0],
        color: 0xff_ffff,
        decoration_color: 0xff_ffff,
        outline_mode: 2,
        glyph_parameter: 0,
        decoration: 0,
        reveal: [0, 0],
        char_delay: 100,
        layer: 1,
        visible: true,
        ruby: false,
        ruby_every_line: false,
        ruby_size: 12,
        ruby_spacing: 0,
        line_break: None,
        glyph_capacity: 196,
    }
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn empty_auxiliary_text_destruction_preserves_windows_text_and_results() {
    let mut code = vec![];
    create(&mut code, 0, 0, [20, 30, 100, 50]);
    for _ in 0..2 {
        for slot in 0..8 {
            call(&mut code, 0x2382, &[slot]);
        }
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    pending(&mut vm);
    assert!(messages.try_handle(&mut vm).unwrap());
    let window = messages.window(0).unwrap().clone();
    let text = messages.text(0).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 200 + i).unwrap();
    }
    for _ in 0..16 {
        pending(&mut vm);
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(messages.window(0), Some(&window));
        assert_eq!(messages.text(0), Some(&text));
        assert_eq!(vm.stack().len_bytes(), 0);
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(200 + i));
        }
    }
}

#[test]
fn auxiliary_text_destruction_rejects_missing_or_invalid_slot_atomically() {
    for args in [vec![], vec![8], vec![u32::MAX]] {
        let mut code = vec![];
        call(&mut code, 0x2382, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 123).unwrap();
        let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        let mut messages = MessageHost::default();
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err());
            assert_eq!(
                (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
                before
            );
            assert_eq!(vm.state().system_integer(0), Some(123));
        }
    }
}

#[test]
fn auxiliary_text_explicit_viewport_creation_stays_unhandled() {
    let mut code = vec![];
    call(&mut code, 0x2381, &[0, 0, 0, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
    assert!(!MessageHost::default().try_handle(&mut vm).unwrap());
    assert_eq!(
        (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
        before
    );
}
fn run(vm: &mut Machine<'_>, messages: &mut MessageHost, commands: usize) {
    for _ in 0..commands {
        pending(vm);
        assert!(messages.try_handle(vm).unwrap());
    }
}

#[test]
fn destroying_constructor_empty_windows_pops_the_slot_only() {
    let script = script(&destroy(&[0, 3, 7]));
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert!((0..MESSAGE_WINDOWS).all(|slot| messages.window(slot).is_none()));
    assert!((0..TEXT_OBJECTS).all(|index| messages.text(index).is_none()));
}

#[test]
fn out_of_range_negative_slots_and_short_stacks_keep_the_command_pending() {
    let mut cases = vec![
        destroy(&[8]),
        destroy(&[u32::MAX]),
        destroy(&[0x8000_0000]),
        0x2141u16.to_le_bytes().to_vec(),
    ];
    for (slot, text) in [(8, 0), (u32::MAX, 0), (0, 12), (0, u32::MAX)] {
        let mut code = vec![];
        create(&mut code, slot, text, [0, 0, 1280, 54]);
        cases.push(code);
    }
    let mut short = vec![];
    call(&mut short, 0x215d, &[54, 1280, 0, 0, 10, 7]);
    cases.push(short);
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let (pc, command, bytes) = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert!(MessageHost::default().try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.pending_system().map(|p| p.0), Some(pc));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn creation_stores_window_text_and_font_and_pops_all_arguments() {
    let mut code = vec![];
    create(&mut code, 7, 10, [3, u32::MAX, 1280, 54]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert_eq!(
        messages.window(7),
        Some(&MessageWindow {
            text: 10,
            visible: false,
            hidden: false,
            position: [3, -1],
            size: [1280, 54],
            switch_positions: [[3, -1]; 2],
            cursor_mode: 0,
            cursor_offset: [0, 0],
            position_switch: false,
            object: SceneObject::default(),
            buttons: [WindowButton::default(); WINDOW_BUTTONS],
            pressed_button: -1,
            switch_state: 0,
        })
    );
    assert_eq!(messages.text(10), Some(&created_text([3, -1], [1280, 54])));
    assert!(messages.font_present(10));
    assert_eq!(
        (0..TEXT_OBJECTS)
            .filter(|&i| messages.font_present(i))
            .count(),
        1
    );
}

#[test]
fn recreating_a_slot_keeps_the_old_text_and_destruction_keeps_fonts() {
    let mut code = vec![];
    create(&mut code, 2, 4, [0, 0, 100, 50]);
    create(&mut code, 2, 5, [10, 20, 30, 40]);
    // Recreating with its own text index replaces that object in place.
    create(&mut code, 2, 5, [1, 2, 3, 4]);
    code.extend(destroy(&[2]));
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    assert_eq!(messages.window(2).map(|w| w.text), Some(5));
    // The old window's text object stays in the table, as 0x140075ce0 leaves it.
    assert_eq!(messages.text(4).map(|t| t.area), Some([0, 0, 100, 50]));
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.text(5).map(|t| t.position), Some([1, 2]));
    run(&mut vm, &mut messages, 1);
    assert!(messages.window(2).is_none());
    assert!(messages.text(5).is_none());
    assert!(messages.text(4).is_some());
    assert!(messages.font_present(4) && messages.font_present(5));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn replacing_a_text_object_used_by_another_window_is_rejected() {
    let mut code = vec![];
    create(&mut code, 0, 3, [0, 0, 10, 10]);
    create(&mut code, 1, 3, [5, 5, 20, 20]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    pending(&mut vm);
    let bytes = vm.stack().len_bytes();
    assert!(messages.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x15d));
    assert_eq!(vm.stack().len_bytes(), bytes);
    assert!(messages.window(1).is_none());
    assert_eq!(messages.text(3).map(|t| t.area), Some([0, 0, 10, 10]));
}

#[test]
fn image_loading_stays_pending_without_an_image_source() {
    {
        let command = 0x2170u16;
        let script = script(&command.to_le_bytes());
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(!MessageHost::default().try_handle(&mut vm).unwrap());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
    }
}

/// Modeled on the window 7 setup intproc.ps3 runs after 0x215d (the area differs);
/// arguments are in push order.
fn configure(code: &mut Vec<u8>) {
    call(code, 0x215b, &[0, 7]);
    call(code, 0x2142, &[4, 7]);
    call(code, 0x2143, &[24, 7]);
    call(code, 0x2144, &[0, 0, 7]);
    call(code, 0x2145, &[50, 1200, 2, 40, 7]);
    call(code, 0x2146, &[0xffffff, 0xff00ff, 7]);
    call(code, 0x2147, &[50, 7]);
    call(code, 0x2148, &[350, 1, 7]);
    call(code, 0x2149, &[1, 7]);
}

#[test]
fn setters_update_the_window_and_its_text_object() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    configure(&mut code);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 10);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert!(!messages.window(7).unwrap().position_switch);
    assert_eq!(
        messages.text(10),
        Some(&TextObject {
            area: [40, 2, 1200, 50],
            cursor: [40, 2],
            glyph_size: 24,
            color: 0xff00ff,
            decoration_color: 0xffffff,
            decoration: 1,
            reveal: [1, 350],
            char_delay: 50,
            layer: 4,
            ..created_text([0, 0], [1280, 54])
        })
    );
}

#[test]
fn guarded_setters_keep_rejected_values() {
    let mut code = vec![];
    create(&mut code, 0, 2, [0, 0, 100, 20]);
    call(&mut code, 0x2143, &[0, 0]);
    call(&mut code, 0x2143, &[u32::MAX, 0]);
    call(&mut code, 0x2144, &[7, u32::MAX, 0]);
    call(&mut code, 0x2146, &[0x123456, 0x0100_0000, 0]);
    call(&mut code, 0x215b, &[5, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 6);
    let text = messages.text(2).unwrap();
    assert_eq!(text.glyph_size, 27);
    assert_eq!(text.spacing, [0, 7]);
    assert_eq!((text.color, text.decoration_color), (0xffffff, 0x123456));
    assert!(messages.window(0).unwrap().position_switch);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn setters_without_a_window_or_arguments_keep_the_command_pending() {
    let mut cases = vec![];
    for slot in [3, 8, u32::MAX] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        call(&mut code, 0x2147, &[50, slot]);
        cases.push(code);
    }
    let mut short = vec![];
    create(&mut short, 0, 0, [0, 0, 10, 10]);
    call(&mut short, 0x2145, &[0, 0, 0]);
    cases.push(short);
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.text(0).map(|t| t.char_delay), Some(100));
    }
}

#[test]
fn window_object_commands_rebuild_hide_and_clear_flags() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2178, &[u32::MAX, 7]);
    call(&mut code, 0x2178, &[5, 7]);
    call(&mut code, 0x217f, &[u32::MAX, 7]);
    call(&mut code, 0x217f, &[5, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    let object = &messages.window(7).unwrap().object;
    assert_eq!(
        object,
        &SceneObject {
            shown: false,
            ..SceneObject::default()
        }
    );
    run(&mut vm, &mut messages, 1);
    let object = &messages.window(7).unwrap().object;
    // Without a draw state the child starts with no inherited flags, like 0x2022.
    assert_eq!(object.flags, 0xffff_ffff);
    assert_eq!(
        object.children.get(&5),
        Some(&SceneObject {
            flags: 0,
            shown: false,
            ..SceneObject::default()
        })
    );
    run(&mut vm, &mut messages, 2);
    let object = &messages.window(7).unwrap().object;
    assert_eq!(object.flags, 0xffff_feff);
    assert_eq!(object.children[&5].flags, 0);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn window_object_commands_need_a_window_and_valid_children() {
    let mut cases = vec![];
    for (command, child) in [(0x2178u16, 768u32), (0x217f, 5)] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        call(&mut code, command, &[child, 0]);
        cases.push(code);
    }
    let mut empty = vec![];
    create(&mut empty, 0, 0, [0, 0, 10, 10]);
    call(&mut empty, 0x2178, &[u32::MAX, 1]);
    cases.push(empty);
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.window(0).unwrap().object, SceneObject::default());
    }
}

#[test]
fn window_child_destruction_removes_existing_children_and_ignores_others() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2178, &[5, 7]);
    call(&mut code, 0x2178, &[6, 7]);
    call(&mut code, 0x2180, &[5, 7]);
    for child in [5, 5, 768, 0x7fff_ffff] {
        call(&mut code, 0x2179, &[child, 7]);
    }
    call(&mut code, 0x2178, &[5, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 4);
    let before = messages.window(7).unwrap().object.clone();
    assert!(before.children[&5].draw.is_some());
    run(&mut vm, &mut messages, 1);
    let object = &messages.window(7).unwrap().object;
    assert_eq!(object.children.keys().copied().collect::<Vec<_>>(), [6]);
    assert_eq!(object.children[&6], before.children[&6]);
    assert_eq!(
        (object.flags, object.draw.clone()),
        (before.flags, before.draw)
    );
    let after = object.clone();
    // Missing children and children beyond the 768-entry table are quiet no-ops.
    run(&mut vm, &mut messages, 3);
    assert_eq!(messages.window(7).unwrap().object, after);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    // A rebuilt child starts fresh rather than reusing the destroyed state.
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.window(7).unwrap().object.children[&5].draw, None);
}

#[test]
fn window_child_destruction_rejects_the_window_object_missing_windows_and_short_stacks() {
    let mut cases = vec![];
    for words in [
        [u32::MAX, 0],
        [0x8000_0000, 0],
        [5, 1],
        [5, 8],
        [5, u32::MAX],
    ] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        call(&mut code, 0x2178, &[5, 0]);
        call(&mut code, 0x2179, &words);
        cases.push(code);
    }
    let mut short = vec![];
    create(&mut short, 0, 0, [0, 0, 10, 10]);
    call(&mut short, 0x2178, &[5, 0]);
    call(&mut short, 0x2179, &[0]);
    cases.push(short);
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 2);
        let before = messages.window(0).unwrap().object.clone();
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.window(0).unwrap().object, before);
    }
}

#[test]
fn window_draw_commands_reuse_scene_draw_fields() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2180, &[u32::MAX, 7]);
    call(&mut code, 0x2182, &[54, 1280, 2, 1, u32::MAX, 7]);
    call(&mut code, 0x2183, &[6, 5, u32::MAX, 7]);
    call(&mut code, 0x2184, &[3, u32::MAX, 7]);
    call(&mut code, 0x2185, &[9, 8, u32::MAX, 7]);
    call(&mut code, 0x2178, &[16, 7]);
    call(&mut code, 0x2180, &[16, 7]);
    call(&mut code, 0x2183, &[0, 588, 16, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    let object = &messages.window(7).unwrap().object;
    assert_eq!(object.draw, Some(DrawState::default()));
    run(&mut vm, &mut messages, 4);
    let words = messages
        .window(7)
        .unwrap()
        .object
        .draw
        .clone()
        .unwrap()
        .words;
    assert_eq!(words[2..11], [1, 2, 1280, 54, 8, 9, 5, 6, 3]);
    run(&mut vm, &mut messages, 3);
    let child = &messages.window(7).unwrap().object.children[&16];
    assert_eq!(child.draw.as_ref().unwrap().words[8..10], [588, 0]);
    assert!(!child.shown);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn window_draw_commands_need_a_target_and_its_draw_state() {
    let mut cases = vec![];
    for (command, words) in [
        (0x2183u16, vec![0, 0, u32::MAX, 0]),
        (0x2180, vec![5, 0]),
        (0x2180, vec![768, 0]),
        (0x2180, vec![u32::MAX, 1]),
        (0x2182, vec![0, 0, u32::MAX, 0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        call(&mut code, command, &words);
        cases.push(code);
    }
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.window(0).unwrap().object, SceneObject::default());
    }
}

#[test]
fn button_records_store_states_hit_rectangles_and_enable_children() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2178, &[16, 7]);
    // State 1 of record 0, with a value whose upper bits the 16-bit field drops.
    call(&mut code, 0x2190, &[0, 0x1_0005, 52, 94, 0, 96, 1, 0, 7]);
    call(&mut code, 0x2191, &[52, 94, 0, 0, 0, 7]);
    call(&mut code, 0x2192, &[0, 7]);
    call(&mut code, 0x2192, &[3, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 6);
    let window = messages.window(7).unwrap();
    let mut states = [[0; 6]; 5];
    states[1] = [96, 0, 94, 52, 5, 0];
    assert_eq!(
        window.buttons[0],
        WindowButton {
            enabled: true,
            states,
            hit: [0, 0, 94, 52],
            state_word: 0,
        }
    );
    assert!(window.object.children[&16].shown);
    // Record 3 has no child 19 to show; only the record is enabled.
    assert!(window.buttons[3].enabled);
    assert_eq!(window.object.children.len(), 1);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn button_commands_reject_bad_records_states_and_missing_windows() {
    let mut cases = vec![];
    for (command, words) in [
        (0x2192u16, vec![32, 0]),
        (0x2192, vec![u32::MAX, 0]),
        (0x2192, vec![0, 1]),
        (0x2190, vec![0, 0, 0, 0, 0, 0, 5, 0, 0]),
        (0x2191, vec![0, 0, 0, 0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        call(&mut code, command, &words);
        cases.push(code);
    }
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert!(messages.window(0).unwrap().buttons == [WindowButton::default(); WINDOW_BUTTONS]);
    }
}

#[test]
fn window_visibility_sets_the_shown_flag_without_a_draw_state() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2178, &[64, 7]);
    call(&mut code, 0x217a, &[5, 64, 7]);
    call(&mut code, 0x217a, &[0, u32::MAX, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    let object = &messages.window(7).unwrap().object;
    assert!(object.children[&64].shown);
    assert!(object.children[&64].draw.is_none());
    run(&mut vm, &mut messages, 1);
    assert!(!messages.window(7).unwrap().object.shown);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn moving_a_window_updates_state_zero_object_and_text_positions() {
    let mut code = vec![];
    create(&mut code, 7, 10, [5, 6, 1280, 54]);
    call(&mut code, 0x2180, &[u32::MAX, 7]);
    call(&mut code, 0x2150, &[0, 1186, 0, 7]);
    call(&mut code, 0x2150, &[4, u32::MAX, 1, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    let window = messages.window(7).unwrap();
    assert_eq!(window.position, [1186, 0]);
    assert_eq!(window.switch_positions, [[1186, 0], [5, 6]]);
    assert_eq!(window.object.draw.as_ref().unwrap().words[8..10], [1186, 0]);
    assert_eq!(messages.text(10).unwrap().position, [1186, 0]);
    run(&mut vm, &mut messages, 1);
    let window = messages.window(7).unwrap();
    assert_eq!(window.position, [1185, 4]);
    assert_eq!(window.switch_positions, [[1185, 4], [5, 6]]);
    assert_eq!(window.object.draw.as_ref().unwrap().words[8..10], [1185, 4]);
    assert_eq!(messages.text(10).unwrap().position, [1185, 4]);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn window_moves_need_a_known_mode_a_window_and_a_draw_state() {
    let mut cases = vec![];
    for (draw, words) in [
        (true, vec![0, 0, 2, 0]),
        (false, vec![0, 0, 0, 0]),
        (true, vec![0, 0, 0, 1]),
        (true, vec![0, 0, 0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 0, [0, 0, 10, 10]);
        if draw {
            call(&mut code, 0x2180, &[u32::MAX, 0]);
        }
        call(&mut code, 0x2150, &words);
        cases.push((code, 1 + usize::from(draw)));
    }
    for (code, setup) in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, setup);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        let window = messages.window(0).unwrap();
        assert_eq!(window.position, [0, 0]);
        assert_eq!(messages.text(0).unwrap().position, [0, 0]);
    }
}

#[test]
fn text_object_commands_set_ruby_fields_and_logical_size() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2114, &[0, 7]);
    call(&mut code, 0x2114, &[1, 7]);
    call(&mut code, 0x2115, &[15, 7]);
    call(&mut code, 0x2116, &[u32::MAX, 7]);
    call(&mut code, 0x2117, &[2, 7]);
    call(&mut code, 0x217c, &[720, 1280, u32::MAX, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    assert!(!messages.text(7).unwrap().ruby);
    run(&mut vm, &mut messages, 4);
    let text = messages.text(7).unwrap();
    assert_eq!(
        (
            text.ruby,
            text.ruby_size,
            text.ruby_spacing,
            text.ruby_every_line
        ),
        (true, 15, -1, true)
    );
    run(&mut vm, &mut messages, 1);
    let object = &messages.window(0).unwrap().object;
    assert_eq!(object.logical_size, [1280, 720]);
    assert!(object.draw.is_none());
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn standalone_text_creation_uses_constructor_values_and_overwrites_in_place() {
    let mut code = vec![];
    call(&mut code, 0x2100, &[0, 0, 3]);
    call(&mut code, 0x2114, &[1, 3]);
    call(&mut code, 0x2100, &[196, 0, 3]);
    let script = script_with_text(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    let expected = TextObject {
        area: [0; 4],
        position: [0, 0],
        glyph_size: 0,
        char_delay: 30,
        glyph_capacity: 0,
        ..created_text([0, 0], [0, 0])
    };
    assert_eq!(messages.text(3), Some(&expected));
    assert!(messages.font_present(3));
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);
    run(&mut vm, &mut messages, 1);
    assert!(messages.text(3).unwrap().ruby);
    run(&mut vm, &mut messages, 1);
    let recreated = TextObject {
        glyph_capacity: 196,
        ..expected.clone()
    };
    assert_eq!(messages.text(3), Some(&recreated));
    assert!((0..MESSAGE_WINDOWS).all(|slot| messages.window(slot).is_none()));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn standalone_text_creation_rejects_unmodeled_arguments_atomically() {
    // Index 12, negative index, a named face, an unverified capacity, a window's text.
    for (words, window) in [
        ([0, 0, 12], false),
        ([0, 0, u32::MAX], false),
        ([0, 1, 3], false),
        ([5, 0, 3], false),
        ([0, 0, 3], true),
    ] {
        let mut code = vec![];
        if window {
            create(&mut code, 0, 3, [0, 0, 10, 10]);
        }
        call(&mut code, 0x2100, &words);
        let script = script_with_text(&code, b"\0MS Gothic\0");
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, usize::from(window));
        let before = messages.text(3).cloned();
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err(), "{words:?}");
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x100));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), Some(9));
        assert_eq!(messages.text(3).cloned(), before);
        assert_eq!(messages.font_present(3), window);
    }
}

/// A PS2A image whose text segment holds `text`, for 0x211e string references.
fn script_with_text(code: &[u8], text: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for n in [
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
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}

#[test]
fn recorded_output_only_release_edge_skips_reveal_not_press_edge() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let script = script_with_text(&code, b"A\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    messages.set_record_output(true);
    pending(&mut vm);
    let mut input = ActionInput::default();
    input.set(0, true, true, false).unwrap();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    assert!(vm.pending_system().is_some());
    assert!(messages.text(7).unwrap().glyphs[0].active);
    assert!(input.pressed(0));
    input.set(0, false, false, true).unwrap();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    assert!(vm.pending_system().is_none());
    assert!(!messages.text(7).unwrap().glyphs[0].active);
    assert!(!input.released(0));
}

fn wait_flags(skip: bool, auto: bool) -> cmvs_rs::message_flags::MessageFlags {
    message_flags(skip, auto, false)
}
/// Flags set through the script commands: 0x22eb skip, 0x22e5 auto advance, 0x2340 bypass.
fn message_flags(skip: bool, auto: bool, bypass: bool) -> cmvs_rs::message_flags::MessageFlags {
    let mut code = vec![];
    call(&mut code, 0x22eb, &[u32::from(skip), 1]);
    call(&mut code, 0x22e5, &[u32::from(auto), 1]);
    call(&mut code, 0x2340, &[u32::from(bypass)]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut flags = cmvs_rs::message_flags::MessageFlags::default();
    for _ in 0..3 {
        pending(&mut vm);
        assert!(flags.try_handle(&mut vm).unwrap());
    }
    assert_eq!(flags.bypass(), bypass);
    flags
}

#[test]
fn message_wait_preserves_parameters_shows_pen_cursor_and_advances_on_release() {
    let mut code = vec![];
    create(&mut code, 0, 7, [100, 200, 100, 50]);
    for child in [2, 3] {
        call(&mut code, 0x2178, &[child, 0]);
        call(&mut code, 0x2180, &[child, 0]);
    }
    call(&mut code, 0x2151, &[7, 5, 3, 0]);
    call(&mut code, 0x214f, &[20, 10, 0]);
    call(&mut code, 0x2153, &[u32::MAX, 123, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 7);
    pending(&mut vm);
    let pc = vm.pc();
    let mut input = ActionInput::default();
    input.set_pointer(-1, -1);
    vm.state_mut().set_system_integer(0, 99).unwrap();
    for count in 1..=2 {
        assert_eq!(
            messages
                .try_wait(&mut vm, &mut input, wait_flags(false, false))
                .unwrap(),
            Some(Step::Yield)
        );
        assert_eq!(vm.pending_system(), Some((pc, 0x153)));
        assert_eq!(vm.stack().len_bytes(), 12);
        assert_eq!(vm.repeat_count(), count);
        for child in messages.window(0).unwrap().object.children.values() {
            assert!(child.shown);
            assert_eq!(&child.draw.as_ref().unwrap().words[8..10], &[15, 27]);
        }
    }
    input.set(0, true, true, false).unwrap();
    messages
        .try_wait(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    assert!(vm.pending_system().is_some());
    assert!(input.pressed(0));
    input.set(0, false, false, true).unwrap();
    input.set(1, true, true, true).unwrap();
    assert_eq!(
        messages
            .try_wait(&mut vm, &mut input, wait_flags(false, false))
            .unwrap(),
        Some(Step::Continue)
    );
    assert_eq!(vm.pc(), pc + 2);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.repeat_count(), 0);
    assert!(!input.released(0));
    assert!(input.pressed(1));
    assert_eq!(vm.state().system_integer(0), Some(99));
    assert!(
        messages
            .window(0)
            .unwrap()
            .object
            .children
            .values()
            .all(|c| !c.shown)
    );
}

#[test]
fn message_wait_button_edges_are_inclusive_and_drag_release_does_not_advance() {
    for outside in [false, true] {
        let mut code = vec![];
        create(&mut code, 0, 7, [100, 200, 100, 100]);
        call(&mut code, 0x2191, &[40, 30, 20, 10, 0, 0]);
        call(&mut code, 0x2192, &[0, 0]);
        call(&mut code, 0x2153, &[u32::MAX, 0, 0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 3);
        pending(&mut vm);
        let mut input = ActionInput::default();
        input.set_pointer(140, 260);
        input.set(0, true, true, false).unwrap();
        messages
            .try_wait(&mut vm, &mut input, wait_flags(false, false))
            .unwrap();
        assert_eq!(messages.window(0).unwrap().pressed_button, 0);
        if outside {
            input.set_pointer(141, 261);
        }
        input.set(0, false, false, true).unwrap();
        messages
            .try_wait(&mut vm, &mut input, wait_flags(false, false))
            .unwrap();
        assert!(vm.pending_system().is_some());
        assert!(!input.released(0));
        assert_eq!(messages.window(0).unwrap().pressed_button, -1);
        input.set_pointer(141, 261);
        input.set(0, false, false, true).unwrap();
        messages
            .try_wait(&mut vm, &mut input, wait_flags(false, false))
            .unwrap();
        assert!(vm.pending_system().is_none());
    }
}

#[test]
fn message_wait_skip_flags_and_actions_complete_without_consuming_other_edges() {
    for (skip, action) in [(true, 1), (false, 7), (false, 8)] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        call(&mut code, 0x2153, &[u32::MAX, 0, 0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let mut input = ActionInput::default();
        input.set(action, true, true, true).unwrap();
        assert_eq!(
            messages
                .try_wait(&mut vm, &mut input, wait_flags(skip, false))
                .unwrap(),
            Some(Step::Continue)
        );
        assert!(vm.pending_system().is_none());
        assert!(input.pressed(action));
        assert!(input.released(action));
    }
}

#[test]
fn message_wait_action_twenty_release_advances_outside_buttons_and_consumes_both_actions() {
    // (hovered, dragged from the button): a drag release consumes action 0 first, but a
    // simultaneous action 20 release still advances.
    for (hovered, dragged) in [(false, false), (true, false), (false, true)] {
        let mut code = vec![];
        create(&mut code, 0, 7, [100, 200, 100, 100]);
        call(&mut code, 0x2191, &[40, 30, 20, 10, 0, 0]);
        call(&mut code, 0x2192, &[0, 0]);
        call(&mut code, 0x2153, &[u32::MAX, 0, 0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 3);
        pending(&mut vm);
        let mut input = ActionInput::default();
        if dragged {
            input.set_pointer(140, 260);
            input.set(0, true, true, false).unwrap();
            messages
                .try_wait(&mut vm, &mut input, wait_flags(false, false))
                .unwrap();
            assert_eq!(messages.window(0).unwrap().pressed_button, 0);
            input.set(0, false, false, true).unwrap();
        } else {
            // A press edge alone never advances; it is cleared with action 20.
            input.set(0, true, true, false).unwrap();
        }
        input.set_pointer(
            if hovered { 140 } else { 141 },
            if hovered { 260 } else { 261 },
        );
        input.set(20, false, false, true).unwrap();
        input.set(1, true, true, true).unwrap();
        let step = messages
            .try_wait(&mut vm, &mut input, wait_flags(false, false))
            .unwrap();
        assert!(input.pressed(1) && input.released(1));
        if hovered {
            assert_eq!(step, Some(Step::Yield));
            assert!(vm.pending_system().is_some());
            assert!(input.released(20) && input.pressed(0));
        } else {
            assert_eq!(step, Some(Step::Continue));
            assert!(vm.pending_system().is_none());
            assert!(!input.released(20));
            assert!(!input.pressed(0) && !input.released(0));
            assert_eq!(input.held(0), !dragged);
        }
    }
}

#[test]
fn message_wait_bypass_completes_like_skip_before_auto_advance_without_consuming_input() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    for child in [2, 3] {
        call(&mut code, 0x2178, &[child, 0]);
    }
    call(&mut code, 0x2153, &[u32::MAX, 0, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    pending(&mut vm);
    let pc = vm.pc();
    let mut input = ActionInput::default();
    input.set(0, true, true, true).unwrap();
    let old = input.clone();
    // Auto advance alone is an explicit barrier; the bypass completes before reaching it.
    assert_eq!(
        messages
            .try_wait(&mut vm, &mut input, message_flags(false, true, true))
            .unwrap(),
        Some(Step::Continue)
    );
    assert_eq!((vm.pending_system(), vm.pc()), (None, pc + 2));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(input, old);
    let window = messages.window(0).unwrap();
    assert!(window.object.children.values().all(|child| !child.shown));
}

#[test]
fn message_wait_hidden_window_ignores_skip_and_preserves_all_input_and_children() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x214e, &[1, 0]);
    call(&mut code, 0x214c, &[1, 0]);
    call(&mut code, 0x2153, &[u32::MAX, 0, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    pending(&mut vm);
    let before = messages.window(0).cloned();
    let mut input = ActionInput::default();
    input.set(0, false, false, true).unwrap();
    let old = input.clone();
    assert_eq!(
        messages
            .try_wait(&mut vm, &mut input, message_flags(true, true, true))
            .unwrap(),
        Some(Step::Yield)
    );
    assert_eq!(messages.window(0), before.as_ref());
    assert_eq!(input, old);
    assert_eq!(vm.stack().len_bytes(), 12);
}

#[test]
fn message_wait_rejects_unsupported_auto_missing_cursor_draw_and_bad_arguments_atomically() {
    for (auto, child, words) in [
        (true, false, vec![u32::MAX, 0, 0]),
        (false, true, vec![u32::MAX, 0, 0]),
        (false, false, vec![u32::MAX, 0, 8]),
        (false, false, vec![u32::MAX, 0, u32::MAX]),
        (false, false, vec![u32::MAX, 0, 1]),
        (false, false, vec![0, 0]),
        (false, false, vec![0]),
        (false, false, vec![]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        let mut setup = 1;
        if child {
            call(&mut code, 0x2178, &[2, 0]);
            call(&mut code, 0x2151, &[0, 0, 3, 0]);
            setup += 2;
        }
        call(&mut code, 0x2153, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, setup);
        pending(&mut vm);
        let before = messages.window(0).cloned();
        let command = vm.pending_system();
        let mut input = ActionInput::default();
        input.set(0, true, true, false).unwrap();
        let old = input.clone();
        assert!(
            messages
                .try_wait(&mut vm, &mut input, wait_flags(false, auto))
                .is_err()
        );
        assert_eq!(messages.window(0), before.as_ref());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), words.len() * 4);
        assert_eq!(vm.repeat_count(), 0);
        assert_eq!(input, old);
    }
}

#[test]
fn recorded_output_requires_explicit_policy_and_yields_without_duplicating_glyphs() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2152, &[0, 123, 0]);
    let script = script_with_text(&code, b"\x82\xa0A\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    let mut input = ActionInput::default();
    run(&mut vm, &mut messages, 1);
    pending(&mut vm);
    let pc = vm.pc();
    let before = messages.text(7).cloned();
    assert_eq!(
        messages
            .try_record_output(&mut vm, &mut input, wait_flags(false, false))
            .unwrap(),
        None
    );
    assert_eq!(messages.text(7), before.as_ref());
    assert_eq!(vm.repeat_count(), 0);
    messages.set_record_output(true);
    for i in 1..=3 {
        assert_eq!(
            messages
                .try_record_output(&mut vm, &mut input, wait_flags(false, false))
                .unwrap(),
            Some(Step::Yield)
        );
        assert_eq!(vm.pending_system(), Some((pc, 0x152)));
        assert_eq!(vm.repeat_count(), i);
        assert_eq!(vm.stack().len_bytes(), 12);
        assert_eq!(messages.text(7).unwrap().glyphs.len(), 2);
        assert_eq!(messages.text(7).unwrap().cursor, [40, 0]);
    }
    let glyphs = &messages.text(7).unwrap().glyphs;
    assert_eq!((glyphs[0].code, glyphs[0].position), (0x82a0, [0, 0]));
    assert_eq!(
        (glyphs[1].code, glyphs[1].position, glyphs[1].delay_ms),
        (65, [27, 0], 40)
    );
    messages.advance_frame(40);
    let glyphs = &messages.text(7).unwrap().glyphs;
    assert_eq!((glyphs[0].shown, glyphs[0].opacity), (true, 29));
    assert!(!glyphs[1].shown);
    messages.advance_frame(350);
    assert!(
        messages
            .text(7)
            .unwrap()
            .glyphs
            .iter()
            .all(|g| !g.active && g.shown && g.opacity == 255)
    );
    assert_eq!(
        messages
            .try_record_output(&mut vm, &mut input, wait_flags(false, false))
            .unwrap(),
        Some(Step::Yield)
    );
    assert_eq!(vm.pc(), pc + 2);
    assert_eq!(vm.pending_system(), None);
    assert_eq!(vm.repeat_count(), 0);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn recorded_output_captures_styles_and_ruby_reserved_height_then_clears() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2114, &[1, 7]);
    call(&mut code, 0x2117, &[1, 7]);
    call(&mut code, 0x2149, &[2, 0]);
    call(&mut code, 0x2146, &[0xabcdef, 0x123456, 0]);
    call(&mut code, 0x2155, &[0x272727, 0]);
    call(&mut code, 0x2152, &[0, u32::MAX, 0]);
    call(&mut code, 0x214a, &[0]);
    let script = script_with_text(&code, b"A\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 6);
    messages.set_record_output(true);
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 42).unwrap();
    messages
        .try_record_output(
            &mut vm,
            &mut ActionInput::default(),
            wait_flags(true, false),
        )
        .unwrap();
    let glyph = &messages.text(7).unwrap().glyphs[0];
    assert_eq!(glyph.position, [0, 12]);
    assert_eq!(glyph.colors, [0x123456, 0xabcdef]);
    assert_eq!(glyph.gradient, 2);
    assert_eq!(glyph.outline, [1, 0x272727]);
    assert!(!glyph.active);
    assert_eq!(vm.state().system_integer(0), Some(42));
    run(&mut vm, &mut messages, 1);
    assert!(messages.text(7).unwrap().glyphs.is_empty());
    assert_eq!(messages.text(7).unwrap().cursor, [0, 0]);
}

#[test]
fn recorded_output_click_finishes_and_consumes_only_action_zero_edges() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let script = script_with_text(&code, b"AB\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    messages.set_record_output(true);
    pending(&mut vm);
    let mut input = ActionInput::default();
    input.set(0, true, true, true).unwrap();
    input.set(1, true, true, true).unwrap();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    assert!(
        messages
            .text(7)
            .unwrap()
            .glyphs
            .iter()
            .all(|g| !g.active && g.opacity == 255)
    );
    assert!(input.held(0));
    assert!(!input.pressed(0) && !input.released(0));
    assert!(input.pressed(1) && input.released(1));
    assert_eq!(vm.pending_system(), None);
}

#[test]
fn recorded_output_action_twenty_release_finishes_and_consumes_both_actions() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let script = script_with_text(&code, b"AB\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    messages.set_record_output(true);
    pending(&mut vm);
    let mut input = ActionInput::default();
    input.set(0, true, true, false).unwrap();
    input.set(20, true, true, true).unwrap();
    input.set(1, true, true, true).unwrap();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    assert_eq!(vm.pending_system(), None);
    assert!(messages.text(7).unwrap().glyphs.iter().all(|g| !g.active));
    assert!(input.held(0) && !input.pressed(0));
    assert!(input.held(20) && !input.pressed(20) && !input.released(20));
    assert!(input.pressed(1) && input.released(1));
}

#[test]
fn recorded_output_bypass_keeps_reveal_timing_and_finishes_without_consuming() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x214e, &[1, 0]);
    call(&mut code, 0x214c, &[1, 0]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let script = script_with_text(&code, b"AB\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    messages.set_record_output(true);
    pending(&mut vm);
    let pc = vm.pc();
    let flags = message_flags(false, false, true);
    let mut input = ActionInput::default();
    input.set(0, false, false, true).unwrap();
    let old = input.clone();
    // Hidden: glyphs are laid out with the normal interval, unlike the +1520 skip flag.
    messages
        .try_record_output(&mut vm, &mut input, flags)
        .unwrap();
    assert_eq!(vm.pending_system(), Some((pc, 0x152)));
    let glyphs = &messages.text(7).unwrap().glyphs;
    assert_eq!(
        glyphs.iter().map(|g| g.delay_ms).collect::<Vec<_>>(),
        [0, 40]
    );
    assert!(glyphs.iter().all(|g| g.active));
    let mut code = vec![];
    call(&mut code, 0x214c, &[0, 0]);
    let other_script = script_with_text(&code, b"");
    let mut other_vm = Machine::new(&other_script).unwrap();
    run(&mut other_vm, &mut messages, 1);
    messages
        .try_record_output(&mut vm, &mut input, flags)
        .unwrap();
    assert_eq!(vm.pending_system(), None);
    assert_eq!(vm.pc(), pc + 2);
    assert!(
        messages
            .text(7)
            .unwrap()
            .glyphs
            .iter()
            .all(|g| !g.active && g.opacity == 255)
    );
    assert_eq!(input, old);
}

#[test]
fn recorded_output_hidden_window_keeps_waiting_even_when_skip_finishes_layout() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x214e, &[1, 0]);
    call(&mut code, 0x214c, &[1, 0]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let script = script_with_text(&code, b"A\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    messages.set_record_output(true);
    pending(&mut vm);
    let pc = vm.pc();
    let mut input = ActionInput::default();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(true, false))
        .unwrap();
    assert_eq!(vm.pending_system(), Some((pc, 0x152)));
    assert_eq!(messages.text(7).unwrap().glyphs.len(), 1);
    let mut code = vec![];
    call(&mut code, 0x214c, &[0, 0]);
    let other_script = script_with_text(&code, b"");
    let mut other_vm = Machine::new(&other_script).unwrap();
    run(&mut other_vm, &mut messages, 1);
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(true, false))
        .unwrap();
    assert_eq!(vm.pending_system(), None);
    assert_eq!(messages.text(7).unwrap().glyphs.len(), 1);
}

/// Records `data[offset..]` in window 0 / text 7 with a 40-wide area and 10-pixel glyphs
/// (line end 30); `setup` runs between the size command and the output.
fn wrapped(setup: &[(u16, Vec<u32>)], data: &[u8], offset: u32) -> MessageHost {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 40, 100]);
    call(&mut code, 0x2143, &[10, 0]);
    for (command, words) in setup {
        call(&mut code, *command, words);
    }
    call(&mut code, 0x2152, &[offset, 0, 0]);
    let script = script_with_text(&code, data);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2 + setup.len());
    messages.set_record_output(true);
    pending(&mut vm);
    messages
        .try_record_output(
            &mut vm,
            &mut ActionInput::default(),
            wait_flags(false, false),
        )
        .unwrap();
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x152));
    messages
}
fn positions(messages: &MessageHost) -> Vec<[i32; 2]> {
    let glyphs = &messages.text(7).unwrap().glyphs;
    glyphs.iter().map(|glyph| glyph.position).collect()
}

#[test]
fn recorded_newlines_move_the_pen_without_glyphs_or_reveal_intervals() {
    let messages = wrapped(&[], b"\\nA\\n\\n\x83\x5cB\\n\0", 0);
    // 0x835c is one double-byte character whose trailing byte is a backslash.
    assert_eq!(positions(&messages), [[0, 10], [0, 30], [10, 30]]);
    let text = messages.text(7).unwrap();
    assert_eq!(text.cursor, [0, 40]);
    assert_eq!(
        text.glyphs
            .iter()
            .map(|g| (g.code, g.delay_ms))
            .collect::<Vec<_>>(),
        [(65, 0), (0x835c, 40), (66, 80)]
    );
    let (blank, ok) = ruby_layout(40, &[], b"\\n\\n\0");
    assert!(ok);
    assert!(blank.text(7).unwrap().glyphs.is_empty());
    assert_eq!(blank.text(7).unwrap().cursor, [0, 20]);
}

#[test]
fn recorded_newlines_reserve_ruby_on_the_destination_line_and_clear_break_scratch() {
    let (messages, ok) = ruby_layout(
        200,
        &[(0x2114, vec![1, 7]), (0x2115, vec![4, 7])],
        b"A\\n{\x82\xa0/\x82\xa2}\\nB",
    );
    assert!(ok);
    let glyphs = layout(&messages);
    assert_eq!(
        glyphs.iter().map(|g| (g.0, g.1)).collect::<Vec<_>>(),
        [
            (65, [0, 0]),
            (0x82a0, [0, 14]),
            (0x82a2, [3, 10]),
            (66, [0, 24])
        ]
    );
    assert_eq!(&messages.text(7).unwrap().ruby_lines[..3], &[0, 1, 0]);
    let messages = wrapped(&[(0x2117, vec![1, 7])], b"A\\nB\0", 0);
    assert_eq!(
        positions(&messages),
        [[0, 0], [0, 22]],
        "explicit newline reserves every-line ruby even with ruby disabled"
    );
    // Build up hanging overflow before the newline; a trailing newline clears both words.
    let messages = wrapped(
        &[(0x211e, vec![1, 3, 0, 7])],
        b"\x81\x42\0\x81\x5b\0\x82\xa0\x82\xa0\x82\xa0\x81\x42\\n\0",
        6,
    );
    assert_eq!(messages.text(7).unwrap().layout_scratch, [0, 0]);
    assert_eq!(messages.text(7).unwrap().cursor, [0, 10]);
}

#[test]
fn recorded_newline_overflow_and_unsupported_escapes_are_transactional() {
    // Ruby-line overflow, escapes that never advance and `\w` without a selector.
    for bytes in [
        [b"\\n".repeat(32), vec![0]].concat(),
        b"A\\nB\\x\0".to_vec(),
        b"A\\nB\\w\0".to_vec(),
        b"A\\nB\\\0".to_vec(),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        call(&mut code, 0x2114, &[1, 7]);
        call(&mut code, 0x2152, &[0, 0, 0]);
        let program = script_with_text(&code, &bytes);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 2);
        messages.set_record_output(true);
        pending(&mut vm);
        let old = messages.text(7).unwrap().clone();
        let pending_command = vm.pending_system();
        let mut input = ActionInput::default();
        input.set(0, true, true, false).unwrap();
        let before_input = input.clone();
        for _ in 0..2 {
            assert!(
                messages
                    .try_record_output(&mut vm, &mut input, wait_flags(false, false))
                    .is_err()
            );
            assert_eq!(messages.text(7), Some(&old));
            assert_eq!(vm.pending_system(), pending_command);
            assert_eq!(vm.stack().len_bytes(), 12);
            assert_eq!(vm.repeat_count(), 0);
            assert_eq!(input, before_input);
        }
    }
}

#[test]
fn recorded_output_wraps_without_rules_keeping_closing_punctuation_on_the_line() {
    // あああ。 fit with 。 past the line end; the next あ and the half-width e wrap.
    let data = b"\x82\xa0\x82\xa0\x82\xa0\x81\x42\x82\xa0abcde\0";
    let plain = wrapped(&[], data, 0);
    let rows = [0, 0, 0, 0, 10, 10, 10, 10, 10, 20];
    let xs = [0, 10, 20, 30, 0, 10, 15, 20, 25, 0];
    let expected: Vec<_> = xs.iter().zip(rows).map(|(&x, y)| [x, y]).collect();
    assert_eq!(positions(&plain), expected);
    let text = plain.text(7).unwrap();
    assert_eq!(text.cursor, [5, 20]);
    assert_eq!(text.layout_scratch, [0, 0]);
    // Reveal delays keep accumulating across lines.
    let delays: Vec<_> = text.glyphs.iter().map(|glyph| glyph.delay_ms).collect();
    assert_eq!(delays, (0..10).map(|n| 40 * n).collect::<Vec<_>>());
    // Every-line ruby reserves its height above the first and each wrapped line.
    let ruby = wrapped(&[(0x2114, vec![1, 7]), (0x2117, vec![1, 7])], data, 0);
    let shifted: Vec<_> = expected
        .iter()
        .map(|&[x, y]| [x, 12 + y / 10 * 22])
        .collect();
    assert_eq!(positions(&ruby), shifted);
    assert!(
        ruby.text(7)
            .unwrap()
            .ruby_lines
            .iter()
            .all(|&flag| flag == 0)
    );
}

#[test]
fn recorded_output_wraps_with_rules_through_hanging_and_inseparable_limits() {
    // Hanging set 。 at offset 0, inseparable set ー at offset 3, message at offset 6.
    let mut data = b"\x81\x42\0\x81\x5b\0".to_vec();
    for code in [
        0x82a0u16, 0x82a0, 0x82a0, 0x8142, 0x8142, 0x82a0, 0x815b, 0x815b, 0x82a0, 0x82a0, 0x82a0,
        0x815b,
    ] {
        data.extend(code.to_be_bytes());
    }
    data.push(0);
    let messages = wrapped(&[(0x211e, vec![1, 3, 0, 7])], &data, 6);
    assert_eq!(
        positions(&messages),
        [
            // One hanging 。 overflows; the second exceeds the limit of 1 and wraps.
            [0, 0],
            [10, 0],
            [20, 0],
            [30, 0],
            [0, 10],
            [10, 10],
            // A repeated ー overflows once, then あ wraps.
            [20, 10],
            [30, 10],
            [0, 20],
            [10, 20],
            [20, 20],
            // ー after a different character wraps without counting.
            [0, 30],
        ]
    );
    let text = messages.text(7).unwrap();
    assert_eq!(text.cursor, [10, 30]);
    assert_eq!(text.layout_scratch, [0, 0]);
}

/// Records `text` in window 0 / text 7 (area width `width`, 10-pixel glyphs, 40 ms
/// interval) after `setup`; returns whether the output succeeded.
fn ruby_layout(width: u32, setup: &[(u16, Vec<u32>)], text: &[u8]) -> (MessageHost, bool) {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, width, 100]);
    call(&mut code, 0x2143, &[10, 0]);
    for (command, words) in setup {
        call(&mut code, *command, words);
    }
    call(&mut code, 0x2152, &[0, 0, 0]);
    let mut data = text.to_vec();
    data.push(0);
    let script = script_with_text(&code, &data);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2 + setup.len());
    messages.set_record_output(true);
    pending(&mut vm);
    let ok = messages
        .try_record_output(
            &mut vm,
            &mut ActionInput::default(),
            wait_flags(false, false),
        )
        .is_ok();
    (messages, ok)
}
/// (code, position, size, delay) of every recorded glyph.
fn layout(messages: &MessageHost) -> Vec<(u16, [i32; 2], u32, u32)> {
    let glyphs = &messages.text(7).unwrap().glyphs;
    glyphs
        .iter()
        .map(|g| (g.code, g.position, g.size, g.delay_ms))
        .collect()
}

#[test]
fn recorded_output_spreads_narrow_ruby_and_re_times_the_last_glyph_per_iteration() {
    // あ{いう/えa}お with ruby size 4 and ruby spacing 3.
    let text = b"\x82\xa0{\x82\xa2\x82\xa4/\x82\xa6a}\x82\xa8";
    let setup = [
        (0x2114, vec![1, 7]),
        (0x2115, vec![4, 7]),
        (0x2116, vec![3, 7]),
    ];
    let (messages, ok) = ruby_layout(200, &setup, text);
    assert!(ok);
    // Line 0 holds a group, so the pen drops by the ruby size first. The annotation starts
    // at base start + gap / 2 = 10 + 8 (22.10: 10240 + 8192). The `/` re-times う to 120
    // and adds an interval; え starts at the group delay 40, then `a` and `}` re-time it.
    assert_eq!(
        layout(&messages),
        [
            (0x82a0, [0, 4], 10, 0),
            (0x82a2, [10, 4], 10, 40),
            (0x82a4, [20, 4], 10, 120),
            (0x82a6, [18, 0], 4, 120),
            (0x82a8, [30, 4], 10, 160),
        ]
    );
    let text_object = messages.text(7).unwrap();
    assert_eq!(text_object.cursor, [40, 4]);
    assert_eq!(text_object.ruby_lines[..2], [1, 0]);
    // Without ruby the annotation is skipped and nothing reserves its height.
    let (messages, ok) = ruby_layout(200, &[(0x2117, vec![1, 7])], text);
    assert!(ok);
    assert_eq!(
        layout(&messages),
        [
            (0x82a0, [0, 0], 10, 0),
            (0x82a2, [10, 0], 10, 40),
            (0x82a4, [20, 0], 10, 120),
            (0x82a8, [30, 0], 10, 160),
        ]
    );
    assert!(
        messages
            .text(7)
            .unwrap()
            .ruby_lines
            .iter()
            .all(|&flag| flag == 0)
    );
}

#[test]
fn recorded_output_centres_wide_ruby_and_wraps_groups_as_a_whole() {
    // ああ{あ/いうえ} with the default ruby size 12 on every line: centred at 20 + 10/2
    // − 36/2 = 7, stepping by the ruby size without spacing.
    let (messages, ok) = ruby_layout(
        200,
        &[(0x2114, vec![1, 7]), (0x2117, vec![1, 7])],
        b"\x82\xa0\x82\xa0{\x82\xa0/\x82\xa2\x82\xa4\x82\xa6}",
    );
    assert!(ok);
    let positions: Vec<_> = layout(&messages).iter().map(|g| (g.1, g.2)).collect();
    assert_eq!(
        positions,
        [
            ([0, 12], 10),
            ([10, 12], 10),
            ([20, 12], 10),
            ([7, 0], 12),
            ([19, 0], 12),
            ([31, 0], 12),
        ]
    );
    // Line end 20: the group wraps as one unit onto line 1, which 0x14005b360 marked, so the
    // new line reserves the ruby height; the base then runs past the line end.
    let (messages, ok) = ruby_layout(
        30,
        &[(0x2114, vec![1, 7]), (0x2115, vec![4, 7])],
        b"\x82\xa0\x82\xa0{\x82\xa2\x82\xa4/\x82\xa6}",
    );
    assert!(ok);
    let positions: Vec<_> = layout(&messages).iter().map(|g| g.1).collect();
    assert_eq!(positions, [[0, 0], [10, 0], [0, 14], [10, 14], [8, 10]]);
    let text_object = messages.text(7).unwrap();
    assert_eq!(text_object.ruby_lines[..3], [0, 1, 0]);
    assert_eq!(text_object.cursor, [20, 14]);
    // An annotation without double-byte characters divides by zero in the original.
    let (messages, ok) = ruby_layout(200, &[(0x2114, vec![1, 7])], b"{\x82\xa0/a}");
    assert!(!ok);
    assert!(messages.text(7).unwrap().glyphs.is_empty());
}

#[test]
fn recorded_output_rejects_unsupported_layout_without_partial_glyphs_or_stack_changes() {
    // Wrapping two characters per line reaches line 32, past the ruby-line flags; a group
    // closing its base before any glyph re-times an uninitialized pointer.
    for bytes in [
        &b"A\\q"[..],
        &b"{/ruby}"[..],
        &b"A{base"[..],
        &b"A{\x82\xa0/\x82"[..],
        &b"A\x81"[..],
        &[b'a'; 80][..],
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 50, 100]);
        call(&mut code, 0x2152, &[0, 0, 0]);
        let mut data = bytes.to_vec();
        data.push(0);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        messages.set_record_output(true);
        pending(&mut vm);
        let pc = vm.pc();
        let before = messages.text(7).cloned();
        let mut input = ActionInput::default();
        input.set(0, true, true, false).unwrap();
        for _ in 0..2 {
            assert!(
                messages
                    .try_record_output(&mut vm, &mut input, wait_flags(false, false))
                    .is_err()
            );
            assert_eq!(vm.pending_system(), Some((pc, 0x152)));
            assert_eq!(vm.stack().len_bytes(), 12);
            assert_eq!(vm.repeat_count(), 0);
            assert_eq!(messages.text(7), before.as_ref());
            assert!(input.pressed(0));
        }
    }
}

#[test]
fn recorded_output_rejects_missing_windows_bad_references_and_short_arguments() {
    for words in [
        vec![0, 0, 8],
        vec![0, 0, u32::MAX],
        vec![0, 0, 1],
        vec![100, 0, 0],
        vec![0, 0],
        vec![0],
        vec![],
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        call(&mut code, 0x2152, &words);
        let script = script_with_text(&code, b"A\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        messages.set_record_output(true);
        pending(&mut vm);
        let before = messages.text(7).cloned();
        let command = vm.pending_system();
        assert!(
            messages
                .try_record_output(
                    &mut vm,
                    &mut ActionInput::default(),
                    wait_flags(false, false)
                )
                .is_err()
        );
        assert_eq!(messages.text(7), before.as_ref());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), 4 * words.len());
    }
}

#[test]
fn text_measurement_counts_bytes_and_controls_without_moving_the_pen() {
    for (bytes, expected) in [
        (&b""[..], [0, 1, 0]),
        (&b"abcd"[..], [4, 1, 4]),
        (&b"\x82\xa0\x82\xa2AB"[..], [6, 1, 6]),
        (&b"abcd\\nx"[..], [4, 2, 1]),
        (&b"a\\cb\\tc\\wxd"[..], [4, 1, 4]),
        (&b"{\x82\xa0\x82\xa2/\x82\xa4}"[..], [4, 1, 4]),
        (&b"{ascii/ruby}"[..], [0, 1, 0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [20, 540, 1280, 180]);
        call(&mut code, 0x214f, &[3, 5, 0]);
        call(&mut code, 0x211f, &[0, 7]);
        let mut data = bytes.to_vec();
        data.push(0);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 2);
        let before = messages.text(7).cloned();
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 99).unwrap();
        }
        run(&mut vm, &mut messages, 1);
        assert_eq!(messages.text(7), before.as_ref());
        for (slot, value) in [
            (0, 1),
            (1, expected[0]),
            (2, expected[1]),
            (4, expected[2]),
            (3, 99),
            (5, 99),
            (6, 99),
        ] {
            assert_eq!(
                vm.state().system_integer(slot),
                Some(value),
                "{bytes:?} slot {slot}"
            );
        }
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn text_measurement_wrap_counts_follow_the_original_default_and_configured_branches() {
    for (bytes, rules, expected) in [
        (&b"abcde"[..], false, [5, 2, 5]),
        (&b"abcde"[..], true, [4, 2, 1]),
        (&b"\x82\xa0\x82\xa2\x82\xa4"[..], false, [6, 2, 6]),
        (&b"\x82\xa0\x82\xa2\x82\xa4"[..], true, [4, 2, 2]),
        (&b"{\x82\xa0\x82\xa2/x}\x82\xa4"[..], false, [6, 2, 6]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 30, 100]);
        call(&mut code, 0x2143, &[10, 0]);
        if rules {
            call(&mut code, 0x211e, &[0, 0, 0, 7]);
        }
        call(&mut code, 0x211f, &[1, 7]);
        let mut data = vec![0];
        data.extend(bytes);
        data.push(0);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 3 + usize::from(rules));
        assert_eq!(
            [1, 2, 4].map(|slot| vm.state().system_integer(slot).unwrap()),
            expected,
            "{bytes:?} rules={rules}"
        );
        assert_eq!(messages.text(7).unwrap().cursor, [0, 0]);
    }
}

#[test]
fn text_measurement_applies_hanging_and_repeated_character_limits() {
    for (hanging, inseparable, bytes, expected, scratch) in [
        (
            &b"\x81\x41"[..],
            &b""[..],
            &b"\x82\xa0\x81\x41"[..],
            [4, 1, 4],
            [1, 0x82a0],
        ),
        (
            &b""[..],
            &b"\x81\x41"[..],
            &b"\x81\x41\x81\x41\x81\x41"[..],
            [4, 2, 2],
            [0, 0],
        ),
        (
            &b""[..],
            &b"\x81\x41"[..],
            &b"\x82\xa0\x81\x41"[..],
            [2, 2, 2],
            [0, 0],
        ),
    ] {
        let mut data = hanging.to_vec();
        data.push(0);
        let second = data.len() as u32;
        data.extend(inseparable);
        data.push(0);
        let input = data.len() as u32;
        data.extend(bytes);
        data.push(0);
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 20, 100]);
        call(&mut code, 0x2143, &[10, 0]);
        call(&mut code, 0x211e, &[1, second, 0, 7]);
        call(&mut code, 0x211f, &[input, 7]);
        call(&mut code, 0x211f, &[input, 7]);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 3);
        for _ in 0..2 {
            run(&mut vm, &mut messages, 1);
            assert_eq!(
                [1, 2, 4].map(|slot| vm.state().system_integer(slot).unwrap()),
                expected
            );
            assert_eq!(messages.text(7).unwrap().layout_scratch, scratch);
        }
    }
}

#[test]
fn text_measurement_rejects_malformed_strings_transactionally() {
    for bytes in [
        &b"\\"[..],
        &b"\\q"[..],
        &b"\\w"[..],
        &b"\x81"[..],
        &b"{base"[..],
        &b"{base/ruby"[..],
        &b"{\x81"[..],
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        call(&mut code, 0x211f, &[0, 7]);
        let mut data = bytes.to_vec();
        data.push(0);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 99).unwrap();
        }
        let pc = vm.pc();
        let before = messages.text(7).cloned();
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err(), "{bytes:?}");
            assert_eq!(messages.text(7), before.as_ref());
            assert_eq!(vm.pending_system(), Some((pc, 0x11f)));
            assert_eq!(vm.stack().len_bytes(), 8);
            for i in 0..7 {
                assert_eq!(vm.state().system_integer(i), Some(99));
            }
        }
    }
}

#[test]
fn text_measurement_rejects_missing_text_bad_references_and_short_stacks() {
    for words in [
        vec![0, 12],
        vec![0, u32::MAX],
        vec![0, 2],
        vec![100, 7],
        vec![7],
        vec![],
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 100, 100]);
        call(&mut code, 0x211f, &words);
        let script = script_with_text(&code, b"text\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let pc = vm.pc();
        let before = messages.text(7).cloned();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(messages.text(7), before.as_ref());
        assert_eq!(vm.pending_system(), Some((pc, 0x11f)));
        assert_eq!(vm.stack().len_bytes(), 4 * words.len());
    }
}

#[test]
fn text_measurement_resolves_string_bank_and_preserves_stack_below_arguments() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0x12345678u32.to_le_bytes());
    call(&mut code, 0x211f, &[0x8000_0003, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    vm.state_mut().set_string(3, b"abc").unwrap();
    run(&mut vm, &mut messages, 1);
    assert_eq!(
        [1, 2, 4].map(|slot| vm.state().system_integer(slot).unwrap()),
        [3, 1, 3]
    );
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
}

#[test]
fn text_measurement_bounds_automatic_ruby_line_access_but_not_explicit_newlines() {
    for (bytes, success) in [(vec![b'a'; 33], false), (b"\\n".repeat(40), true)] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 100]);
        call(&mut code, 0x2143, &[10, 0]);
        call(&mut code, 0x211f, &[0, 7]);
        let mut data = bytes;
        data.push(0);
        let script = script_with_text(&code, &data);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 2);
        pending(&mut vm);
        let before = messages.text(7).cloned();
        let command = vm.pending_system();
        assert_eq!(messages.try_handle(&mut vm).is_ok(), success);
        assert_eq!(messages.text(7), before.as_ref());
        if success {
            assert_eq!(vm.state().system_integer(2), Some(41));
        } else {
            assert_eq!(vm.pending_system(), command);
            assert_eq!(vm.stack().len_bytes(), 8);
        }
    }
}

#[test]
fn line_break_rules_copy_both_strings_and_the_overflow_limit() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    // Text offsets: 0 = hanging set, 5 = inseparable set.
    call(&mut code, 0x211e, &[3, 5, 0, 7]);
    let script = script_with_text(&code, b"abcd\0xy\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    assert_eq!(
        messages.text(7).unwrap().line_break,
        Some(LineBreakRules {
            max_overflow: 3,
            hanging: b"abcd".to_vec(),
            inseparable: b"xy".to_vec(),
        })
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn text_object_commands_reject_missing_objects_and_long_strings() {
    let long = [b'a'; 64];
    let mut text = long.to_vec();
    text.push(0);
    let mut cases = vec![];
    for (command, words) in [
        (0x2115u16, vec![15, 6]),
        (0x2115, vec![15, 12]),
        (0x2115, vec![15, u32::MAX]),
        (0x211e, vec![3, 0, 0, 7]),
        (0x2117, vec![7]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        call(&mut code, command, &words);
        cases.push(code);
    }
    for code in cases {
        let script = script_with_text(&code, &text);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.text(7), Some(&created_text([0, 0], [10, 10])));
    }
}

#[test]
fn frame_animations_are_rebuilt_and_filled_per_entry() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2178, &[2, 0]);
    call(&mut code, 0x2180, &[2, 0]);
    call(&mut code, 0x21cf, &[2, 0]);
    // Entry 1: next 2, 100 ms, source (48, 320, 48, 48), origin (-1, 0x1_0002 -> 2).
    call(
        &mut code,
        0x21d0,
        &[0x1_0002, u32::MAX, 48, 48, 320, 48, 100, 2, 1, 2, 0],
    );
    call(&mut code, 0x21cf, &[2, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 4);
    let animation = |messages: &MessageHost| {
        messages.window(0).unwrap().object.children[&2]
            .draw
            .as_ref()
            .unwrap()
            .animation
            .clone()
    };
    assert_eq!(animation(&messages), Some(Box::default()));
    run(&mut vm, &mut messages, 1);
    let mut expected = Animation::default();
    expected.frames[1] = AnimationFrame {
        next: 2,
        words: [48, 320, 48, 48, -1, 2],
        duration: 100,
    };
    assert_eq!(animation(&messages).as_deref(), Some(&expected));
    // Rebuilding replaces the filled animation with an empty one.
    run(&mut vm, &mut messages, 1);
    assert_eq!(animation(&messages), Some(Box::default()));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn frame_entries_without_an_animation_are_ignored_and_bad_targets_rejected() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2180, &[u32::MAX, 0]);
    call(
        &mut code,
        0x21d0,
        &[0, 0, 48, 48, 0, 0, 100, 1, 0, u32::MAX, 0],
    );
    let ignored = script(&code);
    let mut vm = Machine::new(&ignored).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    let object = &messages.window(0).unwrap().object;
    assert_eq!(object.draw, Some(DrawState::default()));
    assert_eq!(vm.stack().len_bytes(), 0);

    let mut cases = vec![];
    for (draw, command, words) in [
        (false, 0x21cfu16, vec![u32::MAX, 0]),
        (true, 0x21cf, vec![5, 0]),
        (true, 0x21cf, vec![u32::MAX, 1]),
        (true, 0x21d0, vec![0, 0, 0, 0, 0, 0, 0, 0, 64, u32::MAX, 0]),
        (true, 0x21d0, vec![0, 0, 0, u32::MAX, 0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        if draw {
            call(&mut code, 0x2180, &[u32::MAX, 0]);
            call(&mut code, 0x21cf, &[u32::MAX, 0]);
        }
        call(&mut code, command, &words);
        cases.push((code, if draw { 3 } else { 1 }));
    }
    for (code, setup) in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, setup);
        let before = messages.window(0).unwrap().object.clone();
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.window(0).unwrap().object, before);
    }
}

/// Window 0 with child 2 animated by `frames` (next, duration, source x), all 48x48 at y 320.
fn animated(frames: &[(u32, u32, u32)], start: u32) -> Vec<u8> {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2178, &[2, 0]);
    call(&mut code, 0x2180, &[2, 0]);
    call(&mut code, 0x21cf, &[2, 0]);
    for (index, &(next, duration, x)) in frames.iter().enumerate() {
        call(
            &mut code,
            0x21d0,
            &[
                u32::MAX,
                0,
                48,
                48,
                320,
                x,
                duration,
                next,
                index as u32,
                2,
                0,
            ],
        );
    }
    call(&mut code, 0x21d1, &[start, 2, 0]);
    code
}
fn child_words(messages: &MessageHost) -> Vec<u32> {
    messages.window(0).unwrap().object.children[&2]
        .draw
        .as_ref()
        .unwrap()
        .words[2..8]
        .to_vec()
}

#[test]
fn playback_starts_at_a_frame_and_advances_by_clamped_frame_time() {
    let frames = [(1, 100, 0), (2, 100, 48), (0, 100, 96)];
    let script = script(&animated(&frames, 1));
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 4 + frames.len() + 1);
    assert_eq!(vm.stack().len_bytes(), 0);
    // Starting copies the frame; the origin (-1) keeps the draw state's value.
    assert_eq!(child_words(&messages), [48, 320, 48, 48, 0, 0]);
    messages.advance_frame(99);
    assert_eq!(child_words(&messages)[0], 48);
    messages.advance_frame(1);
    assert_eq!(child_words(&messages)[0], 96);
    // One 1000 ms frame counts as 200 ms: two steps, looping back to frame 0.
    messages.advance_frame(1000);
    assert_eq!(child_words(&messages)[0], 48);
    let animation = messages.window(0).unwrap().object.children[&2]
        .draw
        .as_ref()
        .unwrap()
        .animation
        .clone()
        .unwrap();
    assert_eq!(
        (
            animation.playing,
            animation.start,
            animation.current,
            animation.elapsed
        ),
        (true, 1, 1, 0)
    );
}

#[test]
fn ended_animations_are_freed_and_minus_two_disposes_the_child() {
    for (end, survives) in [(u32::MAX, true), ((-2i32) as u32, false)] {
        let script = script(&animated(&[(1, 50, 0), (end, 50, 48)], 0));
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 4 + 2 + 1);
        messages.advance_frame(50);
        assert_eq!(child_words(&messages)[0], 48);
        messages.advance_frame(49);
        assert!(
            messages.window(0).unwrap().object.children[&2]
                .draw
                .as_ref()
                .unwrap()
                .animation
                .is_some()
        );
        messages.advance_frame(1);
        let object = &messages.window(0).unwrap().object;
        assert_eq!(object.children.contains_key(&2), survives, "{end}");
        if survives {
            let child = &object.children[&2];
            assert!(child.draw.as_ref().unwrap().animation.is_none());
            assert_eq!(child.draw.as_ref().unwrap().words[2], 48);
        }
    }
}

#[test]
fn zero_length_loops_stop_after_one_pass_per_frame() {
    // Unwritten frames are zero: frame 0 loops to itself with no duration.
    let script = script(&animated(&[], 0));
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 4 + 1);
    messages.advance_frame(16);
    assert_eq!(child_words(&messages), [0, 0, 0, 0, 0, 0]);
}

#[test]
fn playback_rejects_out_of_table_frames() {
    for code in [animated(&[], 64), {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 540, 1280, 180]);
        call(&mut code, 0x2178, &[2, 0]);
        call(&mut code, 0x2180, &[2, 0]);
        call(&mut code, 0x21cf, &[2, 0]);
        call(&mut code, 0x21d0, &[0, 0, 48, 48, 0, 0, 100, 64, 0, 2, 0]);
        code
    }] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 4);
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn wait_cursor_mode_keeps_offsets_for_modes_one_and_three_only() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    // Mode 0x1_0003 truncates to 3; the offsets truncate to 16 bits.
    call(&mut code, 0x2151, &[0x1_0004, u32::MAX - 1, 0x1_0003, 0]);
    call(&mut code, 0x2151, &[9, 9, 7, 0]);
    call(&mut code, 0x2151, &[9, 9, 2, 0]);
    call(&mut code, 0x2151, &[5, 6, 1, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    let state = |messages: &MessageHost| {
        let window = messages.window(0).unwrap();
        (window.cursor_mode, window.cursor_offset)
    };
    run(&mut vm, &mut messages, 2);
    assert_eq!(state(&messages), (3, [-2, 4]));
    run(&mut vm, &mut messages, 1);
    assert_eq!(state(&messages), (7, [-2, 4]));
    run(&mut vm, &mut messages, 1);
    assert_eq!(state(&messages), (2, [0, 0]));
    run(&mut vm, &mut messages, 1);
    assert_eq!(state(&messages), (1, [6, 5]));
    assert_eq!(vm.stack().len_bytes(), 0);
    for words in [vec![0, 0, 3, 1], vec![0, 3, 0]] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        call(&mut code, 0x2151, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x151));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(state(&messages), (0, [0, 0]));
    }
}

#[test]
fn window_opacity_caps_at_255_and_keeps_negative_values() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2178, &[6, 0]);
    call(&mut code, 0x2180, &[6, 0]);
    call(&mut code, 0x2186, &[128, 6, 0]);
    call(&mut code, 0x2186, &[300, 6, 0]);
    call(&mut code, 0x2186, &[u32::MAX, 6, 0]);
    call(&mut code, 0x2186, &[1, u32::MAX, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    let opacity = |messages: &MessageHost| {
        messages.window(0).unwrap().object.children[&6]
            .draw
            .as_ref()
            .unwrap()
            .words[17]
    };
    run(&mut vm, &mut messages, 4);
    assert_eq!(opacity(&messages), 128);
    run(&mut vm, &mut messages, 1);
    assert_eq!(opacity(&messages), 255);
    run(&mut vm, &mut messages, 1);
    assert_eq!(opacity(&messages), u32::MAX);
    // The window object itself has no draw state.
    pending(&mut vm);
    assert!(messages.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x186));
}

#[test]
fn switch_state_one_position_and_text_clearing() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x215a, &[u32::MAX, 40, 0]);
    call(&mut code, 0x2145, &[100, 900, 5, 6, 0]);
    call(&mut code, 0x214f, &[u32::MAX, 12, 0]);
    call(&mut code, 0x214a, &[0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    let window = messages.window(0).unwrap();
    assert_eq!(window.switch_positions, [[0, 540], [40, -1]]);
    assert_eq!(window.position, [0, 540]);
    run(&mut vm, &mut messages, 1);
    let before = messages.text(7).unwrap().clone();
    assert_eq!(before.cursor, [6, 5]);
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.text(7).unwrap().cursor, [12, -1]);
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.text(7), Some(&before));
    assert_eq!(vm.stack().len_bytes(), 0);
    for (command, words) in [
        (0x214au16, vec![1]),
        (0x215a, vec![0, 1]),
        (0x215a, vec![0]),
    ] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        call(&mut code, command, &words);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn window_cursor_is_absolute_signed_unclamped_and_isolated_from_layout() {
    let points = [[12, 34], [u32::MAX, 0x8000_0000], [9000, 8000], [0, 0]];
    let mut code = vec![];
    create(&mut code, 0, 7, [30, 540, 1280, 180]);
    create(&mut code, 1, 2, [50, 60, 100, 20]);
    for [x, y] in points {
        call(&mut code, 0x214f, &[y, x, 0]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    let window = messages.window(0).cloned();
    let other = messages.text(2).cloned();
    let initial = messages.text(7).unwrap().clone();
    vm.state_mut().set_system_integer(0, 91).unwrap();
    for [x, y] in points {
        let mut expected = initial.clone();
        expected.cursor = [x as i32, y as i32];
        pending(&mut vm);
        let pc = vm.pc();
        assert!(messages.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(91));
        assert_eq!(messages.text(7), Some(&expected));
        assert_eq!(messages.window(0), window.as_ref());
        assert_eq!(messages.text(2), other.as_ref());
    }
}

#[test]
fn window_cursor_errors_keep_the_pending_command_and_all_arguments() {
    for (exists, words) in [
        (false, vec![5, 4, 0]),
        (true, vec![5, 4, 8]),
        (true, vec![5, 4, u32::MAX]),
        (true, vec![5, 0]),
        (true, vec![0]),
        (true, vec![]),
    ] {
        let mut code = vec![];
        if exists {
            create(&mut code, 0, 7, [0, 0, 100, 20]);
        }
        call(&mut code, 0x214f, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, usize::from(exists));
        vm.state_mut().set_system_integer(0, 91).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        let window = messages.window(0).cloned();
        let text = messages.text(7).cloned();
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err());
            assert_eq!(vm.pc(), pc);
            assert_eq!(vm.pending_system(), Some((pc, 0x14f)));
            assert_eq!(vm.stack().len_bytes(), words.len() * 4);
            for (i, value) in words.iter().rev().enumerate() {
                assert_eq!(vm.stack().peek_u32(4 * (i + 1)).unwrap(), *value);
            }
            assert_eq!(messages.window(0), window.as_ref());
            assert_eq!(messages.text(7), text.as_ref());
            assert_eq!(vm.state().system_integer(0), Some(91));
        }
    }
}

#[test]
fn showing_and_hiding_a_window_toggles_window_text_and_object() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2180, &[u32::MAX, 0]);
    call(&mut code, 0x2150, &[600, 30, 0, 0]);
    call(&mut code, 0x214e, &[1, 0]);
    call(&mut code, 0x214e, &[5, 0]);
    call(&mut code, 0x214e, &[0, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    assert!(!messages.window(0).unwrap().visible);
    run(&mut vm, &mut messages, 1);
    let window = messages.window(0).unwrap();
    assert!(window.visible && window.object.shown);
    let text = messages.text(7).unwrap();
    // Showing moves the text object to the window position, which it already shares here.
    assert!(text.visible);
    assert_eq!(text.position, [30, 600]);
    // Already shown: nothing changes.
    run(&mut vm, &mut messages, 1);
    assert!(messages.window(0).unwrap().visible);
    run(&mut vm, &mut messages, 1);
    let window = messages.window(0).unwrap();
    assert!(!window.visible && !window.object.shown);
    assert!(!messages.text(7).unwrap().visible);
    assert_eq!(vm.stack().len_bytes(), 0);
    for words in [vec![1, 3], vec![1, 8], vec![1, u32::MAX], vec![0]] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        call(&mut code, 0x214e, &words);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x14e));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn window_presence_and_temporary_hiding() {
    let mut code = vec![];
    create(&mut code, 2, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x214b, &[2]);
    call(&mut code, 0x214b, &[3]);
    // Hiding a window that is not shown does nothing.
    call(&mut code, 0x214c, &[1, 2]);
    call(&mut code, 0x214e, &[1, 2]);
    call(&mut code, 0x214c, &[9, 2]);
    call(&mut code, 0x214c, &[0, 2]);
    call(&mut code, 0x214c, &[1, 2]);
    // Hiding the window clears the temporary flag.
    call(&mut code, 0x214e, &[0, 2]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    assert_eq!(vm.state().system_integer(0), Some(1));
    run(&mut vm, &mut messages, 1);
    assert_eq!(vm.state().system_integer(0), Some(0));
    run(&mut vm, &mut messages, 1);
    assert!(!messages.window(2).unwrap().hidden);
    run(&mut vm, &mut messages, 2);
    let window = messages.window(2).unwrap();
    assert!(window.visible && window.hidden && !window.object.shown);
    assert!(!messages.text(7).unwrap().visible);
    run(&mut vm, &mut messages, 1);
    let window = messages.window(2).unwrap();
    assert!(!window.hidden && window.object.shown);
    assert!(messages.text(7).unwrap().visible);
    run(&mut vm, &mut messages, 2);
    let window = messages.window(2).unwrap();
    assert!(!window.visible && !window.hidden && !window.object.shown);
    assert_eq!(vm.stack().len_bytes(), 0);
    for (command, words) in [
        (0x214bu16, vec![8]),
        (0x214b, vec![u32::MAX]),
        (0x214c, vec![1, 3]),
        (0x214c, vec![2]),
    ] {
        let mut code = vec![];
        create(&mut code, 2, 7, [0, 0, 10, 10]);
        call(&mut code, command, &words);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err(), "{command:x}");
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), Some(9));
    }
}

#[test]
fn pointer_hit_test_is_relative_to_the_window_and_half_open() {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    // Rectangle (1100, 20) 100x50 inside the window at (0, 540).
    for _ in 0..6 {
        call(&mut code, 0x215f, &[50, 100, 20, 1100, 0]);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    let mut input = ActionInput::default();
    let mut hits = vec![];
    for (x, y) in [
        (1100, 560),
        (1199, 609),
        (1200, 560),
        (1100, 610),
        (1099, 600),
        (-1, -1),
    ] {
        input.set_pointer(x, y);
        pending(&mut vm);
        assert!(!messages.try_handle(&mut vm).unwrap());
        assert!(messages.try_handle_pointer(&mut vm, &input).unwrap());
        hits.push(vm.state().system_integer(0));
    }
    assert_eq!(hits, [Some(1), Some(1), Some(0), Some(0), Some(0), Some(0)]);
    assert_eq!(vm.stack().len_bytes(), 0);
    for words in [vec![0, 0, 0, 0, 1], vec![0, 0, 0, 0, 8], vec![0, 0, 0, 0]] {
        let mut code = vec![];
        create(&mut code, 0, 7, [0, 0, 10, 10]);
        call(&mut code, 0x215f, &words);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle_pointer(&mut vm, &input).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x15f));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn window_scale_snaps_near_one_and_preserves_other_float_bits() {
    let values = [
        0.95f32.to_bits(),
        1.05f32.to_bits(),
        1.0f32.to_bits(),
        0.95f32.to_bits() - 1,
        1.05f32.to_bits() + 1,
        2.0f32.to_bits(),
        (-0.0f32).to_bits(),
        (-2.0f32).to_bits(),
        f32::INFINITY.to_bits(),
        0x7fc01234,
    ];
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    call(&mut code, 0x2180, &[u32::MAX, 0]);
    for raw in values {
        call(&mut code, 0x20ec, &[raw, 0]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    vm.state_mut().set_system_integer(0, 11).unwrap();
    run(&mut vm, &mut messages, 2);
    let initial = messages.window(0).unwrap().clone();
    for (i, raw) in values.into_iter().enumerate() {
        run(&mut vm, &mut messages, 1);
        let expected = if i < 3 { 1.0f32.to_bits() } else { raw };
        let mut window = initial.clone();
        window.object.draw.as_mut().unwrap().words[18..20].fill(expected);
        assert_eq!(messages.window(0), Some(&window));
        assert_eq!(messages.text(7).unwrap().scale_bits, expected);
        assert_eq!(vm.state().system_integer(0), Some(11));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn window_glyph_parameter_preserves_raw_bits_and_only_changes_the_linked_text() {
    let values = [1, 0x8000_0000, u32::MAX, 0];
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    create(&mut code, 1, 2, [10, 20, 100, 30]);
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0x1234u32.to_le_bytes());
    for value in values {
        call(&mut code, 0x2155, &[value, 0]);
    }
    call(&mut code, 0x2155, &[99, 0]);
    create(&mut code, 0, 7, [0, 540, 1280, 180]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 2);
    vm.state_mut().set_system_integer(0, 42).unwrap();
    let window = messages.window(0).unwrap().clone();
    let other = messages.text(2).unwrap().clone();
    let initial = messages.text(7).unwrap().clone();
    assert_eq!(initial.glyph_parameter, 0);
    for value in values {
        let mut expected = initial.clone();
        expected.glyph_parameter = value;
        run(&mut vm, &mut messages, 1);
        assert_eq!(messages.text(7), Some(&expected));
        assert_eq!(messages.text(2), Some(&other));
        assert_eq!(messages.window(0), Some(&window));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x1234);
        assert_eq!(vm.state().system_integer(0), Some(42));
    }
    run(&mut vm, &mut messages, 2);
    assert_eq!(messages.text(7), Some(&initial));
}

#[test]
fn window_glyph_parameter_errors_preserve_pending_command_stack_and_objects() {
    for (exists, words) in [
        (false, vec![5, 0]),
        (true, vec![5, 8]),
        (true, vec![5, u32::MAX]),
        (true, vec![0]),
        (true, vec![]),
    ] {
        let mut code = vec![];
        if exists {
            create(&mut code, 0, 7, [0, 0, 100, 20]);
        }
        call(&mut code, 0x2155, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, usize::from(exists));
        vm.state_mut().set_system_integer(0, 42).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        let window = messages.window(0).cloned();
        let text = messages.text(7).cloned();
        for _ in 0..2 {
            assert!(messages.try_handle(&mut vm).is_err());
            assert_eq!(vm.pc(), pc);
            assert_eq!(vm.pending_system(), Some((pc, 0x155)));
            assert_eq!(vm.stack().len_bytes(), words.len() * 4);
            for (i, value) in words.iter().rev().enumerate() {
                assert_eq!(vm.stack().peek_u32(4 * (i + 1)).unwrap(), *value);
            }
            assert_eq!(messages.window(0), window.as_ref());
            assert_eq!(messages.text(7), text.as_ref());
            assert_eq!(vm.state().system_integer(0), Some(42));
        }
    }
}

#[test]
fn window_scale_rejects_bad_targets_and_truncated_arguments_without_mutation() {
    for (create_window, draw, words) in [
        (false, false, vec![1.0f32.to_bits(), 0]),
        (true, false, vec![1.0f32.to_bits(), 0]),
        (true, true, vec![1.0f32.to_bits(), 8]),
        (true, true, vec![1.0f32.to_bits(), u32::MAX]),
        (true, true, vec![0]),
    ] {
        let mut code = vec![];
        if create_window {
            create(&mut code, 0, 7, [0, 0, 100, 100]);
        }
        if draw {
            call(&mut code, 0x2180, &[u32::MAX, 0]);
        }
        call(&mut code, 0x20ec, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(
            &mut vm,
            &mut messages,
            usize::from(create_window) + usize::from(draw),
        );
        pending(&mut vm);
        let pc = vm.pc();
        let stack = vm.stack().len_bytes();
        let window = messages.window(0).cloned();
        let text = messages.text(7).cloned();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0xec)));
        assert_eq!(vm.stack().len_bytes(), stack);
        assert_eq!(messages.window(0), window.as_ref());
        assert_eq!(messages.text(7), text.as_ref());
    }
}

#[test]
fn window_image_release_rejects_missing_slots_and_short_stacks_without_mutation() {
    for args in [vec![], vec![8], vec![u32::MAX], vec![1]] {
        let mut code = vec![];
        create(&mut code, 0, 0, [10, 20, 100, 40]);
        call(&mut code, 0x2171, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        let before = messages.window(0).unwrap().clone();
        let text = messages.text(0).unwrap().clone();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x171)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(messages.window(0), Some(&before));
        assert_eq!(messages.text(0), Some(&text));
    }
}

#[test]
fn window_object_presence_ignores_images_visibility_and_draw_state_and_tracks_children() {
    let mut code = vec![];
    create(&mut code, 0, 0, [10, 20, 100, 40]);
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0x12345678u32.to_le_bytes());
    let first = [
        (u32::MAX, 1),
        (0x80000000, 1),
        (64, 0),
        (768, 0),
        (65536, 0),
    ];
    for (child, _) in first {
        call(&mut code, 0x217b, &[child, 0]);
    }
    call(&mut code, 0x2178, &[64, 0]);
    call(&mut code, 0x217b, &[64, 0]);
    call(&mut code, 0x2179, &[64, 0]);
    call(&mut code, 0x217b, &[64, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    vm.state_mut().set_system_integer(1, 99).unwrap();
    let check = |vm: &mut Machine<'_>, messages: &mut MessageHost, result| {
        let window = messages.window(0).unwrap().clone();
        let text = messages.text(0).unwrap().clone();
        pending(vm);
        let pc = vm.pc();
        assert!(messages.try_handle(vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.state().system_integer(0), Some(result));
        assert_eq!(vm.state().system_integer(1), Some(99));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
        assert_eq!(messages.window(0), Some(&window));
        assert_eq!(messages.text(0), Some(&text));
    };
    for (_, result) in first {
        check(&mut vm, &mut messages, result);
    }
    run(&mut vm, &mut messages, 1);
    assert!(!messages.window(0).unwrap().object.children[&64].shown);
    check(&mut vm, &mut messages, 1);
    run(&mut vm, &mut messages, 1);
    check(&mut vm, &mut messages, 0);
}
#[test]
fn window_object_query_rejects_missing_windows_and_short_arguments_without_mutation() {
    for args in [vec![], vec![0], vec![0, 1], vec![0, 8], vec![0, u32::MAX]] {
        let mut code = vec![];
        create(&mut code, 0, 0, [10, 20, 100, 40]);
        call(&mut code, 0x217b, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, 1);
        vm.state_mut().set_system_integer(0, 17).unwrap();
        let before = messages.window(0).unwrap().clone();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(messages.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x17b)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(vm.state().system_integer(0), Some(17));
        assert_eq!(messages.window(0), Some(&before));
    }
}

#[test]
fn direct_text_setters_reuse_window_setter_rules_and_pop_their_words() {
    let mut code = vec![];
    call(&mut code, 0x2100, &[0, 0, 1]);
    call(&mut code, 0x2102, &[720, 1280, 1]);
    call(&mut code, 0x2103, &[50, 400, 20, 10, 1]);
    call(&mut code, 0x2105, &[2, u32::MAX, 32, 1]);
    call(&mut code, 0x2105, &[u32::MAX, 3, 0, 1]);
    call(&mut code, 0x2107, &[0x0100_0000, 0x123456, 1]);
    call(&mut code, 0x210c, &[7, 8, 1]);
    call(&mut code, 0x210d, &[350, 1, 1]);
    call(&mut code, 0x210e, &[0, 1]);
    call(&mut code, 0x210f, &[3, 1]);
    call(&mut code, 0x2110, &[8, 1]);
    call(&mut code, 0x2113, &[1, 1]);
    call(&mut code, 0x211a, &[0x272727, 1]);
    call(&mut code, 0x2104, &[1]);
    let script = script_with_text(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.text(1).unwrap().outline_mode, 2);
    vm.state_mut().set_system_integer(0, 9).unwrap();
    run(&mut vm, &mut messages, 12);
    let text = messages.text(1).unwrap();
    assert_eq!(text.position, [1280, 720]);
    assert_eq!(text.area, [10, 20, 400, 50]);
    // Size only when positive; each spacing only when nonnegative.
    assert_eq!((text.glyph_size, text.spacing), (32, [3, 2]));
    assert_eq!((text.color, text.decoration_color), (0x123456, 0xff_ffff));
    assert_eq!(text.cursor, [8, 7]);
    assert_eq!(text.reveal, [1, 350]);
    assert_eq!((text.char_delay, text.decoration, text.layer), (0, 3, 8));
    assert_eq!((text.outline_mode, text.glyph_parameter), (1, 0x272727));
    run(&mut vm, &mut messages, 1);
    assert_eq!(messages.text(1).unwrap().cursor, [10, 20]);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
}

#[test]
fn direct_text_destruction_and_rejected_targets_keep_state_atomic() {
    let mut code = vec![];
    call(&mut code, 0x2100, &[0, 0, 4]);
    call(&mut code, 0x2101, &[4]);
    call(&mut code, 0x2101, &[4]);
    let destroying = script_with_text(&code, b"\0");
    let mut vm = Machine::new(&destroying).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    assert!(messages.text(4).is_none());
    assert!(messages.font_present(4));
    assert_eq!(vm.stack().len_bytes(), 0);
    // Missing, out-of-range and negative targets, and freeing a window's text object.
    for (command, words, window) in [
        (0x2102u16, vec![0, 0, 4], false),
        (0x2113, vec![1, 12], false),
        (0x2104, vec![u32::MAX], false),
        (0x2101, vec![12], false),
        (0x2101, vec![3], true),
    ] {
        let mut code = vec![];
        if window {
            create(&mut code, 0, 3, [0, 0, 10, 10]);
        }
        call(&mut code, command, &words);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        run(&mut vm, &mut messages, usize::from(window));
        let before = messages.text(3).cloned();
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err(), "{command:x}");
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.text(3).cloned(), before);
    }
}

/// Text object 1 with a 20-pixel glyph, no spacing, area (10, 5, 70, 100), outline mode 0.
fn direct_output(code: &mut Vec<u8>) {
    call(code, 0x2100, &[0, 0, 1]);
    call(code, 0x2103, &[100, 70, 5, 10, 1]);
    call(code, 0x2105, &[0, 0, 20, 1]);
    call(code, 0x2113, &[0, 1]);
}

#[test]
fn direct_half_width_text_uses_backend_ink_extent_with_minimum_half_size_and_wraps() {
    #[derive(Debug)]
    struct Metrics;
    impl cmvs_rs::message::GlyphMetrics for Metrics {
        fn ink_width(&self, code: u16, _: u32) -> Result<u32, &'static str> {
            match code {
                0x20 => Ok(0),
                0x57 => Ok(17),
                _ => Err("missing synthetic glyph"),
            }
        }
    }
    let mut code = vec![];
    direct_output(&mut code);
    call(&mut code, 0x210b, &[1, 1]);
    let script = script_with_text(&code, b"\0W W W\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    messages.set_record_output(true);
    messages.set_glyph_metrics(Box::new(Metrics));
    run(&mut vm, &mut messages, 5);
    let text = messages.text(1).unwrap();
    assert_eq!(
        text.glyphs.iter().map(|g| g.position).collect::<Vec<_>>(),
        [[10, 5], [27, 5], [37, 5], [54, 5], [10, 25]]
    );
    assert_eq!(text.cursor, [27, 25]);
}

#[test]
fn device_window_creation_matches_explicit_viewport_and_cleans_six_words() {
    let mut code = vec![];
    call(&mut code, 0x2140, &[60, 70, 20, 10, 3, 6]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(messages.window(6).unwrap().text, 3);
    assert_eq!(messages.text(3), Some(&created_text([10, 20], [70, 60])));
}

#[test]
fn polled_window_output_appends_only_on_initialization_and_never_yields() {
    let mut code = vec![];
    create(&mut code, 6, 6, [0, 0, 300, 100]);
    for initialize in [0, 1, 1] {
        call(&mut code, 0x2159, &[initialize, 0, 8191, 6]);
    }
    let script = script_with_text(&code, b"AB\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    messages.set_record_output(true);
    run(&mut vm, &mut messages, 1);
    for expected in [0, 0, 1] {
        pending(&mut vm);
        assert!(
            messages
                .try_record_poll(&mut vm, cmvs_rs::message_flags::MessageFlags::default())
                .unwrap()
        );
        assert!(vm.pending_system().is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(messages.text(6).unwrap().glyphs.len(), 2);
        assert_eq!(vm.state().system_integer(0), Some(expected));
        if expected == 0 {
            messages.advance_frame(if messages.text(6).unwrap().glyphs[0].elapsed_ms == 0 {
                20
            } else {
                400
            });
        }
    }
}

#[test]
fn direct_text_output_places_glyphs_at_once_and_follows_its_own_layout() {
    // 4 x あ wraps before the fourth; \n; 0x814a; a ruby group draws only its base. The
    // leading NUL is 0x2100's empty face name; the output string starts at offset 1.
    let text = b"\0\x82\xa0\x82\xa0\x82\xa0\x82\xa0\\n\x81\x4a{\x82\xa0/\x82\xa2}\0";
    let mut code = vec![];
    direct_output(&mut code);
    call(&mut code, 0x210b, &[1, 1]);
    let script = script_with_text(&code, text);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 4);
    pending(&mut vm);
    // Without the research output policy the command stays a barrier.
    assert!(!messages.try_handle(&mut vm).unwrap());
    messages.set_record_output(true);
    assert!(messages.try_handle(&mut vm).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
    let text = messages.text(1).unwrap();
    let placed: Vec<_> = text
        .glyphs
        .iter()
        .map(|g| (g.code, g.position, g.outline[0]))
        .collect();
    assert_eq!(
        placed,
        [
            (0x82a0, [10, 5], 0),
            (0x82a0, [30, 5], 0),
            (0x82a0, [50, 5], 0),
            (0x82a0, [10, 25], 0),
            (0x814a, [6, 45], 0),
            (0x82a0, [16, 45], 0),
        ]
    );
    assert!(
        text.glyphs
            .iter()
            .all(|g| g.shown && !g.active && g.delay_ms == 0 && g.opacity == 255)
    );
    assert_eq!(text.cursor, [36, 45]);
}

#[test]
fn direct_text_output_rejects_gdi_widths_and_stalling_escapes_atomically() {
    for bad in [
        &b"\0\x82\xa0A\0"[..],
        b"\0\\x\0",
        b"\0\\w\0",
        b"\0{\x82\xa0\0",
    ] {
        let mut code = vec![];
        direct_output(&mut code);
        call(&mut code, 0x210b, &[1, 1]);
        let script = script_with_text(&code, bad);
        let mut vm = Machine::new(&script).unwrap();
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        run(&mut vm, &mut messages, 4);
        let before = messages.text(1).cloned();
        pending(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(messages.try_handle(&mut vm).is_err(), "{bad:?}");
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x10b));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(messages.text(1).cloned(), before);
    }
}

/// Records 0x2152 output of `bytes` into window 0 / text 7 (27-pixel glyphs, 100 delay so
/// a 40 ms reveal interval) and returns the text object.
fn record(bytes: &[u8]) -> TextObject {
    let mut code = vec![];
    create(&mut code, 0, 7, [0, 0, 100, 100]);
    call(&mut code, 0x2152, &[0, 0, 0]);
    let program = script_with_text(&code, bytes);
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 1);
    messages.set_record_output(true);
    pending(&mut vm);
    let mut input = ActionInput::default();
    messages
        .try_record_output(&mut vm, &mut input, wait_flags(false, false))
        .unwrap();
    messages.text(7).unwrap().clone()
}

#[test]
fn wait_escapes_switch_instant_reveal_and_tab_restarts_the_delay() {
    let text = record(b"AB\\wcCD\\wsE\0");
    let timing: Vec<_> = text
        .glyphs
        .iter()
        .map(|g| (g.code, g.delay_ms, g.active, g.shown))
        .collect();
    assert_eq!(
        timing,
        [
            (u16::from(b'A'), 0, true, false),
            (u16::from(b'B'), 40, true, false),
            // `\wc` shows glyphs at once and restarts the delay; `\ws` resumes revealing.
            (u16::from(b'C'), 0, false, true),
            (u16::from(b'D'), 0, false, true),
            (u16::from(b'E'), 0, true, false),
        ]
    );
    let text = record(b"AB\\tC\0");
    let delays: Vec<_> = text.glyphs.iter().map(|g| g.delay_ms).collect();
    assert_eq!(delays, [0, 40, 0]);
    // Unknown `\w` selectors are skipped without changing the mode.
    let text = record(b"A\\wxB\0");
    let delays: Vec<_> = text.glyphs.iter().map(|g| g.delay_ms).collect();
    assert_eq!(delays, [0, 40]);
}

#[test]
fn clear_escape_drops_glyphs_and_returns_the_pen_to_the_area_origin() {
    let text = record(b"AB\\nC\\cD\0");
    let placed: Vec<_> = text
        .glyphs
        .iter()
        .map(|g| (g.code, g.position, g.delay_ms))
        .collect();
    // The delay keeps running across the clear.
    assert_eq!(placed, [(u16::from(b'D'), [0, 0], 120)]);
    assert_eq!(text.cursor, [13, 0]);
}

#[test]
fn window_button_hit_test_reports_hover_click_and_hold_and_consumes_releases() {
    let mut code = vec![];
    create(&mut code, 0, 0, [100, 200, 100, 100]);
    // Button 0 covers x 110..=140, y 220..=260.
    call(&mut code, 0x2191, &[40, 30, 20, 10, 0, 0]);
    call(&mut code, 0x2192, &[0, 0]);
    for _ in 0..5 {
        call(&mut code, 0x2194, &[0]);
    }
    call(&mut code, 0x2194, &[8]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 3);
    let mut input = ActionInput::default();
    let mut query = |pointer: [i32; 2], flags: [bool; 3], input: &mut ActionInput| {
        input.set_pointer(pointer[0], pointer[1]);
        input.set(0, flags[0], flags[1], flags[2]).unwrap();
        pending(&mut vm);
        assert!(messages.try_handle_buttons(&mut vm, input).unwrap());
        let results: Vec<u32> = (4..7)
            .map(|i| vm.state().system_integer(i).unwrap())
            .collect();
        (results, messages.window(0).unwrap().pressed_button)
    };
    // A press on the inclusive far corner records the button and reports holding it.
    assert_eq!(
        query([140, 260], [true, true, false], &mut input),
        (vec![0, 0, 1], 0)
    );
    // Releasing over it is a click; the release is consumed and the record cleared.
    assert_eq!(
        query([140, 260], [false, false, true], &mut input),
        (vec![0, 1, 0], -1)
    );
    assert!(!input.released(0));
    assert_eq!(
        query([141, 261], [false, false, false], &mut input),
        (vec![u32::MAX, 0, 0], -1)
    );
    // Pressed inside, released elsewhere: no click, but the release is still consumed.
    assert_eq!(
        query([110, 220], [true, true, false], &mut input),
        (vec![0, 0, 1], 0)
    );
    assert_eq!(
        query([200, 300], [false, false, true], &mut input),
        (vec![u32::MAX, 0, 0], -1)
    );
    assert!(!input.released(0));
    // Slot 8 only sets an engine flag in the original; the command stays pending here.
    pending(&mut vm);
    assert!(messages.try_handle_buttons(&mut vm, &mut input).is_err());
    assert!(vm.pending_system().is_some());
}

#[test]
fn window_button_state_commands_switch_the_child_source_once() {
    let mut code = vec![];
    create(&mut code, 7, 10, [0, 0, 1280, 54]);
    call(&mut code, 0x2178, &[16, 7]);
    call(&mut code, 0x2180, &[16, 7]);
    call(&mut code, 0x2190, &[0, 0x1_0005, 52, 94, 0, 96, 1, 0, 7]);
    call(&mut code, 0x2192, &[0, 7]);
    call(&mut code, 0x2195, &[0, 7]);
    call(&mut code, 0x2195, &[1, 7]);
    call(&mut code, 0x2196, &[1, 0, 7]);
    call(&mut code, 0x2197, &[0, 7]);
    call(&mut code, 0x2196, &[5, 0, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 5);
    let result = |vm: &mut Machine<'_>, messages: &mut MessageHost| {
        pending(vm);
        assert!(messages.try_handle(vm).unwrap());
        vm.state().system_integer(0)
    };
    assert_eq!(result(&mut vm, &mut messages), Some(1));
    assert_eq!(result(&mut vm, &mut messages), Some(0));
    result(&mut vm, &mut messages);
    let window = messages.window(7).unwrap();
    let words = window.object.children[&16].draw.as_ref().unwrap().words;
    assert_eq!(words[2..6], [96, 0, 94, 52]);
    assert_eq!(window.buttons[0].state_word, 1);
    assert_eq!(result(&mut vm, &mut messages), Some(1));
    // State 5 lies past the five state records.
    pending(&mut vm);
    let bytes = vm.stack().len_bytes();
    assert!(messages.try_handle(&mut vm).is_err());
    assert_eq!(vm.stack().len_bytes(), bytes);
}

#[test]
fn window_draw_query_and_hidden_flag_report_window_state() {
    let mut code = vec![];
    create(&mut code, 0, 0, [0, 0, 100, 100]);
    call(&mut code, 0x2180, &[u32::MAX, 0]);
    call(&mut code, 0x2186, &[77, u32::MAX, 0]);
    call(&mut code, 0x2183, &[20, 10, u32::MAX, 0]);
    call(&mut code, 0x214e, &[1, 0]);
    call(&mut code, 0x214c, &[1, 0]);
    call(&mut code, 0x218d, &[u32::MAX, 0]);
    call(&mut code, 0x214d, &[0]);
    call(&mut code, 0x218d, &[3, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    run(&mut vm, &mut messages, 7);
    let state = vm.state();
    let words = messages
        .window(0)
        .unwrap()
        .object
        .draw
        .as_ref()
        .unwrap()
        .words;
    assert_eq!(
        (0..4)
            .map(|i| state.system_integer(i).unwrap())
            .collect::<Vec<_>>(),
        [77, 10, 20, words[10]]
    );
    assert_eq!(state.system_float_bits(8), Some(words[18]));
    assert_eq!(state.system_float_bits(9), Some(words[19]));
    run(&mut vm, &mut messages, 1);
    assert_eq!(vm.state().system_integer(0), Some(1));
    // A missing child sets an engine flag in the original.
    pending(&mut vm);
    assert!(messages.try_handle(&mut vm).is_err());
}

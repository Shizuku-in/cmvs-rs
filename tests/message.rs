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
        scale_bits: 1.0f32.to_bits(),
        logical_size: [1280, 720],
        position,
        area: [0, 0, size[0] as i32, size[1] as i32],
        cursor: [0, 0],
        ruby_lines: [0; 32],
        layout_scratch: [0; 2],
        glyph_size: 27,
        spacing: [0, 0],
        color: 0xff_ffff,
        decoration_color: 0xff_ffff,
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
    }
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
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
fn device_sized_creation_and_other_window_commands_are_left_pending() {
    for command in [0x2140u16, 0x2170] {
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

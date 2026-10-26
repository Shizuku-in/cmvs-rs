use cmvs_rs::{
    machine::{Machine, Step},
    message::{
        LineBreakRules, MESSAGE_WINDOWS, MessageHost, MessageWindow, TEXT_OBJECTS, TextObject,
        WINDOW_BUTTONS, WindowButton,
    },
    ps3::Script,
    scene::{DrawState, SceneObject},
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
        logical_size: [1280, 720],
        position,
        area: [0, 0, size[0] as i32, size[1] as i32],
        cursor: [0, 0],
        glyph_size: 27,
        spacing: [0, 0],
        color: 0xff_ffff,
        decoration_color: 0xff_ffff,
        decoration: 0,
        reveal: [0, 0],
        char_delay: 100,
        layer: 1,
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
            position: [3, -1],
            size: [1280, 54],
            switch_positions: [[3, -1]; 2],
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

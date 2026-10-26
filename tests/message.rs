use cmvs_rs::{
    machine::{Machine, Step},
    message::{MESSAGE_WINDOWS, MessageHost, MessageWindow, TEXT_OBJECTS, TextObject},
    ps3::Script,
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
        })
    );
    assert_eq!(
        messages.text(10),
        Some(&TextObject {
            logical_size: [1280, 720],
            position: [3, -1],
            area: [0, 0, 1280, 54],
        })
    );
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
    for command in [0x2140u16, 0x2142] {
        let script = script(&command.to_le_bytes());
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(!MessageHost::default().try_handle(&mut vm).unwrap());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
    }
}

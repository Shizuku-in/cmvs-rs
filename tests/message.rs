use cmvs_rs::{
    machine::{Machine, Step},
    message::{MESSAGE_WINDOWS, MessageHost, TEXT_OBJECTS},
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
fn destroy(slots: &[u32]) -> Vec<u8> {
    let mut code = vec![];
    for &slot in slots {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(slot.to_le_bytes());
        code.extend(0x2141u16.to_le_bytes());
    }
    code
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn destroying_constructor_empty_windows_pops_the_slot_only() {
    let script = script(&destroy(&[0, 3, 7]));
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut messages = MessageHost::default();
    for _ in 0..3 {
        pending(&mut vm);
        assert!(messages.try_handle(&mut vm).unwrap());
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert!((0..MESSAGE_WINDOWS).all(|slot| messages.window_text(slot).is_none()));
    assert!((0..TEXT_OBJECTS).all(|index| !messages.text_present(index)));
}

#[test]
fn out_of_range_negative_slots_and_short_stacks_keep_the_command_pending() {
    for code in [
        destroy(&[8]),
        destroy(&[u32::MAX]),
        destroy(&[0x8000_0000]),
        0x2141u16.to_le_bytes().to_vec(),
    ] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        assert!(MessageHost::default().try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x141)));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn creation_and_other_window_commands_are_left_pending() {
    for command in [0x2140u16, 0x2142] {
        let script = script(&command.to_le_bytes());
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(!MessageHost::default().try_handle(&mut vm).unwrap());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(command - 0x2000));
    }
}

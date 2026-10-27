use cmvs_rs::{
    machine::{Machine, Step},
    message_flags::MessageFlags,
    ps3::Script,
};

/// 0x22eb reads/writes the skip flag (+1520), 0x22e5 the auto-advance flag (+1552).
const COMMANDS: [u16; 2] = [0x22eb, 0x22e5];

fn reserve(code: &mut Vec<u8>, bytes: u16, raw: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(bytes.to_le_bytes());
    code.extend(raw.to_le_bytes());
}
/// Arguments are listed from SP-4 outward.
fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        reserve(code, 4, arg);
    }
    code.extend(op.to_le_bytes());
}
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
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn mode_1548_preserves_signed_abs_remainder_including_minimum_integer() {
    let cases = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 0),
        (7, 1),
        (u32::MAX, 1),
        ((-2i32) as u32, 2),
        ((-3i32) as u32, 0),
        (i32::MAX as u32, 1),
        (i32::MIN as u32, (-2i32) as u32),
    ];
    let mut code = vec![];
    call(&mut code, 0x22eb, &[1, 1]);
    call(&mut code, 0x22e5, &[1, 1]);
    for (raw, _) in cases {
        call(&mut code, 0x22d0, &[u32::MAX, raw]);
        call(&mut code, 0x22d0, &[0, 999]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut flags = MessageFlags::default();
    for _ in 0..2 {
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
    }
    for (_, expected) in cases {
        vm.state_mut().set_system_integer(0, 99).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(flags.try_handle(&mut vm).unwrap());
        assert_eq!(flags.mode_1548(), expected);
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert!(flags.skip() && flags.auto_advance());
        pending(&mut vm);
        assert!(flags.try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert!(flags.skip() && flags.auto_advance());
    }
}

#[test]
fn mode_query_defaults_to_zero_and_unused_value_may_remain_unknown() {
    let mut code = vec![];
    reserve(&mut code, 8, 77);
    reserve(&mut code, 4, 0);
    code.extend(0x22d0u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut flags = MessageFlags::default();
    assert_eq!(flags.mode_1548(), 0);
    assert!(!flags.try_handle(&mut vm).unwrap());
    for slot in 0..7 {
        vm.state_mut().set_system_integer(slot, 42).unwrap();
    }
    pending(&mut vm);
    assert!(flags.try_handle(&mut vm).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(0));
    for slot in 1..7 {
        assert_eq!(vm.state().system_integer(slot), Some(42));
    }
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 77);
    assert_eq!(flags, MessageFlags::default());
}

#[test]
fn mode_errors_preserve_existing_setting_results_and_pending_arguments() {
    let mut bad = vec![];
    for args in [vec![1], vec![0], vec![]] {
        let mut code = vec![];
        call(&mut code, 0x22d0, &args);
        bad.push(code);
    }
    let mut unknown = vec![];
    reserve(&mut unknown, 8, 0);
    reserve(&mut unknown, 4, 1);
    unknown.extend(0x22d0u16.to_le_bytes());
    bad.push(unknown);
    for tail in bad {
        let mut code = vec![];
        call(&mut code, 0x22d0, &[1, 2]);
        code.extend(tail);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut flags = MessageFlags::default();
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
        pending(&mut vm);
        let before = flags;
        let command = vm.pending_system();
        let len = vm.stack().len_bytes();
        vm.state_mut().set_system_integer(0, 99).unwrap();
        for _ in 0..2 {
            assert!(flags.try_handle(&mut vm).is_err());
            assert_eq!(flags, before);
            assert_eq!(vm.pending_system(), command);
            assert_eq!(vm.stack().len_bytes(), len);
            assert_eq!(vm.state().system_integer(0), Some(99));
        }
    }
}

#[test]
fn constructor_values_are_inactive_and_query_ignores_the_second_argument() {
    for op in COMMANDS {
        let mut code = vec![];
        // SP-8 is a reserved but never-written word; the query branch does not read it.
        reserve(&mut code, 8, 5);
        reserve(&mut code, 4, 0);
        code.extend(op.to_le_bytes());
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut flags = MessageFlags::default();
        assert!(!flags.skip() && !flags.auto_advance());
        pending(&mut vm);
        let pc = vm.pc();
        assert!(flags.try_handle(&mut vm).unwrap());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 4));
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.state().result, None);
    }
}

#[test]
fn set_normalizes_the_value_and_keeps_the_system_result() {
    for op in COMMANDS {
        let mut code = vec![];
        call(&mut code, op, &[u32::MAX, 7]);
        call(&mut code, op, &[0, 0]);
        call(&mut code, op, &[1, 0]);
        call(&mut code, op, &[0, u32::MAX]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        let mut flags = MessageFlags::default();
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(9));
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1));
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
        pending(&mut vm);
        flags.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn the_two_flags_are_independent() {
    let mut code = vec![];
    call(&mut code, 0x22e5, &[1, 1]);
    call(&mut code, 0x22eb, &[0, 0]);
    call(&mut code, 0x22e5, &[0, 0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut flags = MessageFlags::default();
    pending(&mut vm);
    flags.try_handle(&mut vm).unwrap();
    assert!(flags.auto_advance() && !flags.skip());
    pending(&mut vm);
    flags.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    pending(&mut vm);
    flags.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
}

#[test]
fn unreadable_or_missing_arguments_keep_the_command_pending() {
    for op in COMMANDS {
        let mut unknown_value = vec![];
        reserve(&mut unknown_value, 8, 0);
        reserve(&mut unknown_value, 4, 1);
        unknown_value.extend(op.to_le_bytes());
        let mut short_set = vec![];
        call(&mut short_set, op, &[1]);
        let mut short_query = vec![];
        call(&mut short_query, op, &[0]);
        for code in [unknown_value, short_set, short_query] {
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            let mut flags = MessageFlags::default();
            pending(&mut vm);
            let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
            assert!(flags.try_handle(&mut vm).is_err());
            assert_eq!(vm.pending_system(), Some((pc, op - 0x2000)));
            assert_eq!(vm.stack().len_bytes(), bytes);
            assert_eq!(vm.state().system_integer(0), None);
            assert_eq!(flags, MessageFlags::default());
        }
    }
}

#[test]
fn bypass_is_set_from_sp4_without_a_query_or_system_result() {
    let mut code = vec![];
    for value in [1, 0, 0x8000_0000, 0, u32::MAX] {
        call(&mut code, 0x2340, &[value]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 7).unwrap();
    let mut flags = MessageFlags::default();
    assert!(!flags.bypass());
    for expected in [true, false, true, false, true] {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(flags.try_handle(&mut vm).unwrap());
        assert_eq!(flags.bypass(), expected);
        assert_eq!((vm.pending_system(), vm.pc()), (None, pc + 2));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(vm.state().system_integer(0), Some(7));
    assert!(!flags.skip() && !flags.auto_advance());
    assert_eq!(flags.mode_1548(), 0);
}

#[test]
fn bypass_needs_a_known_argument() {
    let mut unknown = vec![];
    reserve(&mut unknown, 8, 1);
    unknown.extend(0x2340u16.to_le_bytes());
    for code in [unknown, 0x2340u16.to_le_bytes().to_vec()] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut flags = MessageFlags::default();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        assert!(flags.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x340)));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(flags, MessageFlags::default());
    }
}

#[test]
fn other_commands_are_left_pending() {
    let mut code = vec![];
    reserve(&mut code, 4, 0);
    code.extend(0x22efu16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    assert!(!MessageFlags::default().try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x2ef));
}

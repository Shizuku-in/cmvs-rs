use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    skip::SkipFlag,
};

fn reserve(code: &mut Vec<u8>, bytes: u16, raw: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(bytes.to_le_bytes());
    code.extend(raw.to_le_bytes());
}
/// Arguments are listed from SP-4 outward, then 0x22eb.
fn call(code: &mut Vec<u8>, args: &[u32]) {
    for &arg in args.iter().rev() {
        reserve(code, 4, arg);
    }
    code.extend(0x22ebu16.to_le_bytes());
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
fn constructor_value_is_inactive_and_query_ignores_the_second_argument() {
    let mut code = vec![];
    // SP-8 is a reserved but never-written word; the query branch does not read it.
    reserve(&mut code, 8, 5);
    reserve(&mut code, 4, 0);
    code.extend(0x22ebu16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut skip = SkipFlag::default();
    assert!(!skip.active());
    pending(&mut vm);
    let pc = vm.pc();
    assert!(skip.try_handle(&mut vm).unwrap());
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 4));
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(vm.state().result, None);
}

#[test]
fn set_normalizes_the_value_and_keeps_the_system_result() {
    let mut code = vec![];
    call(&mut code, &[u32::MAX, 7]);
    call(&mut code, &[0, 0]);
    call(&mut code, &[1, 0]);
    call(&mut code, &[0, u32::MAX]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut skip = SkipFlag::default();
    pending(&mut vm);
    skip.try_handle(&mut vm).unwrap();
    assert!(skip.active());
    assert_eq!(vm.state().system_integer(0), Some(9));
    pending(&mut vm);
    skip.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
    pending(&mut vm);
    skip.try_handle(&mut vm).unwrap();
    assert!(!skip.active());
    pending(&mut vm);
    skip.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn unreadable_or_missing_arguments_keep_the_command_pending() {
    let mut unknown_value = vec![];
    reserve(&mut unknown_value, 8, 0);
    reserve(&mut unknown_value, 4, 1);
    unknown_value.extend(0x22ebu16.to_le_bytes());
    let mut short_set = vec![];
    call(&mut short_set, &[1]);
    let mut short_query = vec![];
    call(&mut short_query, &[0]);
    for code in [unknown_value, short_set, short_query] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut skip = SkipFlag::default();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        assert!(skip.try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x2eb)));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), None);
        assert!(!skip.active());
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
    assert!(!SkipFlag::default().try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x2ef));
}

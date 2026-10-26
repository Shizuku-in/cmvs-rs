use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    sequence::{SequenceHeader, SequenceHost},
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
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn reset_zeroes_the_header_and_pops_nothing() {
    // A word left by earlier code stays on the stack: 0x2280 returns 0x4000.
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(5u32.to_le_bytes());
    code.extend(0x2280u16.to_le_bytes());
    code.extend(0x2281u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 3).unwrap();
    let mut sequence = SequenceHost::default();
    pending(&mut vm);
    assert!(sequence.try_handle(&mut vm).unwrap());
    assert_eq!(sequence.header(), SequenceHeader::default());
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.state().system_integer(0), Some(3));
    // Other commands of the family stay pending.
    pending(&mut vm);
    assert!(!sequence.try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x281));
}

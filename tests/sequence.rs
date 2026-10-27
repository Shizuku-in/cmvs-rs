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
    code.extend(0x2282u16.to_le_bytes());
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
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x282));
}

#[test]
fn count_query_preserves_stack_header_and_other_results() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0xdeadbeefu32.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x2282u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 99).unwrap();
    vm.state_mut().set_system_integer(4, 7).unwrap();
    let mut sequence = SequenceHost::default();
    assert!(!sequence.try_handle(&mut vm).unwrap());
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(sequence.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.state().system_integer(4), Some(7));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdeadbeef);
        assert_eq!(sequence.header(), SequenceHeader::default());
    }
    pending(&mut vm);
    assert!(!sequence.try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x282));
}

#[test]
fn current_index_uses_activity_not_count_and_preserves_raw_index_bits() {
    for (active, count, index, expected) in [
        (0, 0, 0, u32::MAX),
        (0, 7, 3, u32::MAX),
        (1, 7, 3, 3),
        (2, 0, 0x80000000, 0x80000000),
        (u32::MAX, 1, u32::MAX, u32::MAX),
    ] {
        let header = SequenceHeader {
            active,
            count,
            index,
            start: 77,
            waited: 88,
        };
        assert_eq!(header.current_index(), expected);
        assert_eq!(header.start, 77);
        assert_eq!(header.waited, 88);
    }
}

#[test]
fn current_index_query_returns_minus_one_when_inactive_and_does_not_consume_parameters() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0xdeadbeefu32.to_le_bytes());
    code.extend(0x2281u16.to_le_bytes());
    code.extend(0x2281u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut sequence = SequenceHost::default();
    for slot in 0..7 {
        vm.state_mut().set_system_integer(slot, slot + 10).unwrap();
    }
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        let header = sequence.header();
        assert!(sequence.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
        for slot in 1..7 {
            assert_eq!(vm.state().system_integer(slot), Some(slot + 10));
        }
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdeadbeef);
        assert_eq!(sequence.header(), header);
    }
}

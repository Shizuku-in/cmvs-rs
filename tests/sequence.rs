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
    code.extend(0x228bu16.to_le_bytes());
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
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x28b));
}

#[test]
fn count_query_preserves_stack_header_and_other_results() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0xdeadbeefu32.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x228bu16.to_le_bytes());
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
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x28b));
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

fn call(code: &mut Vec<u8>, opcode: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(opcode.to_le_bytes());
}
fn run(vm: &mut Machine<'_>, sequence: &mut SequenceHost) {
    pending(vm);
    assert!(sequence.try_handle(vm).unwrap());
}

#[test]
fn wait_append_caps_at_sixteen_preserves_results_and_reset_keeps_inactive_entries() {
    use cmvs_rs::sequence::SequenceEntry;
    let mut code = vec![];
    for duration in 0..17 {
        call(&mut code, 0x2286, &[duration]);
    }
    call(&mut code, 0x2285, &[]);
    call(&mut code, 0x2280, &[]);
    call(&mut code, 0x2286, &[999]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    for count in 1..=17 {
        run(&mut vm, &mut host);
        assert_eq!(host.header().count, count.min(16));
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    for i in 0..16 {
        assert_eq!(
            host.entries()[i],
            Some(SequenceEntry::Wait { duration: i as u32 })
        );
    }
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(16));
    let entries = *host.entries();
    host.advance_frame(42);
    run(&mut vm, &mut host);
    assert_eq!(host.header(), SequenceHeader::default());
    assert_eq!(host.entries(), &entries);
    assert_eq!(host.clock(), 42);
    run(&mut vm, &mut host);
    assert_eq!(
        host.entries()[0],
        Some(SequenceEntry::Wait { duration: 999 })
    );
    assert_eq!(&host.entries()[1..], &entries[1..]);
    assert_eq!(host.header().count, 1);
}

#[test]
fn wait_sequence_start_stop_restart_and_queries_follow_header_lifecycle() {
    let mut code = vec![];
    for (op, args) in [
        (0x2282, vec![]),
        (0x2286, vec![10]),
        (0x2286, vec![20]),
        (0x2282, vec![]),
        (0x2281, vec![]),
        (0x2282, vec![]),
        (0x2283, vec![]),
        (0x2282, vec![]),
        (0x2281, vec![]),
    ] {
        call(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::with_tick(100);
    run(&mut vm, &mut host);
    assert_eq!(host.header(), SequenceHeader::default());
    for _ in 0..3 {
        run(&mut vm, &mut host);
    }
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 1,
            count: 2,
            index: 0,
            start: 100,
            waited: 0
        }
    );
    host.advance_frame(15);
    assert_eq!(host.header().index, 1);
    assert_eq!(host.header().waited, 10);
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(1));
    let active = host.header();
    run(&mut vm, &mut host);
    assert_eq!(host.header(), active, "start while active is a no-op");
    run(&mut vm, &mut host);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            ..active
        }
    );
    host.advance_frame(100);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            ..active
        }
    );
    run(&mut vm, &mut host);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 1,
            count: 2,
            index: 0,
            start: 215,
            waited: 0
        }
    );
    host.advance_frame(30);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            count: 2,
            index: 2,
            start: 215,
            waited: 30
        }
    );
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
}

#[test]
fn waits_use_wrapping_absolute_time_and_carry_leftover_time_without_frame_clamp() {
    for start in [0, u32::MAX - 15] {
        let mut code = vec![];
        for duration in [0, 20, 40, 0] {
            call(&mut code, 0x2286, &[duration]);
        }
        call(&mut code, 0x2282, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::with_tick(start);
        for _ in 0..5 {
            run(&mut vm, &mut host);
        }
        host.advance_frame(0);
        assert_eq!(host.header().index, 1);
        host.advance_frame(25);
        assert_eq!(
            (
                host.header().index,
                host.header().waited,
                host.header().active
            ),
            (2, 20, 1)
        );
        assert_eq!(host.clock(), start.wrapping_add(25));
        host.advance_frame(34);
        assert_eq!(host.header().active, 1);
        host.advance_frame(1);
        assert_eq!(
            (
                host.header().index,
                host.header().waited,
                host.header().active
            ),
            (4, 60, 0)
        );
        let finished = host.header();
        host.advance_frame(1000);
        assert_eq!(host.header(), finished);
    }
    let mut code = vec![];
    call(&mut code, 0x2286, &[300]);
    call(&mut code, 0x2282, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    run(&mut vm, &mut host);
    run(&mut vm, &mut host);
    host.advance_frame(300);
    assert_eq!(
        host.header().active,
        0,
        "sequence time is not clamped to 200ms"
    );
}

#[test]
fn finishing_waits_clears_header_keeps_entries_clock_stack_and_results() {
    for active in [false, true] {
        let mut code = vec![];
        call(&mut code, 0x2286, &[u32::MAX]);
        if active {
            call(&mut code, 0x2282, &[]);
        }
        call(&mut code, 0x2284, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        run(&mut vm, &mut host);
        if active {
            run(&mut vm, &mut host);
        }
        host.advance_frame(10);
        let entries = *host.entries();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        run(&mut vm, &mut host);
        assert_eq!(host.header(), SequenceHeader::default());
        assert_eq!(host.entries(), &entries);
        assert_eq!(host.clock(), 10);
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
    }
}

#[test]
fn failed_wait_append_preserves_active_sequence_and_unknown_actions_remain_pending() {
    let mut code = vec![];
    call(&mut code, 0x2286, &[100]);
    call(&mut code, 0x2282, &[]);
    call(&mut code, 0x2286, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    run(&mut vm, &mut host);
    run(&mut vm, &mut host);
    host.advance_frame(25);
    let header = host.header();
    let entries = *host.entries();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    pending(&mut vm);
    for _ in 0..2 {
        assert!(host.try_handle(&mut vm).is_err());
        assert_eq!(host.header(), header);
        assert_eq!(host.entries(), &entries);
        assert_eq!(host.clock(), 25);
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x286)));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    for opcode in [0x2287u16, 0x2288, 0x2289, 0x228a] {
        let program = script(&opcode.to_le_bytes());
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        assert!(!host.try_handle(&mut vm).unwrap());
        assert_eq!(vm.pending_system(), Some((0, opcode - 0x2000)));
        assert_eq!(host.header(), header);
        assert_eq!(host.entries(), &entries);
    }
}

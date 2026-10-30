use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    timers::VmTimers,
};

fn command(code: &mut Vec<u8>, op: u16, slot: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(slot.to_le_bytes());
    code.extend(op.to_le_bytes());
}
fn program(code: &[u8]) -> Script {
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        0,
        0,
        0,
        code.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn constructor_and_run_updates_match_all_ten_slots_with_wrapping() {
    let mut timers = VmTimers::default();
    assert_eq!(timers.values(), &[0; 10]);
    timers.begin_run(u32::MAX - 2);
    assert_eq!(timers.values(), &[u32::MAX - 2; 10]);
    timers.begin_run(10);
    assert_eq!(timers.values(), &[7; 10]);
    timers.begin_run(0);
    assert_eq!(timers.values(), &[7; 10]);
}

#[test]
fn reset_is_local_query_returns_raw_word_and_commands_do_not_advance_time() {
    let mut code = vec![];
    command(&mut code, 0x200b, 9);
    command(&mut code, 0x200c, 9);
    command(&mut code, 0x200c, 0);
    let program = program(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut timers = VmTimers::default();
    timers.begin_run(0x80000001);
    vm.state_mut().set_system_integer(0, 123).unwrap();
    pending(&mut vm);
    timers.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(123));
    assert_eq!(timers.values()[9], 0);
    for expected in [0, 0x80000001] {
        pending(&mut vm);
        let pc = vm.pc();
        timers.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(&timers.values()[..9], &[0x80000001; 9]);
}

#[test]
fn invalid_slots_and_missing_arguments_keep_vm_and_timers_unchanged() {
    for op in [0x200b, 0x200c] {
        for slot in [Some(10), Some(u32::MAX), None] {
            let mut code = vec![];
            if let Some(slot) = slot {
                command(&mut code, op, slot)
            } else {
                code.extend(op.to_le_bytes())
            }
            let program = program(&code);
            let mut vm = Machine::new(&program).unwrap();
            pending(&mut vm);
            let mut timers = VmTimers::default();
            timers.begin_run(42);
            vm.state_mut().set_system_integer(0, 88).unwrap();
            let pending = vm.pending_system();
            let sp = vm.stack().len_bytes();
            assert!(timers.try_handle(&mut vm).is_err());
            assert_eq!(timers.values(), &[42; 10]);
            assert_eq!(vm.pending_system(), pending);
            assert_eq!(vm.stack().len_bytes(), sp);
            assert_eq!(vm.state().system_integer(0), Some(88));
        }
    }
}

#[test]
fn unrelated_commands_remain_pending() {
    let program = program(&0x200du16.to_le_bytes());
    let mut vm = Machine::new(&program).unwrap();
    let mut timers = VmTimers::default();
    assert!(!timers.try_handle(&mut vm).unwrap());
    pending(&mut vm);
    assert!(!timers.try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system(), Some((0, 13)));
}

#[test]
fn stopwatch_preserves_stack_and_other_results_and_wraps_without_resetting_slots() {
    let mut code = vec![];
    // Keep a sentinel on the stack: neither stopwatch command consumes it.
    command(&mut code, 0x200f, 0x12345678);
    for op in [0x200eu16, 0x200f, 0x200f, 0x200e, 0x200f] {
        code.extend(op.to_le_bytes());
    }
    let program = program(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut timers = VmTimers::default();
    timers.begin_run(u32::MAX - 2);
    vm.state_mut().set_system_integer(4, 99).unwrap();
    // Before the first mark, the constructor's zero mark is used.
    pending(&mut vm);
    timers.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 2));
    pending(&mut vm);
    timers.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 2));
    timers.begin_run(10);
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        timers.try_handle(&mut vm).unwrap();
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.state().system_integer(0), Some(10));
        assert_eq!(vm.state().system_integer(4), Some(99));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
    }
    pending(&mut vm);
    timers.try_handle(&mut vm).unwrap();
    pending(&mut vm);
    timers.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(timers.clock(), 7);
    assert_eq!(timers.values(), &[7; 10]);
}

#[test]
fn frame_clock_and_delta_queries_follow_the_last_run_entry() {
    let mut code = vec![];
    for op in [0x2008u16, 0x2009, 0x2008, 0x2009] {
        code.extend(op.to_le_bytes());
    }
    let script = program(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut timers = VmTimers::default();
    let query = |vm: &mut Machine<'_>, timers: &mut VmTimers| {
        pending(vm);
        assert!(timers.try_handle(vm).unwrap());
        vm.state().system_integer(0)
    };
    timers.begin_run(16);
    assert_eq!(query(&mut vm, &mut timers), Some(16));
    assert_eq!(query(&mut vm, &mut timers), Some(16));
    timers.begin_run(u32::MAX);
    assert_eq!(query(&mut vm, &mut timers), Some(15));
    assert_eq!(query(&mut vm, &mut timers), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}

use cmvs_rs::{
    history::{HISTORY_CAPACITY, HistoryHost, MAX_HISTORY_TEXT},
    machine::{Machine, Step},
    ps3::Script,
};

fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for value in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
fn script(code: &[u8], text: &[u8]) -> Script {
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(text)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn append_preserves_raw_strings_numeric_bits_results_and_lower_stack() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(77u32.to_le_bytes());
    call(
        &mut code,
        0x2136,
        &[0, 2, 4, 7, 1, u32::MAX, 0x80000000, 4, 5, 6],
    );
    let script = script(&code, b"a\0b\0\x82\xa0\0c\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut history = HistoryHost::default();
    for slot in 0..7 {
        vm.state_mut().set_system_integer(slot, 99).unwrap();
    }
    pending(&mut vm);
    let pc = vm.pc();
    assert!(history.try_handle(&mut vm).unwrap());
    let entry = history.entries().front().unwrap();
    assert_eq!(
        entry.strings,
        [
            b"a".to_vec(),
            b"b".to_vec(),
            vec![0x82, 0xa0],
            b"c".to_vec()
        ]
    );
    assert_eq!(entry.words, [1, u32::MAX, 0x80000000, 4, 5, 6]);
    assert_eq!(vm.pc(), pc + 2);
    assert_eq!(vm.pending_system(), None);
    assert_eq!(vm.repeat_count(), 0);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 77);
    for slot in 0..7 {
        assert_eq!(vm.state().system_integer(slot), Some(99));
    }
}

#[test]
fn history_keeps_newest_first_evicts_oldest_and_owns_string_copies() {
    let mut code = vec![];
    for i in 0..130 {
        call(&mut code, 0x2136, &[0, 0, 0x80000003, 0, i, 0, 0, 0, 0, 0]);
    }
    let script = script(&code, b"\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut history = HistoryHost::default();
    for i in 0..130u32 {
        vm.state_mut()
            .set_string(
                3,
                &i.to_le_bytes().iter().map(|b| b + 1).collect::<Vec<_>>(),
            )
            .unwrap();
        pending(&mut vm);
        assert!(history.try_handle(&mut vm).unwrap());
        assert_eq!(
            history.entries().len(),
            (i as usize + 1).min(HISTORY_CAPACITY)
        );
    }
    assert_eq!(history.entries().front().unwrap().words[0], 129);
    assert_eq!(history.entries().back().unwrap().words[0], 2);
    assert_eq!(
        history.entries().get(1).unwrap().strings[2],
        vec![129, 1, 1, 1]
    );
    vm.state_mut().set_string(3, b"changed").unwrap();
    assert_eq!(
        history.entries().front().unwrap().strings[2],
        vec![130, 1, 1, 1]
    );
}

#[test]
fn clear_releases_all_entries_preserves_stack_results_and_allows_refilling() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(77u32.to_le_bytes());
    call(&mut code, 0x212f, &[]); // Empty-list branch.
    for i in 0..HISTORY_CAPACITY {
        call(&mut code, 0x2136, &[0, 0, 2, 0, i as u32, 0, 0, 0, 0, 0]);
    }
    call(&mut code, 0x212f, &[]);
    call(&mut code, 0x212f, &[]); // Repeated clear.
    call(&mut code, 0x2136, &[0, 0, 2, 0, 999, 0, 0, 0, 0, 0]);
    let script = script(&code, b"a\0body\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut history = HistoryHost::default();
    for slot in 0..7 {
        vm.state_mut().set_system_integer(slot, 99).unwrap();
    }
    for expected in std::iter::once(0)
        .chain(1..=HISTORY_CAPACITY)
        .chain([0, 0, 1])
    {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(history.try_handle(&mut vm).unwrap());
        assert_eq!(history.entries().len(), expected);
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 77);
        for slot in 0..7 {
            assert_eq!(vm.state().system_integer(slot), Some(99));
        }
    }
    let entry = history.entries().front().unwrap();
    assert_eq!(entry.words[0], 999);
    assert_eq!(entry.strings[2], b"body");
}

#[test]
fn fixed_strings_accept_31_bytes_but_only_dynamic_text_accepts_larger_data() {
    let mut code = vec![];
    call(
        &mut code,
        0x2136,
        &[0x80000000, 0x80000000, 1, 0x80000000, 0, 0, 0, 0, 0, 0],
    );
    let mut text = vec![0];
    text.extend(vec![b'y'; MAX_HISTORY_TEXT]);
    text.push(0);
    let script = script(&code, &text);
    let mut vm = Machine::new(&script).unwrap();
    let mut history = HistoryHost::default();
    vm.state_mut().set_string(0, &[b'x'; 31]).unwrap();
    pending(&mut vm);
    assert!(history.try_handle(&mut vm).unwrap());
    let entry = history.entries().front().unwrap();
    assert_eq!(entry.strings[0].len(), 31);
    assert_eq!(entry.strings[2].len(), MAX_HISTORY_TEXT);
}

#[test]
fn invalid_references_long_fixed_strings_and_short_stacks_are_transactional() {
    let mut bad = vec![
        vec![],
        vec![0; 9],
        vec![0x3fffffff, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![0xc0000000, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    for index in [0, 1, 3] {
        let mut args = vec![0; 10];
        args[index] = 0x80000000;
        bad.push(args);
    }
    let mut args = vec![0; 10];
    args[2] = 1;
    bad.push(args);
    for args in bad {
        let mut code = vec![];
        call(&mut code, 0x2136, &[0; 10]);
        call(&mut code, 0x2136, &args);
        let mut text = vec![0];
        text.extend(vec![b'y'; MAX_HISTORY_TEXT + 1]);
        text.push(0);
        let script = script(&code, &text);
        let mut vm = Machine::new(&script).unwrap();
        let mut history = HistoryHost::default();
        vm.state_mut().set_string(0, &[b'x'; 32]).unwrap();
        pending(&mut vm);
        history.try_handle(&mut vm).unwrap();
        pending(&mut vm);
        let command = vm.pending_system();
        let pc = vm.pc();
        let entries = history.entries().clone();
        vm.state_mut().set_system_integer(0, 42).unwrap();
        for _ in 0..2 {
            assert!(history.try_handle(&mut vm).is_err());
            assert_eq!(history.entries(), &entries);
            assert_eq!(vm.pending_system(), command);
            assert_eq!(vm.pc(), pc);
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            for (i, value) in args.iter().enumerate() {
                assert_eq!(vm.stack().peek_u32((i + 1) * 4).unwrap(), *value);
            }
            assert_eq!(vm.state().system_integer(0), Some(42));
        }
    }
}

#[test]
fn unsupported_commands_and_no_pending_command_are_not_consumed() {
    let program = script(&0x2137u16.to_le_bytes(), b"");
    let mut vm = Machine::new(&program).unwrap();
    let mut history = HistoryHost::default();
    assert!(!history.try_handle(&mut vm).unwrap());
    pending(&mut vm);
    let command = vm.pending_system();
    assert!(!history.try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system(), command);
    assert!(history.entries().is_empty());
}

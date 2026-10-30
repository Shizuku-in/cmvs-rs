use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    vm::SystemResult,
};

fn word(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}
fn scalar(tag: u16, raw: u32) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend(raw.to_le_bytes());
    bytes
}
fn expr(kind: u16, terms: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = kind.to_le_bytes().to_vec();
    for term in terms {
        bytes.extend(term);
    }
    word(&mut bytes, 0x20f);
    bytes
}
fn slot(index: i32) -> Vec<u8> {
    let mut bytes = 0x121u16.to_le_bytes().to_vec();
    bytes.extend(expr(0x200, vec![scalar(0x100, index as u32)]));
    bytes
}
fn op(opcode: u16) -> Vec<u8> {
    opcode.to_le_bytes().to_vec()
}
fn script(code: &[u8], labels: &[u32], text: &[u8]) -> Script {
    let mut payload = vec![];
    for label in labels {
        payload.extend(label.to_le_bytes());
    }
    payload.extend(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48,
        0,
        0,
        labels.len() as u32,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(value.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn run_to_system(vm: &mut Machine<'_>) -> Step {
    loop {
        let step = vm.step().unwrap();
        if matches!(step, Step::System { .. }) {
            return step;
        }
    }
}

#[test]
fn frame_argument_is_copied_into_string_slot_like_main_label_186() {
    let mut code = expr(0x201, vec![scalar(0x120, 0)]);
    word(&mut code, 0x430);
    word(&mut code, 0x410);
    word(&mut code, 1);
    word(&mut code, 0x2161);
    let function = code.len() as u32;
    code.extend(expr(0x201, vec![slot(4), scalar(0x122, 8), op(0x170)]));
    code.extend(expr(0x201, vec![scalar(0x122, 8)]));
    word(&mut code, 0x430);
    word(&mut code, 0x2162);
    word(&mut code, 0x411);
    word(&mut code, 4);
    let script = script(&code, &[0, function], b"- main -\0");
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..4 {
        vm.step().unwrap();
    }
    assert_eq!(vm.state().string(4), Some(&b"- main -"[..]));
    assert_eq!(vm.state().string_buffer(), b"- main -");
    // The bottom entry became a placeholder, so the nested index result survives.
    assert_eq!(vm.state().result, Some(4));
    assert_eq!(vm.state().integer(4096), Some(4));
    assert_eq!(vm.state().condition, Some(true));
    assert_eq!(
        run_to_system(&mut vm),
        Step::System {
            pc: function as usize + 36,
            command: 0x162
        }
    );
    assert_eq!(vm.state().result, Some(0));
    assert!(vm.state().string_buffer().is_empty());
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 0);
    vm.complete_system(SystemResult(0x4004)).unwrap();
    assert_eq!(
        run_to_system(&mut vm),
        Step::System {
            pc: 16,
            command: 0x161
        }
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn binary_operators_concatenate_and_slot_reads_append() {
    let mut code = expr(
        0x201,
        vec![
            slot(1),
            scalar(0x120, 0),
            scalar(0x120, 3),
            op(0x163),
            op(0x170),
        ],
    );
    code.extend(expr(0x201, vec![slot(2), slot(1), op(0x170)]));
    code.extend(expr(0x201, vec![slot(200), scalar(0x120, 0), op(0x170)]));
    code.extend(expr(0x201, vec![slot(1)]));
    let script = script(&code, &[0], b"ab\0cd\0");
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    assert_eq!(vm.state().string(1), Some(&b"abcd"[..]));
    vm.step().unwrap();
    assert_eq!(vm.state().string(2), Some(&b"abcd"[..]));
    vm.step().unwrap();
    assert_eq!(vm.state().string(200), None);
    assert_eq!(vm.state().result, Some(200));
    assert_eq!(vm.state().string_buffer(), b"ab");
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(0x8000_0001));
    assert!(vm.state().string_buffer().is_empty());
}

#[test]
fn failures_commit_nothing_and_leave_pc_unchanged() {
    let long = vec![b'x'; 600];
    let mut text = long.clone();
    text.push(0);
    let cases = [
        expr(0x201, vec![slot(1), slot(9), op(0x170)]),
        expr(0x201, vec![slot(1), scalar(0x122, 8), op(0x170)]),
        expr(0x201, vec![scalar(0x127, 0), scalar(0x120, 0), op(0x170)]),
        expr(
            0x201,
            vec![
                slot(1),
                scalar(0x120, 0),
                scalar(0x120, 0),
                op(0x163),
                op(0x170),
            ],
        ),
        expr(0x201, vec![slot(1), scalar(0x125, 0), op(0x170)]),
        expr(0x201, vec![slot(1), op(0x170)]),
    ];
    for code in cases {
        let script = script(&code, &[0], &text);
        let mut vm = Machine::new(&script).unwrap();
        assert!(vm.step().is_err());
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.state().string(1), None);
        assert_eq!(vm.state().integer(4096), None);
        assert_eq!(vm.state().result, None);
        assert!(vm.state().string_buffer().is_empty());
    }
}

#[test]
fn call_frame_table_allows_63_nested_calls() {
    let mut code = vec![];
    word(&mut code, 0x410);
    word(&mut code, 0);
    let script = script(&code, &[0], b"");
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..63 {
        vm.step().unwrap();
    }
    assert!(vm.step().is_err());
    assert_eq!(vm.stack().len_bytes(), 63 * 4);
}

#[test]
fn strlen_0x20f8_reports_resolved_byte_length() {
    let mut code = vec![];
    for offset in [0u32, 9] {
        code.extend(expr(0x201, vec![scalar(0x120, offset)]));
        word(&mut code, 0x430);
        word(&mut code, 0x20f8);
    }
    // An unassigned string-variable reference cannot be resolved.
    word(&mut code, 0x440);
    word(&mut code, 4);
    code.extend(0x8000_0005u32.to_le_bytes());
    word(&mut code, 0x20f8);
    let script = script(&code, &[0], b"logo.pb3\0\0");
    let mut vm = Machine::new(&script).unwrap();
    for expected in [8, 0] {
        run_to_system(&mut vm);
        assert!(cmvs_rs::text::try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    run_to_system(&mut vm);
    assert!(cmvs_rs::text::try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0xf8));
    assert_eq!(vm.state().system_integer(0), Some(0));
}

#[test]
fn compare_0x20f1_folds_ascii_letters_and_matches_shift_jis_pairs_exactly() {
    // Offsets: abc 0, ABC 4, abd 8, ab 12, あ 15, lead+a 18, lead+A 21, empty 24, lead 25.
    let text = b"abc\0ABC\0abd\0ab\0\x82\xa0\0\x82a\0\x82A\0\0\x82\0";
    let cases = [
        (0, 4, 1),
        (4, 0, 1),
        (0, 8, 0),
        (12, 0, 0),
        (0, 12, 0),
        (15, 15, 1),
        (18, 21, 0), // the byte after a lead byte is not folded
        (24, 24, 1),
    ];
    let mut code = vec![];
    for (first, second, _) in cases.iter().copied().chain([(25, 25, 0)]) {
        push(&mut code, second);
        push(&mut code, first);
        word(&mut code, 0x20f1);
    }
    let script = script(&code, &[0], text);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(1, 9).unwrap();
    for (_, _, expected) in cases {
        run_to_system(&mut vm);
        assert!(cmvs_rs::text::try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(vm.state().system_integer(1), Some(9));
    // A matching trailing lead byte walks past the first string's terminator.
    run_to_system(&mut vm);
    assert!(cmvs_rs::text::try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0xf1));
    assert_eq!(vm.stack().len_bytes(), 8);
}

fn push(code: &mut Vec<u8>, raw: u32) {
    word(code, 0x440);
    word(code, 4);
    code.extend(raw.to_le_bytes());
}
/// 0x20fc arguments: SP-12 value, SP-8 format reference, SP-4 destination reference.
fn format_call(code: &mut Vec<u8>, value: u32, pattern: u32, target: u32) {
    for raw in [value, pattern, target] {
        push(code, raw);
    }
    word(code, 0x20fc);
}

#[test]
fn format_0x20fc_writes_zero_padded_low_digits_into_a_string_slot() {
    let text = b"mask%z3a.pb3\0%z+12/100%%\0";
    let mut code = vec![];
    for value in [5, 0, 1234, u32::MAX] {
        format_call(&mut code, value, 0, 0x8000_0031);
    }
    // Width 12 exposes the original's wrapping powers of ten; slot 200 resolves to slot 0.
    format_call(&mut code, 1_234_567_890, 13, 0x8000_00c8);
    let script = script(&code, &[0], text);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 7).unwrap();
    for expected in [
        &b"mask005a.pb3"[..],
        b"mask000a.pb3",
        b"mask234a.pb3",
        b"mask295a.pb3",
    ] {
        run_to_system(&mut vm);
        assert!(cmvs_rs::text::try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().string(49), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.pending_system(), None);
    }
    run_to_system(&mut vm);
    assert!(cmvs_rs::text::try_handle(&mut vm).unwrap());
    assert_eq!(vm.state().string(0), Some(&b"101234567890/100%"[..]));
    assert_eq!(vm.state().system_integer(0), Some(7));
}

#[test]
fn format_0x20fc_rejects_unmodeled_conversions_targets_overlap_and_overflow() {
    // Offsets: %d 0, %za 3, % 7, %z1%z1 9, %z33 16, x%z1 21, 1020 bytes + %z4 26.
    let mut text = b"%d\0%za\0%\0%z1%z1\0%z33\0x%z1\0".to_vec();
    text.extend([b'a'; 1020]);
    text.extend(b"%z4\0");
    let mut cases = vec![];
    for (pattern, target) in [
        (0, 0x8000_0031),
        (3, 0x8000_0031),
        (7, 0x8000_0031),
        (9, 0x8000_0031),
        (16, 0x8000_0031),
        (21, 0),
        (21, 0xc000_0004),
        (0x8000_0003, 0x8000_0003),
        (0x8000_0000, 0x8000_0080),
        (26, 0x8000_0031),
    ] {
        let mut code = vec![];
        format_call(&mut code, 1, pattern, target);
        cases.push(code);
    }
    let mut short = vec![];
    push(&mut short, 0);
    push(&mut short, 0x8000_0031);
    word(&mut short, 0x20fc);
    cases.push(short);
    for code in cases {
        let script = script(&code, &[0], &text);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(3, b"x%z1").unwrap();
        vm.state_mut().set_string(0, b"y%z1").unwrap();
        run_to_system(&mut vm);
        let (pending, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(cmvs_rs::text::try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), pending);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().string(49), None);
        assert_eq!(vm.state().string(3), Some(&b"x%z1"[..]));
        assert_eq!(vm.state().string(0), Some(&b"y%z1"[..]));
    }
}

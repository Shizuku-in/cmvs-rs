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

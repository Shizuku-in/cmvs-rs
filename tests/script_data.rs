use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
};

fn scalar(tag: u16, raw: u32) -> Vec<u8> {
    let mut b = tag.to_le_bytes().to_vec();
    b.extend(raw.to_le_bytes());
    b
}
fn expr(terms: Vec<Vec<u8>>) -> Vec<u8> {
    let mut b = 0x200u16.to_le_bytes().to_vec();
    for term in terms {
        b.extend(term);
    }
    b.extend(0x20fu16.to_le_bytes());
    b
}
fn assign(tag: u16, offset: u32, value: u32) -> Vec<u8> {
    expr(vec![
        scalar(tag, offset),
        scalar(0x100, value),
        0x170u16.to_le_bytes().to_vec(),
    ])
}
fn script(code: &[u8], data: &[u8]) -> Script {
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        data.len() as u32,
        0,
        0,
        0,
        (code.len() + data.len()) as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(data)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    Script::decode(&raw).unwrap()
}

#[test]
fn data_words_are_unaligned_little_endian_and_alias_across_tags() {
    let mut code = assign(0x106, 1, 0x12345678);
    code.extend(expr(vec![scalar(0x104, 1)]));
    code.extend(expr(vec![
        scalar(0x106, 1),
        scalar(0x100, 2),
        0x175u16.to_le_bytes().to_vec(),
    ]));
    code.extend(assign(0x106, 0, u32::from_le_bytes(*b"cat\0")));
    let script = script(&code, &[0xaa; 6]);
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    assert_eq!(vm.script_data(), [0xaa, 0x78, 0x56, 0x34, 0x12, 0xaa]);
    assert_eq!(script.unknown(), [0xaa; 6]);
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(0x12345678));
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(0x1234567a));
    vm.step().unwrap();
    assert_eq!(vm.resolve_string(0x4000_0000).unwrap(), b"cat");
}

#[test]
fn string_expression_failure_rolls_back_nested_numeric_data_writes() {
    let mut code = 0x201u16.to_le_bytes().to_vec();
    code.extend(0x121u16.to_le_bytes());
    code.extend(assign(0x106, 0, 65));
    code.extend(scalar(0x125, 0));
    code.extend(0x160u16.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    let script = script(&code, b"old\0");
    let mut vm = Machine::new(&script).unwrap();
    assert!(vm.step().is_err()); // string bank slot 65 is unknown
    assert_eq!(vm.script_data(), b"old\0");
    assert_eq!(vm.pc(), 0);
    assert_eq!(vm.state().integer(4096), None);
}

#[test]
fn failed_nested_expression_restores_overlapping_data_and_state() {
    let code = expr(vec![
        assign(0x106, 0, 1),
        assign(0x104, 1, 2),
        0x163u16.to_le_bytes().to_vec(),
        scalar(0x100, 0),
        0x161u16.to_le_bytes().to_vec(),
    ]);
    let script = script(&code, &[0xaa; 8]);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_integer(4096, 91).unwrap();
    assert!(vm.step().is_err());
    assert_eq!(vm.pc(), 0);
    assert_eq!(vm.script_data(), [0xaa; 8]);
    assert_eq!(vm.state().integer(4096), Some(91));
    assert_eq!(vm.state().result, None);
    assert!(vm.step().is_err());
    assert_eq!(vm.script_data(), [0xaa; 8]);
}

#[test]
fn negative_and_truncated_data_addresses_are_errors_even_for_assignment() {
    for offset in [u32::MAX, 4, u32::MAX / 2] {
        let script = script(&assign(0x106, offset, 7), &[0; 7]);
        let mut vm = Machine::new(&script).unwrap();
        assert!(vm.step().is_err());
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.script_data(), [0; 7]);
    }
    let empty = script(&expr(vec![scalar(0x106, 0)]), &[]);
    assert!(Machine::new(&empty).unwrap().step().is_err());
}

#[test]
fn common_script_data_is_isolated_and_persists_across_calls() {
    let mut code = assign(0x106, 0, 11);
    for _ in 0..2 {
        code.extend(expr(vec![scalar(0x100, 0)]));
        code.extend(0x430u16.to_le_bytes());
    }
    code.extend(0x2081u16.to_le_bytes());
    code.extend(expr(vec![scalar(0x106, 0)]));
    // Host callback call consumes one stack argument.
    code.extend(expr(vec![scalar(0x100, 0)]));
    code.extend(0x430u16.to_le_bytes());
    code.extend(0x208au16.to_le_bytes());
    code.extend(0x2000u16.to_le_bytes());
    let root = script(&code, &[0; 4]);
    let mut vm = Machine::new(&root).unwrap();
    loop {
        if matches!(vm.step().unwrap(), Step::System { .. }) {
            break;
        }
    }
    let mut common = assign(0x106, 0, 77);
    let ret = common.len();
    common.extend([0x14, 0x04, 0, 0]);
    vm.enter_common(0, b"synthetic.ps3".to_vec(), script(&common, &[0; 4]))
        .unwrap();
    vm.step().unwrap();
    assert_eq!(vm.script_data(), 77u32.to_le_bytes());
    vm.step().unwrap();
    assert_eq!(vm.current_slot(), 0);
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(11));
    loop {
        if matches!(vm.step().unwrap(), Step::System { .. }) {
            break;
        }
    }
    vm.call_script(1, ret).unwrap();
    assert_eq!(vm.script_data(), 77u32.to_le_bytes());
    vm.step().unwrap();
    assert_eq!(vm.script_data(), 11u32.to_le_bytes());
}

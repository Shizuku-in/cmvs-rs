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
/// Data array: base dword, then the index expression.
fn data_array(tag: u16, base: u32, index: Vec<u8>) -> Vec<u8> {
    let mut b = tag.to_le_bytes().to_vec();
    b.extend(base.to_le_bytes());
    b.extend(index);
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

#[test]
fn data_arrays_address_base_plus_four_times_index_like_tag_0x106() {
    for tag in [0x105, 0x110] {
        // 0x110[2 + 4 * 1] = 0x11223344, read back through 0x106 at byte 6, then += 2.
        let mut code = expr(vec![
            data_array(tag, 2, expr(vec![scalar(0x100, 1)])),
            scalar(0x100, 0x11223344),
            0x170u16.to_le_bytes().to_vec(),
        ]);
        code.extend(expr(vec![scalar(0x106, 6)]));
        code.extend(expr(vec![
            data_array(tag, 2, expr(vec![scalar(0x100, 1)])),
            scalar(0x100, 2),
            0x175u16.to_le_bytes().to_vec(),
        ]));
        // The base and scaled index add in u32 arithmetic: -8 + 4 * 3 = byte 4.
        code.extend(expr(vec![data_array(
            tag,
            (-8i32) as u32,
            expr(vec![scalar(0x100, 3)]),
        )]));
        let script = script(&code, &[0; 12]);
        let mut vm = Machine::new(&script).unwrap();
        vm.step().unwrap();
        assert_eq!(
            vm.script_data(),
            [0, 0, 0, 0, 0, 0, 0x44, 0x33, 0x22, 0x11, 0, 0]
        );
        vm.step().unwrap();
        assert_eq!(vm.state().result, Some(0x11223344));
        vm.step().unwrap();
        assert_eq!(vm.state().result, Some(0x11223346));
        vm.step().unwrap();
        assert_eq!(vm.state().result, Some(0x3346_0000));
        assert_eq!(vm.state().integer(4096), Some(0x3346_0000));
    }
}

#[test]
fn data_array_addresses_outside_the_segment_are_errors() {
    for tag in [0x105, 0x110] {
        for (base, index) in [(0, u32::MAX), (4, 1), (0, 0x2000_0000)] {
            let code = expr(vec![
                data_array(tag, base, expr(vec![scalar(0x100, index)])),
                scalar(0x100, 7),
                0x170u16.to_le_bytes().to_vec(),
            ]);
            let script = script(&code, &[0; 11]);
            let mut vm = Machine::new(&script).unwrap();
            assert!(vm.step().is_err(), "{base} {index}");
            assert_eq!(vm.pc(), 0);
            assert_eq!(vm.script_data(), [0; 11]);
        }
    }
}

#[test]
fn data_array_index_writes_roll_back_when_the_expression_fails() {
    for tag in [0x105, 0x110] {
        // The index expression writes byte 0 and yields 1; reading unknown integer 100 fails.
        let mut unknown = 0x101u16.to_le_bytes().to_vec();
        unknown.extend(expr(vec![scalar(0x100, 100)]));
        let code = expr(vec![
            data_array(tag, 0, assign(0x106, 0, 1)),
            unknown,
            0x170u16.to_le_bytes().to_vec(),
        ]);
        let script = script(&code, &[0xaa; 8]);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_integer(4096, 91).unwrap();
        assert!(vm.step().is_err());
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.script_data(), [0xaa; 8]);
        assert_eq!(vm.state().integer(4096), Some(91));
    }
}

#[test]
fn data_array_0x105_preserves_lazy_aliases_and_wraps_scaled_indices() {
    let mut code = expr(vec![
        data_array(0x105, 0, expr(vec![scalar(0x100, 0)])),
        assign(0x104, 0, 7),
        0x163u16.to_le_bytes().to_vec(),
    ]);
    code.extend(expr(vec![data_array(
        0x105,
        0,
        expr(vec![scalar(0x100, 0x4000_0001)]),
    )]));
    code.extend(expr(vec![data_array(
        0x105,
        8,
        expr(vec![scalar(0x100, u32::MAX)]),
    )]));
    let script = script(&code, &[1, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]);
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(14)); // descriptor reads the later value 7
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(0x12345678)); // scaled index wraps to 4
    vm.step().unwrap();
    assert_eq!(vm.state().result, Some(0x12345678)); // negative index, valid offset
}

#[test]
fn failed_data_array_0x105_restores_overlapping_index_and_destination_writes() {
    let write = expr(vec![
        data_array(0x105, 1, assign(0x104, 0, 0)),
        scalar(0x100, 0x11223344),
        0x170u16.to_le_bytes().to_vec(),
    ]);
    let code = expr(vec![
        write,
        scalar(0x100, 0),
        0x161u16.to_le_bytes().to_vec(),
    ]);
    let script = script(&code, &[0xaa; 8]);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_integer(4096, 91).unwrap();
    vm.state_mut().result = Some(123);
    vm.state_mut().condition = Some(false);
    vm.state_mut().set_system_integer(0, 456).unwrap();
    for _ in 0..2 {
        assert!(vm.step().is_err());
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.script_data(), [0xaa; 8]);
        assert_eq!(vm.state().integer(4096), Some(91));
        assert_eq!(vm.state().result, Some(123));
        assert_eq!(vm.state().condition, Some(false));
        assert_eq!(vm.state().system_integer(0), Some(456));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

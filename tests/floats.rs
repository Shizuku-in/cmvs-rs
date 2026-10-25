use cmvs_rs::{
    bytecode::{self, Expression, InstructionKind},
    evaluate::State,
    machine::Machine,
    ps3::Script,
};

fn scalar(tag: u16, raw: u32) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend(raw.to_le_bytes());
    bytes
}
fn float(value: f32) -> Vec<u8> {
    scalar(0x12a, value.to_bits())
}
fn expr(kind: u16, terms: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = kind.to_le_bytes().to_vec();
    for term in terms {
        bytes.extend(term);
    }
    bytes.extend(0x20fu16.to_le_bytes());
    bytes
}
fn var(tag: u16, index: i32) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend(expr(0x200, vec![scalar(0x100, index as u32)]));
    bytes
}
fn op(opcode: u16) -> Vec<u8> {
    opcode.to_le_bytes().to_vec()
}
fn parse(code: &[u8]) -> Expression {
    match bytecode::decode_at(code, 0).unwrap().kind {
        InstructionKind::Expression(e) => e,
        _ => panic!("not an expression"),
    }
}
fn eval(state: &mut State, terms: Vec<Vec<u8>>) -> Result<f32, bytecode::DecodeError> {
    state.evaluate_float_with_labels(&parse(&expr(0x202, terms)), &[])
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

#[test]
fn float_variable_assignment_matches_main_0x39210() {
    let mut state = State::default();
    let value = eval(&mut state, vec![var(0x12b, 120), float(1.0), op(0x170)]).unwrap();
    assert_eq!(value, 1.0);
    assert_eq!(state.float_bits(120), Some(1.0f32.to_bits()));
    assert_eq!(state.result, Some(1.0f32.to_bits()));
    assert_eq!(state.condition, Some(true));
    // Only the nested index expression touches integer 4096.
    assert_eq!(state.integer(4096), Some(120));
    eval(&mut state, vec![float(-0.0)]).unwrap();
    assert_eq!(
        (state.result, state.condition),
        (Some(0x8000_0000), Some(false))
    );
}

#[test]
fn integer_storage_converts_with_truncation_and_x86_indefinite() {
    let mut state = State::default();
    for (value, expected) in [
        (2.75, 2),
        (-2.75, -2),
        (f32::NAN, i32::MIN),
        (3e9, i32::MIN),
    ] {
        eval(&mut state, vec![var(0x101, 1), float(value), op(0x170)]).unwrap();
        assert_eq!(state.integer(1), Some(expected));
    }
    for (value, expected) in [(0.5, false), (1.5, true), (-1.0, true)] {
        eval(&mut state, vec![var(0x102, 2), float(value), op(0x170)]).unwrap();
        assert_eq!(state.bit(2), Some(expected));
    }
    state.set_integer(3, 7).unwrap();
    assert_eq!(
        eval(&mut state, vec![var(0x101, 3), float(0.5), op(0x163)]).unwrap(),
        7.5
    );
    assert_eq!(
        eval(
            &mut state,
            vec![scalar(0x100, (-4i32) as u32), float(0.5), op(0x160)]
        )
        .unwrap(),
        -2.0
    );
}

#[test]
fn operators_follow_float_and_truncating_integer_paths() {
    let mut state = State::default();
    let cases: [(f32, f32, u16, f32); 10] = [
        (1.5, 2.25, 0x163, 3.75),
        (1.5, 2.25, 0x164, -0.75),
        (1.5, 2.0, 0x160, 3.0),
        (5.0, 0.0, 0x161, f32::INFINITY),
        (7.5, 2.0, 0x162, 1.0),
        (6.9, 3.2, 0x165, 2.0),
        (1.9, 4.0, 0x168, 16.0),
        (2.0, 1.0, 0x16a, 1.0),
        (2.0, 2.0, 0x16d, 1.0),
        (0.0, 3.0, 0x16f, 1.0),
    ];
    for (left, right, opcode, expected) in cases {
        let value = eval(&mut state, vec![float(left), float(right), op(opcode)]).unwrap();
        assert_eq!(value, expected, "opcode {opcode:#x}");
    }
    for terms in [
        vec![float(f32::NAN), float(1.0), op(0x171)],
        vec![float(1.0), float(0.0), op(0x162)],
    ] {
        assert!(eval(&mut state, terms).is_err());
    }
}

#[test]
fn compound_increment_and_failed_expressions_roll_back() {
    let mut state = State::default();
    eval(&mut state, vec![var(0x12b, 5), float(2.0), op(0x170)]).unwrap();
    assert_eq!(
        eval(&mut state, vec![var(0x12b, 5), float(0.5), op(0x176)]).unwrap(),
        1.5
    );
    assert_eq!(
        eval(&mut state, vec![var(0x12b, 5), op(0x173)]).unwrap(),
        2.5
    );
    let before = (state.float_bits(5), state.result, state.integer(4096));
    let failing = vec![
        var(0x12b, 5),
        float(9.0),
        op(0x170),
        var(0x12b, 6),
        op(0x163),
    ];
    assert!(eval(&mut state, failing).is_err());
    assert_eq!(
        (state.float_bits(5), state.result, state.integer(4096)),
        before
    );
    assert!(eval(&mut state, vec![var(0x12b, 2048), float(1.0), op(0x170)]).is_err());
    assert!(eval(&mut state, vec![scalar(0x108, 0), float(1.0), op(0x170)]).is_err());
}

#[test]
fn frame_locals_at_depth_zero_and_arguments_use_the_stack() {
    let mut code = vec![];
    for value in [0u32, 0, 9] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(expr(
        0x200,
        vec![scalar(0x108, 0), scalar(0x100, 5), op(0x170)],
    ));
    code.extend(expr(
        0x200,
        vec![scalar(0x108, 0), scalar(0x100, 1), op(0x163)],
    ));
    code.extend(expr(0x202, vec![scalar(0x12c, 4), float(1.5), op(0x170)]));
    code.extend(expr(
        0x200,
        vec![
            scalar(0x108, 0),
            scalar(0x100, 2),
            op(0x170),
            scalar(0x108, 64),
            op(0x163),
        ],
    ));
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..5 {
        vm.step().unwrap();
    }
    assert_eq!(vm.stack().read_u32_at(0).unwrap(), 5);
    assert_eq!(vm.state().result, Some(6));
    vm.step().unwrap();
    assert_eq!(vm.stack().read_u32_at(4).unwrap(), 1.5f32.to_bits());
    assert_eq!(vm.stack().read_u32_at(8).unwrap(), 9);
    // Reading past the live stack fails and restores the earlier frame write.
    assert!(vm.step().is_err());
    assert_eq!(vm.stack().read_u32_at(0).unwrap(), 5);
    assert_eq!(vm.state().result, Some(1.5f32.to_bits()));
}

#[test]
fn frame_arguments_are_read_below_the_call_frame() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(41u32.to_le_bytes());
    code.extend(0x410u16.to_le_bytes());
    code.extend(1u16.to_le_bytes());
    let function = code.len() as u32;
    code.extend(expr(
        0x200,
        vec![scalar(0x103, 8), scalar(0x100, 1), op(0x163)],
    ));
    code.extend(expr(0x200, vec![scalar(0x103, 12)]));
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(function.to_le_bytes());
    payload.extend(&code);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        2,
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
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..3 {
        vm.step().unwrap();
    }
    assert_eq!(vm.state().result, Some(42));
    assert!(vm.step().is_err());
}

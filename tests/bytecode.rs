use cmvs_rs::bytecode::{self, InstructionKind, Term};

fn word(code: &mut Vec<u8>, value: u16) {
    code.extend(value.to_le_bytes());
}
fn scalar(code: &mut Vec<u8>, tag: u16, raw: u32) {
    word(code, tag);
    code.extend(raw.to_le_bytes());
}
fn integer(value: i32) -> Vec<u8> {
    let mut code = vec![];
    word(&mut code, 0x200);
    scalar(&mut code, 0x100, value as u32);
    word(&mut code, 0x20f);
    code
}
fn expression(code: &[u8]) -> cmvs_rs::bytecode::Expression {
    match bytecode::decode_at(code, 0).unwrap().kind {
        InstructionKind::Expression(expr) => expr,
        _ => panic!("not an expression"),
    }
}
fn binary(left: i32, right: i32, operator: u16) -> Vec<u8> {
    let mut code = integer(left);
    code.truncate(code.len() - 2);
    scalar(&mut code, 0x100, right as u32);
    word(&mut code, operator);
    word(&mut code, 0x20f);
    code
}

#[test]
fn literals_containing_opcode_bytes_are_not_instructions() {
    let mut code = integer(0x2000020f);
    word(&mut code, 0x430);
    word(&mut code, 0x201e);
    let decoded = bytecode::decode_layout(&code, 0, &[0]).unwrap();
    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded[1].pc, 10);
    assert_eq!(decoded[2].kind, InstructionKind::System { command: 30 });
}

#[test]
fn signed_arithmetic_boolean_and_x86_shift_mask() {
    for (left, right, op, want) in [
        (i32::MAX, 1, 0x163, i32::MIN),
        (-7, 3, 0x161, -2),
        (-7, 3, 0x162, -1),
        (-8, 1, 0x169, -4),
        (1, 33, 0x168, 2),
        (1, -1, 0x168, i32::MIN),
        (-1, 0, 0x16c, 1),
        (3, 8, 0x16e, 1),
        (0, 8, 0x16e, 0),
        (0, 8, 0x16f, 1),
        (3, 3, 0x171, 1),
        (3, 3, 0x172, 0),
        (6, 3, 0x160, 18),
        (6, 3, 0x164, 3),
        (6, 3, 0x165, 2),
        (6, 3, 0x166, 7),
        (6, 3, 0x167, 5),
        (2, 1, 0x16a, 1),
        (2, 2, 0x16b, 1),
        (2, 2, 0x16d, 1),
    ] {
        assert_eq!(
            bytecode::eval_constant(&expression(&binary(left, right, op))).unwrap(),
            want
        );
    }
}

#[test]
fn postfix_operators_consume_top_pair_in_order() {
    let mut code = integer(10);
    code.truncate(code.len() - 2);
    scalar(&mut code, 0x100, 2);
    scalar(&mut code, 0x100, 3);
    word(&mut code, 0x160);
    word(&mut code, 0x164);
    word(&mut code, 0x20f);
    assert_eq!(bytecode::eval_constant(&expression(&code)).unwrap(), 4);
}

#[test]
fn invalid_arithmetic_and_stack_shapes_fail() {
    for (a, b, op) in [(1, 0, 0x161), (i32::MIN, -1, 0x161), (1, 0, 0x162)] {
        assert!(bytecode::eval_constant(&expression(&binary(a, b, op))).is_err());
    }
    let mut code = integer(1);
    code.truncate(code.len() - 2);
    word(&mut code, 0x163);
    word(&mut code, 0x20f);
    assert!(bytecode::eval_constant(&expression(&code)).is_err());
    assert!(bytecode::eval_constant(&expression(&binary(1, 2, 0x170))).is_err());
}

#[test]
fn nested_constant_and_symbolic_variable_are_distinct() {
    let mut nested = vec![];
    word(&mut nested, 0x200);
    nested.extend(integer(-123));
    word(&mut nested, 0x20f);
    assert_eq!(bytecode::eval_constant(&expression(&nested)).unwrap(), -123);
    let mut reference = vec![];
    word(&mut reference, 0x200);
    word(&mut reference, 0x101);
    reference.extend(integer(2400));
    word(&mut reference, 0x20f);
    let expr = expression(&reference);
    assert!(matches!(expr.terms[0], Term::Nested { tag: 0x101, .. }));
    assert!(bytecode::eval_constant(&expr).is_err());
}

#[test]
fn string_reference_and_float_bits_preserved() {
    for (kind, tag, value) in [(0x201, 0x120, 351), (0x202, 0x12a, 0x80000000)] {
        let mut code = vec![];
        word(&mut code, kind);
        scalar(&mut code, tag, value);
        word(&mut code, 0x20f);
        let expr = expression(&code);
        assert!(matches!(expr.terms[0], Term::Scalar { raw, .. } if raw == value));
        assert!(bytecode::eval_constant(&expr).is_err());
    }
}

#[test]
fn indexed_operand_boundaries() {
    let mut code = vec![];
    word(&mut code, 0x200);
    scalar(&mut code, 0x111, 40);
    word(&mut code, 16);
    code.extend(integer(2));
    code.extend(integer(3));
    word(&mut code, 0x20f);
    let expr = expression(&code);
    assert!(
        matches!(&expr.terms[0], Term::Indexed { base: 40, stride: 16, indices, .. } if indices.len() == 2)
    );
    assert_eq!(expr.end, code.len());
}

#[test]
fn every_expression_prefix_and_unknown_token_fail() {
    let code = binary(2, 3, 0x163);
    for end in 0..code.len() {
        assert!(bytecode::decode_at(&code[..end], 0).is_err());
    }
    assert!(bytecode::decode_at(&[0, 2, 0xff, 0x01], 0).is_err());
    assert!(bytecode::decode_at(&code, usize::MAX).is_err());
}

#[test]
fn depth_limit_stops_malicious_nesting() {
    let mut code = vec![];
    for _ in 0..70 {
        word(&mut code, 0x200);
    }
    code.extend(integer(1));
    for _ in 0..70 {
        word(&mut code, 0x20f);
    }
    assert_eq!(
        bytecode::decode_at(&code, 0).unwrap_err().reason,
        "expression nesting limit"
    );
}

#[test]
fn jump_0400_is_six_bytes_not_nop() {
    let mut code = vec![];
    scalar(&mut code, 0x400, 0);
    let instructions = bytecode::decode_layout(&code, 0, &[0]).unwrap();
    assert_eq!(instructions[0].end, 6);
    assert_eq!(instructions[0].kind, InstructionKind::Jump { target: 0 });
    code[2..6].copy_from_slice(&2u32.to_le_bytes());
    assert!(bytecode::decode_layout(&code, 0, &[0]).is_err());
}

#[test]
fn bad_label_indices_and_entry_inside_operand_fail() {
    let mut code = vec![];
    word(&mut code, 0x410);
    word(&mut code, 1);
    assert!(bytecode::decode_layout(&code, 0, &[0]).is_err());
    let code = integer(0);
    assert!(bytecode::decode_layout(&code, 2, &[0]).is_err());
    assert!(bytecode::decode_layout(&code, 0, &[2]).is_err());
}

#[test]
fn bounded_random_inputs_do_not_panic() {
    let mut state = 0x56e8d123u32;
    for size in 0..256 {
        let mut bytes = vec![0; size];
        for b in &mut bytes {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = state as u8;
        }
        let _ = bytecode::decode_at(&bytes, 0);
    }
}

#[test]
fn string_assignment_is_structurally_decoded_without_claiming_numeric_evaluation() {
    let mut code = vec![];
    word(&mut code, 0x201);
    scalar(&mut code, 0x125, 12);
    scalar(&mut code, 0x120, 0x20f0170);
    word(&mut code, 0x170);
    word(&mut code, 0x20f);
    let expr = expression(&code);
    assert_eq!(expr.end, code.len());
    assert!(bytecode::eval_constant(&expr).is_err());
}

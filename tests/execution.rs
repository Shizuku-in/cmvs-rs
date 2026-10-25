use cmvs_rs::{
    bytecode::{self, Expression, InstructionKind},
    evaluate::State,
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
fn int(value: i32) -> Vec<u8> {
    expr(0x200, vec![scalar(0x100, value as u32)])
}
fn reference(tag: u16, index: i32) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend(int(index));
    bytes
}
fn operator(op: u16) -> Vec<u8> {
    op.to_le_bytes().to_vec()
}
fn assignment(tag: u16, index: i32, value: i32) -> Vec<u8> {
    expr(
        0x200,
        vec![
            reference(tag, index),
            scalar(0x100, value as u32),
            operator(0x170),
        ],
    )
}
fn expression(code: &[u8]) -> Expression {
    match bytecode::decode_at(code, 0).unwrap().kind {
        InstructionKind::Expression(e) => e,
        _ => panic!("not expression"),
    }
}
fn evaluate(state: &mut State, code: &[u8]) -> Result<i32, bytecode::DecodeError> {
    state.evaluate_integer(&expression(code))
}
fn fixture(code: &[u8], labels: &[u32], text: &[u8]) -> Vec<u8> {
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
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&fixture(code, &[0], &[])).unwrap()
}

#[test]
fn assignment_initializes_unknown_without_reading_it() {
    let mut state = State::default();
    assert_eq!(state.integer(100), None);
    assert_eq!(
        evaluate(&mut state, &assignment(0x101, 100, 37)).unwrap(),
        37
    );
    assert_eq!(state.integer(100), Some(37));
    assert_eq!(state.integer(4096), Some(37));
    assert_eq!(state.result, Some(37));
    assert_eq!(state.condition, Some(true));
}

#[test]
fn bits_normalize_storage_but_assignment_returns_original_integer() {
    let mut state = State::default();
    assert_eq!(evaluate(&mut state, &assignment(0x102, 9, 42)).unwrap(), 42);
    assert_eq!(state.bit(9), Some(true));
    assert_eq!(
        evaluate(&mut state, &expr(0x200, vec![reference(0x102, 9)])).unwrap(),
        1
    );
    evaluate(&mut state, &assignment(0x102, 9, 0)).unwrap();
    assert_eq!(state.bit(9), Some(false));
}

#[test]
fn unknown_read_and_invalid_index_do_not_guess_or_commit_nested_results() {
    let mut state = State::default();
    for (tag, index) in [
        (0x101, 10),
        (0x102, 5),
        (0x101, -1),
        (0x101, 8192),
        (0x102, 65536),
    ] {
        assert!(evaluate(&mut state, &expr(0x200, vec![reference(tag, index)])).is_err());
        assert_eq!(state.result, None);
        assert_eq!(state.integer(4096), None);
    }
}

#[test]
fn compound_assignments_match_binary_operations() {
    for (op, expected) in [
        (0x175, 9),
        (0x176, 3),
        (0x177, 18),
        (0x178, 2),
        (0x179, 2),
        (0x17a, 7),
        (0x17b, 0),
        (0x17c, 5),
        (0x17d, 48),
        (0x17e, 0),
    ] {
        let mut state = State::default();
        state.set_integer(8, 6).unwrap();
        let code = expr(
            0x200,
            vec![reference(0x101, 8), scalar(0x100, 3), operator(op)],
        );
        assert_eq!(evaluate(&mut state, &code).unwrap(), expected);
        assert_eq!(state.integer(8), Some(expected));
    }
}

#[test]
fn increment_and_decrement_return_updated_value_and_wrap() {
    let mut state = State::default();
    state.set_integer(8, i32::MAX).unwrap();
    let inc = expr(0x200, vec![reference(0x101, 8), operator(0x173)]);
    assert_eq!(evaluate(&mut state, &inc).unwrap(), i32::MIN);
    let dec = expr(0x200, vec![reference(0x101, 8), operator(0x174)]);
    assert_eq!(evaluate(&mut state, &dec).unwrap(), i32::MAX);
}

#[test]
fn descriptors_resolve_after_nested_side_effects() {
    let mut state = State::default();
    state.set_integer(8, 1).unwrap();
    let code = expr(
        0x200,
        vec![
            reference(0x101, 8),
            assignment(0x101, 8, 7),
            operator(0x163),
        ],
    );
    assert_eq!(evaluate(&mut state, &code).unwrap(), 14);
}

#[test]
fn failed_outer_expression_rolls_back_all_nested_assignments() {
    let mut state = State::default();
    state.set_integer(8, 1).unwrap();
    let code = expr(
        0x200,
        vec![assignment(0x101, 8, 7), scalar(0x100, 0), operator(0x161)],
    );
    assert!(evaluate(&mut state, &code).is_err());
    assert_eq!(state.integer(8), Some(1));
    assert_eq!(state.integer(4096), None);
    assert_eq!(state.result, None);
}

#[test]
fn literal_cannot_be_assigned_and_policy_limits_are_atomic() {
    let code = expr(
        0x200,
        vec![scalar(0x100, 1), scalar(0x100, 2), operator(0x170)],
    );
    assert!(evaluate(&mut State::default(), &code).is_err());
    let mut state = State::new(100, 100);
    assert!(evaluate(&mut state, &assignment(0x101, 8, 7)).is_err()); // slot 4096 unavailable
    assert_eq!(state.integer(8), None);
}

#[test]
fn pending_system_is_stable_and_resume_reclaims_exact_byte_count() {
    let mut code = int(42);
    word(&mut code, 0x430);
    word(&mut code, 0x201e);
    word(&mut code, 0x2000);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    assert_eq!(vm.step().unwrap(), Step::Continue);
    assert_eq!(vm.step().unwrap(), Step::Continue);
    let request = Step::System {
        pc: 12,
        command: 30,
    };
    assert_eq!(vm.step().unwrap(), request);
    assert_eq!(vm.step().unwrap(), request);
    assert!(vm.complete_system(SystemResult(0x4008)).is_err());
    assert_eq!(vm.pc(), 12);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(
        vm.complete_system(SystemResult(0x4004)).unwrap(),
        Step::Continue
    );
    assert_eq!(vm.pc(), 14);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.step().unwrap(), Step::Yield);
    assert_eq!(vm.pc(), 16);
}

#[test]
fn integer_expression_updates_condition_and_branch_execution() {
    for (value, op, jumps) in [
        (0, 0x401, true),
        (1, 0x401, false),
        (1, 0x402, true),
        (0, 0x402, false),
    ] {
        let mut code = int(value);
        code.extend(scalar(op, 26));
        code.extend(int(99));
        word(&mut code, 0x2000);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.step().unwrap();
        vm.step().unwrap();
        assert_eq!(vm.pc(), if jumps { 26 } else { 16 });
    }
}

#[test]
fn unknown_condition_and_unsupported_instruction_leave_pc_unchanged() {
    let code = scalar(0x401, 0);
    let first_script = script(&code);
    let mut vm = Machine::new(&first_script).unwrap();
    assert!(vm.step().is_err());
    assert_eq!(vm.pc(), 0);
    let mut code = vec![];
    word(&mut code, 0x440);
    word(&mut code, 4);
    code.extend(7u32.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    assert!(vm.step().is_err());
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn call_label_and_return_use_shared_parameter_stack() {
    let mut code = vec![];
    word(&mut code, 0x410);
    word(&mut code, 1);
    word(&mut code, 0x2000);
    code.extend(int(7));
    word(&mut code, 0x411);
    word(&mut code, 0);
    let script = Script::decode(&fixture(&code, &[0, 6], &[])).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    assert_eq!(vm.pc(), 6);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 4);
    vm.step().unwrap();
    vm.step().unwrap();
    assert_eq!(vm.pc(), 4);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().result, Some(7));
    assert_eq!(vm.step().unwrap(), Step::Yield);
}

#[test]
fn script_switch_cannot_be_resumed_as_an_ordinary_command() {
    let mut code = vec![];
    word(&mut code, 0x2080);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    assert!(vm.complete_system(SystemResult(0)).is_err());
    assert!(vm.complete_system(SystemResult(0x4000)).is_err());
    assert_eq!(vm.pc(), 0);
}

#[test]
fn string_literal_preserves_integer_condition_and_negative_zero_float_clears_it() {
    let mut code = int(1);
    code.extend(expr(0x201, vec![scalar(0x120, 0)]));
    code.extend(expr(0x202, vec![scalar(0x12a, 0x80000000)]));
    let script = Script::decode(&fixture(&code, &[0], b"asset\0")).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    vm.step().unwrap();
    assert_eq!(vm.state().condition, Some(true));
    assert_eq!(vm.state().result, Some(0));
    vm.step().unwrap();
    assert_eq!(vm.state().condition, Some(false));
    assert_eq!(vm.state().result, Some(0x80000000));
}

#[test]
fn cli_step_budget_stops_a_real_control_flow_loop() {
    let path = std::env::temp_dir().join(format!("cmvs-loop-test-{}.ps3", std::process::id()));
    std::fs::write(&path, fixture(&scalar(0x400, 0), &[0], &[])).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&path)
        .args(["--steps", "3"])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("budget exhausted"));
}

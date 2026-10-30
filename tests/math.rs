use cmvs_rs::{
    machine::{Machine, Step},
    math,
    ps3::Script,
};

fn script(commands: &[(u16, &[u32])]) -> Script {
    let mut code = vec![];
    for (op, args) in commands {
        for arg in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    }
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

#[test]
fn abs_clamp_max_and_min_are_signed_and_pop_their_arguments() {
    let neg = |v: i32| v as u32;
    let cases: [(u16, &[u32], i32); 7] = [
        (0x20d0, &[neg(-5)], 5),
        (0x20d0, &[neg(i32::MIN)], i32::MIN),
        (0x20d1, &[neg(-3), 0, 10], 0),
        (0x20d1, &[20, 0, 10], 10),
        // The upper bound wins when the bounds cross.
        (0x20d1, &[5, 8, 2], 2),
        (0x20d2, &[neg(-1), neg(-7)], -1),
        (0x20d3, &[neg(-1), neg(-7)], -7),
    ];
    let commands: Vec<(u16, &[u32])> = cases.iter().map(|&(op, args, _)| (op, args)).collect();
    let script = script(&commands);
    let mut vm = Machine::new(&script).unwrap();
    for &(_, _, expected) in &cases {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert!(math::try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected as u32));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

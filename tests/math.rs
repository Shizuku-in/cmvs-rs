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
fn inspected_polar_point_writes_xy_results_with_signed_wrapping_and_truncation() {
    use cmvs_rs::{
        dispatch::{self, Flow, Hosts},
        profile::Registry,
    };
    let profile = Registry::builtin().profile("cmvs-20071117").unwrap();
    for (angle, radius, xy) in [
        (0, 100, [100, 0]),
        (90, 100, [0, 99]),
        (180, 100, [-99, 0]),
        (-90, 100, [0, -99]),
        (0, -100, [-100, 0]),
        (i32::MAX, 100, [99, -1]),
        (0, i32::MIN, [i32::MIN, 0]),
    ] {
        let script = script(&[(0x20d8, &[angle as u32, radius as u32])]);
        let mut vm = Machine::new(&script).unwrap();
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 77).unwrap();
        }
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert_eq!(
            dispatch::run(
                cmvs_rs::profile::Semantics::Root,
                &mut vm,
                &mut Hosts {
                    math: true,
                    ..Default::default()
                }
            )
            .unwrap(),
            None
        );
        assert_eq!(
            dispatch::run(
                profile.semantics(0xd8),
                &mut vm,
                &mut Hosts {
                    math: true,
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(
            [vm.state().system_integer(1), vm.state().system_integer(2)],
            xy.map(|x| Some(x as u32))
        );
        for i in [0, 3, 4, 5, 6] {
            assert_eq!(vm.state().system_integer(i), Some(77));
        }
        assert_eq!(vm.stack().len_bytes(), 0);
    }
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

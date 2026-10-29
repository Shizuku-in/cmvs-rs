use cmvs_rs::{
    gesture::PointerGesture,
    machine::{Machine, Step},
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
    code.extend(0x27ffu16.to_le_bytes());
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
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
fn run(gesture: &mut PointerGesture, vm: &mut Machine<'_>) -> bool {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    gesture.try_handle(vm).unwrap()
}
fn tracking_is_reset(gesture: &PointerGesture) -> bool {
    [44, 88, 172]
        .iter()
        .all(|at| gesture.field(*at) == Some(u32::MAX))
        && [48, 52, 72, 76, 80, 84, 92, 108, 120, 180, 184]
            .iter()
            .all(|at| gesture.field(*at) == Some(0))
}

#[test]
fn defaults_follow_the_constructor() {
    let gesture = PointerGesture::default();
    for at in [0, 4, 8, 12, 16, 20, 24, 28] {
        assert_eq!(gesture.field(at), Some(0));
    }
    assert_eq!((gesture.field(32), gesture.field(36)), (Some(64), Some(90)));
    assert!(tracking_is_reset(&gesture));
    assert_eq!(gesture.field(40), Some(4));
    assert_eq!(gesture.field(116), Some(1));
    for at in (188..=304).step_by(4) {
        assert_eq!(gesture.field(at), Some(0));
    }
}

#[test]
fn region_setter_keeps_raw_rectangles_without_resetting_other_fields_or_results() {
    let script = script(&[
        (0x235a, &[27]),
        (0x2366, &[0, 9, u32::MAX, 2, 0x8000_0000, 4]),
        (0x2366, &[5, 0, 10, 20, 30, 40]),
    ]);
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    assert!(run(&mut gesture, &mut vm));
    vm.state_mut().set_system_integer(0, 123).unwrap();
    assert!(run(&mut gesture, &mut vm));
    assert_eq!(
        (188..=204)
            .step_by(4)
            .map(|at| gesture.field(at).unwrap())
            .collect::<Vec<_>>(),
        [1, u32::MAX, 2, 0x8000_0000, 4]
    );
    assert!(run(&mut gesture, &mut vm));
    assert_eq!(
        (288..=304)
            .step_by(4)
            .map(|at| gesture.field(at).unwrap())
            .collect::<Vec<_>>(),
        [0, 10, 20, 30, 40]
    );
    assert_eq!(gesture.field(36), Some(27));
    assert_eq!(gesture.field(208), Some(0));
    assert_eq!(vm.state().system_integer(0), Some(123));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn region_setter_ignores_unsigned_out_of_range_slots_and_still_consumes_all_words() {
    let script = script(&[
        (0x2366, &[6, 1, 2, 3, 4, 5]),
        (0x2366, &[u32::MAX, 1, 2, 3, 4, 5]),
    ]);
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    let before = gesture.clone();
    for _ in 0..2 {
        assert!(run(&mut gesture, &mut vm));
        assert_eq!(gesture, before);
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn region_setter_requires_all_six_arguments_before_committing() {
    for count in 0..6 {
        let args = vec![0; count];
        let script = script(&[(0x2366, &args)]);
        let mut vm = Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let before_vm = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        let mut gesture = PointerGesture::default();
        let before = gesture.clone();
        assert!(gesture.try_handle(&mut vm).is_err());
        assert_eq!(gesture, before);
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            before_vm
        );
    }
}

#[test]
fn setters_store_normalized_flags_and_argument_order() {
    let script = script(&[
        (0x2350, &[7]),
        (0x2351, &[2, 0]),
        (0x2352, &[5]),
        (0x2356, &[1]),
        (0x2357, &[0]),
        (0x2358, &[9]),
        (0x2364, &[7]),
    ]);
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    for _ in 0..7 {
        assert!(run(&mut gesture, &mut vm));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    let fields: Vec<_> = (0..=28)
        .step_by(4)
        .map(|at| gesture.field(at).unwrap())
        .collect();
    // 0x2351 takes SP-4 into +24 and SP-8 into +28; 0x2364 keeps the raw word.
    assert_eq!(fields, [1, 7, 1, 1, 0, 1, 1, 0]);
    assert!(tracking_is_reset(&gesture));
    assert!(!run(&mut gesture, &mut vm));
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x7ff));
}

#[test]
fn bounded_setters_ignore_out_of_range_values_but_consume_arguments() {
    let script = script(&[
        (0x235a, &[0]),
        (0x235a, &[91]),
        (0x235a, &[1]),
        (0x235b, &[u32::MAX]),
        (0x235b, &[0]),
    ]);
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    let mut seen = vec![];
    for _ in 0..5 {
        assert!(run(&mut gesture, &mut vm));
        assert_eq!(vm.stack().len_bytes(), 0);
        seen.push((gesture.field(36).unwrap(), gesture.field(32).unwrap()));
    }
    assert_eq!(seen, [(90, 64), (90, 64), (1, 64), (1, 64), (1, 0)]);
}

#[test]
fn missing_arguments_keep_the_command_pending() {
    let mut code = 0x440u16.to_le_bytes().to_vec();
    code.extend(4u16.to_le_bytes());
    code.extend(1u32.to_le_bytes());
    code.extend(0x2351u16.to_le_bytes());
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
    let mut raw = b"PS2A".to_vec();
    for n in [48u32, 0, 0, 1, 10, 0, 0, 0, 0, 14, 0] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    vm.step().unwrap();
    vm.step().unwrap();
    assert!(gesture.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system(), Some((8, 0x351)));
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(gesture, PointerGesture::default());
}

#[test]
fn hold_query_reports_zero_without_a_recognized_hold_and_pops_its_argument() {
    let script = script(&[(0x2362, &[1]), (0x2362, &[0])]);
    let mut vm = Machine::new(&script).unwrap();
    let mut gesture = PointerGesture::default();
    for _ in 0..2 {
        vm.state_mut().set_system_integer(0, 9).unwrap();
        assert!(run(&mut gesture, &mut vm));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(0));
    }
    // The query does not reset tracking.
    assert_eq!(gesture, PointerGesture::default());
}

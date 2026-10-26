use cmvs_rs::{
    input::{self, ActionInput},
    machine::{Machine, Step},
    ps3::Script,
};

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
fn commands(ops: &[u16]) -> Script {
    script(
        &ops.iter()
            .flat_map(|op| op.to_le_bytes())
            .collect::<Vec<_>>(),
    )
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
fn results(vm: &Machine<'_>) -> (Option<u32>, Option<u32>) {
    (vm.state().system_integer(0), vm.state().system_integer(4))
}

#[test]
fn queries_report_release_and_hold_of_actions_zero_and_one() {
    let script = commands(&[0x21a0, 0x21a2, 0x21a0]);
    let mut vm = Machine::new(&script).unwrap();
    let mut input = ActionInput::default();
    input.set(0, true, true, false).unwrap();
    input.set(1, false, false, true).unwrap();
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(results(&vm), (Some(0), Some(1)));
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(results(&vm), (Some(1), Some(0)));
    // Queries leave the flags alone; hosts without input report nothing active.
    assert_eq!(vm.stack().len_bytes(), 0);
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut ActionInput::default()).unwrap());
    assert_eq!(results(&vm), (Some(0), Some(0)));
    assert!(input.pressed(0) && input.released(1));
}

#[test]
fn consuming_clears_press_and_release_but_keeps_hold() {
    let script = commands(&[0x21a1, 0x21a3, 0x21a0]);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 7).unwrap();
    let mut input = ActionInput::default();
    input.set(0, true, true, true).unwrap();
    input.set(1, true, true, true).unwrap();
    input.set(2, true, true, true).unwrap();
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(7));
    assert!(input.held(0) && !input.pressed(0) && !input.released(0));
    assert!(input.pressed(1));
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut input).unwrap());
    assert!(input.held(1) && !input.pressed(1) && !input.released(1));
    assert!(input.pressed(2) && input.released(2));
    pending(&mut vm);
    assert!(input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(results(&vm), (Some(0), Some(1)));
}

#[test]
fn skipping_reports_the_script_flag_or_held_skip_actions() {
    let held = |action| {
        let mut input = ActionInput::default();
        input.set(action, true, false, false).unwrap();
        input
    };
    for (input, skip, expected) in [
        (ActionInput::default(), false, 0),
        (ActionInput::default(), true, 1),
        (held(7), false, 1),
        (held(8), false, 1),
        (held(6), false, 0),
        (held(9), true, 1),
    ] {
        let script = commands(&[0x22ef]);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        pending(&mut vm);
        assert!(input::try_handle_skipping(&mut vm, &input, skip).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.pending_system(), None);
    }
}

#[test]
fn neighbouring_commands_are_left_pending() {
    for op in [0x219f, 0x21a4] {
        let script = commands(&[op]);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(!input::try_handle(&mut vm, &mut ActionInput::default()).unwrap());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(op - 0x2000));
    }
}

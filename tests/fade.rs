use cmvs_rs::{
    fade::{self, FadeHost, SkipGate},
    input::ActionInput,
    machine::{Machine, Step},
    ps3::Script,
    scene::SceneHost,
    scene_render,
};

fn command(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for value in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
fn script(code: &[u8]) -> Script {
    let mut raw = b"PS2A".to_vec();
    for value in [
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
        raw.extend(value.to_le_bytes());
    }
    raw.extend(code.iter().map(|v| v.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    loop {
        if matches!(vm.step().unwrap(), Step::System { .. }) {
            break;
        }
    }
}
fn create(direction: u32, duration: u32) -> FadeHost {
    let mut code = vec![];
    command(&mut code, 0x2220, &[direction, 0x80332211, duration, 9]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut fade = FadeHost::default();
    fade.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);
    fade
}

#[test]
fn both_directions_advance_only_on_frames_and_have_distinct_lifetimes() {
    for direction in [0, 1] {
        let mut fade = create(direction, 2000);
        assert_eq!(fade.current().unwrap().color, 0x80332211);
        fade.advance(1000);
        assert_eq!(
            fade.current().unwrap().opacity,
            if direction == 0 { 127 } else { 128 }
        );
        assert_eq!(fade.current().unwrap().remaining(), 1000);
        fade.advance(1000);
        if direction == 0 {
            let end = fade.current().unwrap().clone();
            assert!(end.finished);
            assert_eq!(end.opacity, 255);
            assert_eq!(end.remaining(), u32::MAX);
            fade.advance(500);
            assert_eq!(fade.current(), Some(&end));
        } else {
            assert!(fade.current().is_none());
        }
    }
    let mut fade = create(1, 0);
    assert_eq!(fade.current().unwrap().opacity, 255);
    fade.advance(0);
    assert!(fade.current().is_none());
}

#[test]
fn waits_preserve_pc_stack_and_repeat_count_until_completion() {
    let mut code = vec![];
    command(&mut code, 0x2224, &[0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    let mut fade = create(1, 100);
    for repeat in 1..=3 {
        assert_eq!(
            fade.try_wait(&mut vm, SkipGate::default(), &mut ActionInput::default())
                .unwrap(),
            Some(Step::Yield)
        );
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.repeat_count()),
            (pc, 4, repeat)
        );
        assert_eq!(fade.current().unwrap().elapsed, 0);
        assert_eq!(vm.state().system_integer(0), Some(0));
    }
    fade.advance(100);
    assert_eq!(
        fade.try_wait(&mut vm, SkipGate::default(), &mut ActionInput::default())
            .unwrap(),
        Some(Step::Continue)
    );
    assert_eq!(
        (vm.pc(), vm.stack().len_bytes(), vm.repeat_count()),
        (pc + 2, 0, 0)
    );
    assert!(vm.yield_pending_system().is_err());
}

#[test]
fn skippable_wait_without_input_and_invalid_creation_do_not_commit() {
    let mut code = vec![];
    command(&mut code, 0x2224, &[1]);
    command(&mut code, 0x2220, &[2, 0, 100, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    let mut fade = create(1, 100);
    let before = fade.current().cloned();
    vm.state_mut().set_system_integer(0, 42).unwrap();
    // With effects on, no skip flag and no input, a skippable wait just waits.
    let step = fade.try_wait(&mut vm, SkipGate::default(), &mut ActionInput::default());
    assert_eq!(step.unwrap(), Some(Step::Yield));
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(fade.current(), before.as_ref());
    fade.advance(100);
    fade.try_wait(&mut vm, SkipGate::default(), &mut ActionInput::default())
        .unwrap();
    pending(&mut vm);
    assert!(fade.try_handle(&mut vm).is_err());
    assert_eq!(vm.stack().len_bytes(), 16);
    assert!(fade.current().is_none());
    assert!(before.is_some());
    let program = script(&0x2220u16.to_le_bytes());
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    assert!(fade.try_handle(&mut vm).is_err());
}

#[test]
fn query_force_finish_destroy_and_replace_follow_original_results() {
    let mut code = vec![];
    for op in [0x2223, 0x2222, 0x2223, 0x2221, 0x2223] {
        command(&mut code, op, &[]);
    }
    let program = script(&code);
    for direction in [0, 1] {
        let mut vm = Machine::new(&program).unwrap();
        let mut fade = create(direction, 10);
        for (op, result) in [
            (0x2223, Some(10)),
            (0x2222, None),
            (0x2223, Some(u32::MAX)),
            (0x2221, None),
            (0x2223, Some(u32::MAX)),
        ] {
            pending(&mut vm);
            assert!(fade.try_handle(&mut vm).unwrap());
            if let Some(result) = result {
                assert_eq!(vm.state().system_integer(0), Some(result));
            }
            if op == 0x2222 {
                assert_eq!(fade.current().is_none(), direction == 1);
            }
        }
    }
    let mut fade = create(0, 100);
    fade.advance(50);
    let mut replace = vec![];
    command(&mut replace, 0x2220, &[1, 0, 10, 0]);
    let program = script(&replace);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    fade.try_handle(&mut vm).unwrap();
    assert_eq!(fade.current().unwrap().elapsed, 0);
    assert_eq!(fade.current().unwrap().opacity, 255);
}

#[test]
fn solid_fade_composites_color_and_retains_opaque_end_state() {
    let mut code = vec![];
    command(&mut code, 0x22cb, &[0xffffff]);
    command(&mut code, 0x2220, &[0, 0x80112233, 100, 9]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..2 {
        pending(&mut vm);
        scene.try_handle(&mut vm).unwrap();
    }
    assert_eq!(
        scene_render::render(&scene, 1, 1).unwrap().pixels(),
        &[255; 4]
    );
    scene.advance_frame(50);
    assert_eq!(
        scene_render::render(&scene, 1, 1).unwrap().pixels(),
        &[136, 145, 153, 255]
    );
    scene.advance_frame(50);
    assert_eq!(
        scene_render::render(&scene, 1, 1).unwrap().pixels(),
        &[0x11, 0x22, 0x33, 255]
    );
}

#[test]
fn elapsed_and_product_use_u32_wrapping_and_wait_uses_signed_remaining() {
    let mut fade = create(0, u32::MAX);
    fade.advance(20_000_000);
    assert_eq!(
        fade.current().unwrap().opacity,
        (255u32.wrapping_mul(20_000_000) / u32::MAX) as i32
    );
    fade.advance(u32::MAX);
    assert_eq!(fade.current().unwrap().elapsed, 19_999_999);
    let mut code = vec![];
    command(&mut code, 0x2224, &[0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    assert_eq!(
        fade.try_wait(&mut vm, SkipGate::default(), &mut ActionInput::default())
            .unwrap(),
        Some(Step::Continue)
    );
}

/// Runs one 0x2224 with the given skip argument against a fresh 100 ms fade.
fn skip_wait(
    direction: u32,
    argument: u32,
    gate: SkipGate,
    input: &mut ActionInput,
) -> (Option<Step>, FadeHost, u32, usize, Option<u32>) {
    let mut code = vec![];
    command(&mut code, 0x2224, &[argument]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    let mut fade = create(direction, 100);
    let step = fade.try_wait(&mut vm, gate, input).unwrap();
    let pc = vm.pc() as u32;
    (
        step,
        fade,
        pc,
        vm.stack().len_bytes(),
        vm.state().system_integer(0),
    )
}

#[test]
fn skippable_wait_ends_at_once_without_effects_held_skip_actions_or_skip_flag() {
    let off = SkipGate {
        effects: false,
        ..SkipGate::default()
    };
    let flag = SkipGate {
        skip: true,
        ..SkipGate::default()
    };
    let mut held7 = ActionInput::default();
    held7.set(7, true, false, false).unwrap();
    let mut held8 = ActionInput::default();
    held8.set(8, true, false, false).unwrap();
    for (gate, mut input) in [
        (off, ActionInput::default()),
        (flag, ActionInput::default()),
        (SkipGate::default(), held7),
        (SkipGate::default(), held8),
    ] {
        for direction in [0, 1] {
            let (step, fade, pc, stack, result) = skip_wait(direction, 1, gate, &mut input);
            // 0xc004: yield after advancing PC and popping the argument.
            assert_eq!(
                (step, pc, stack, result),
                (Some(Step::Yield), 10, 0, Some(1))
            );
            // Forced like 0x2222: direction 0 keeps an opaque overlay, direction 1 removes it.
            match direction {
                0 => assert_eq!(
                    fade.current().map(|f| (f.finished, f.opacity)),
                    Some((true, 255))
                ),
                _ => assert!(fade.current().is_none()),
            }
        }
    }
}

#[test]
fn released_decide_action_is_consumed_only_by_a_skippable_wait() {
    let mut input = ActionInput::default();
    input.set(0, true, true, true).unwrap();
    // A zero argument ignores input entirely.
    let (step, fade, _, stack, result) = skip_wait(1, 0, SkipGate::default(), &mut input);
    assert_eq!((step, stack, result), (Some(Step::Yield), 4, Some(0)));
    assert!(fade.current().is_some());
    assert!(input.released(0));
    let (step, fade, _, stack, result) = skip_wait(1, 1, SkipGate::default(), &mut input);
    assert_eq!((step, stack, result), (Some(Step::Yield), 0, Some(1)));
    assert!(fade.current().is_none());
    // 0x140054d90 clears the release and press flags but keeps the held flag.
    assert!(!input.released(0) && !input.pressed(0) && input.held(0));
    // Other actions do not end the wait.
    let mut other = ActionInput::default();
    other.set(1, true, true, true).unwrap();
    let (step, _, _, stack, _) = skip_wait(1, 1, SkipGate::default(), &mut other);
    assert_eq!((step, stack), (Some(Step::Yield), 4));
    assert!(other.released(1));
    assert!(ActionInput::default().set(24, true, true, true).is_err());
}

#[test]
fn effect_use_is_set_and_read_by_0x22d1() {
    let mut code = vec![];
    // Read (SP-4 = 0, SP-8 unread), clear, read, set with a nonzero value, read.
    for (value, set) in [
        (None, 0u32),
        (Some(0u32), 1),
        (None, 0),
        (Some(7), 1),
        (None, 0),
    ] {
        if let Some(value) = value {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        } else {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(99u32.to_le_bytes());
        }
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(set.to_le_bytes());
        code.extend(0x22d1u16.to_le_bytes());
    }
    code.extend(0x22d1u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut effects = true;
    let mut results = vec![];
    for _ in 0..5 {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        vm.state_mut().set_system_integer(0, 5).unwrap();
        assert!(fade::try_handle_effects(&mut vm, &mut effects).unwrap());
        results.push((vm.state().system_integer(0), effects));
    }
    assert_eq!(
        results,
        [
            (Some(1), true),
            (Some(5), false),
            (Some(0), false),
            (Some(5), true),
            (Some(1), true),
        ]
    );
    assert_eq!(vm.stack().len_bytes(), 0);
    // An empty stack keeps the command pending.
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    assert!(fade::try_handle_effects(&mut vm, &mut effects).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x2d1));
}

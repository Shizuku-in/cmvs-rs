use cmvs_rs::{
    camera::CameraHost,
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
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn ending_without_a_running_move_only_pops_the_argument() {
    for flag in [0u32, 1] {
        let mut code = vec![];
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(flag.to_le_bytes());
        code.extend(0x2068u16.to_le_bytes());
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 4).unwrap();
        let mut camera = CameraHost::default();
        pending(&mut vm);
        assert!(camera.try_handle(&mut vm).unwrap());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
        assert_eq!(vm.state().system_integer(0), Some(4));
        assert!(!camera.moving());
    }
}

#[test]
fn a_missing_argument_keeps_the_command_pending() {
    let script = script(&0x2068u16.to_le_bytes());
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    assert!(CameraHost::default().try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x68));
}

/// Pushes `args` so the first one ends up at SP-4, then issues `op`.
fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
fn f(value: f32) -> u32 {
    value.to_bits()
}

#[test]
fn constructor_values_match_0x14004fab0() {
    let host = CameraHost::default();
    let camera = host.camera(1).unwrap();
    assert_eq!(camera.words[0], 1);
    assert_eq!(
        [2, 6, 7, 8, 9, 10, 11, 15, 16, 17, 18].map(|i| camera.float(i)),
        [
            3.0, 10.0, 10.0, 6.0, 1024.0, 612.0, 24.0, 512.0, 306.0, 1.0, 1.0
        ]
    );
    assert_eq!(camera.words[21], 0);
    assert!(host.camera(2).is_none());
}

#[test]
fn indexed_and_camera0_setters_write_their_fields() {
    let mut code = vec![];
    call(&mut code, 0x205a, &[1, f(1.239), f(36.0), f(-0.555)]);
    call(&mut code, 0x2058, &[1, f(1.0), f(2.0), f(3.0)]);
    call(&mut code, 0x2059, &[1, f(800.0), f(600.0)]);
    call(&mut code, 0x205b, &[1, f(-370.25)]);
    call(&mut code, 0x205c, &[1, f(30.0)]);
    call(&mut code, 0x205d, &[1, f(7.0), f(8.0)]);
    call(&mut code, 0x205e, &[1, 5]);
    call(&mut code, 0x205f, &[1, 2]);
    call(&mut code, 0x2066, &[1, 9]);
    call(&mut code, 0x2067, &[1, f(2.0), f(0.5)]);
    call(&mut code, 0x2062, &[f(4.0), f(5.0), f(6.0)]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = CameraHost::default();
    for _ in 0..11 {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm).unwrap());
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    let c = host.camera(1).unwrap();
    // Hundredths truncate toward zero.
    assert_eq!([1, 2, 3].map(|i| c.float(i)), [1.23, 36.0, -0.55]);
    assert_eq!([7, 8, 6].map(|i| c.float(i)), [1.0, 2.0, 3.0]);
    assert_eq!([9, 10].map(|i| c.float(i)), [800.0, 600.0]);
    // 0x205d overwrote the halves 0x2059 stored.
    assert_eq!([15, 16].map(|i| c.float(i)), [7.0, 8.0]);
    assert_eq!(c.float(14), 10.25);
    assert_eq!(c.float(11), 30.0);
    assert_eq!((c.words[37], c.words[0], c.words[39]), (5, 2, 9));
    assert_eq!([17, 18].map(|i| c.float(i)), [2.0, 0.5]);
    assert_eq!(
        [1, 2, 3].map(|i| host.camera(0).unwrap().float(i)),
        [4.0, 5.0, 6.0]
    );
}

#[test]
fn bad_indices_and_short_stacks_keep_the_command_pending() {
    let mut short = vec![];
    call(&mut short, 0x205a, &[0, f(1.0)]);
    let mut wide = vec![];
    call(&mut wide, 0x205c, &[2, f(1.0)]);
    for code in [short, wide] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut host = CameraHost::default();
        let bytes = vm.stack().len_bytes();
        assert!(host.try_handle(&mut vm).is_err());
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(host.camera(0), Some(&Default::default()));
    }
}

#[test]
fn integer_at_152_is_set_by_0x209c() {
    let mut code = vec![];
    call(&mut code, 0x209c, &[1, 7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = CameraHost::default();
    pending(&mut vm);
    assert!(host.try_handle(&mut vm).unwrap());
    assert_eq!(host.camera(1).unwrap().words[38], 7);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn projection_query_without_perspective_truncates_xyz_and_preserves_other_results() {
    for camera_mode in [0, 1, 2, 4, u32::MAX] {
        for point in [
            [f(12.75), f(-9.75), f(-0.5)],
            [f(f32::NAN), f(f32::INFINITY), f(2147483648.0)],
        ] {
            let mut code = vec![];
            call(&mut code, 0x205f, &[1, camera_mode]);
            // A caller's preceding stack word must survive the seven-argument query.
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(0x12345678u32.to_le_bytes());
            call(
                &mut code,
                0x2056,
                &[1, 99, point[0], point[1], point[2], f(8.0), f(0.0)],
            );
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            let mut host = CameraHost::default();
            pending(&mut vm);
            host.try_handle(&mut vm).unwrap();
            let before = host.camera(1).unwrap().clone();
            for i in 0..7 {
                vm.state_mut().set_system_integer(i, 100 + i).unwrap();
            }
            pending(&mut vm);
            assert!(host.try_handle(&mut vm).unwrap());
            let expected = if point[0] == f(12.75) {
                [12, -9, 0]
            } else {
                [i32::MIN; 3]
            };
            assert_eq!(
                [1, 2, 3].map(|i| vm.state().system_integer(i)),
                expected.map(|v| Some(v as u32))
            );
            for i in [0, 4, 5, 6] {
                assert_eq!(vm.state().system_integer(i), Some(100 + i));
            }
            assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
            assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 4));
            assert_eq!(host.camera(1), Some(&before));
            assert!(vm.pending_system().is_none());
        }
    }
}

#[test]
fn perspective_query_uses_selected_camera_position_scale_and_reference_depth() {
    let mut code = vec![];
    for (op, args) in [
        (0x205f, vec![1, 3]),
        (0x2058, vec![1, f(8.0), f(6.0), f(10.0)]),
        (0x2059, vec![1, f(800.0), f(600.0)]),
        (0x205a, vec![1, f(2.0), f(3.0), f(4.0)]),
        (0x205c, vec![1, f(1.5)]),
        (0x2067, vec![1, f(2.0), f(0.5)]),
    ] {
        call(&mut code, op, &args);
    }
    for mode in [0, 1, 2] {
        call(
            &mut code,
            0x2056,
            &[1, mode, f(10.0), f(-1.0), f(24.0), f(2.0), f(40.0)],
        );
    }
    // Zero relative depth produces nonfinite XY; cvttss2si returns i32::MIN, not 0.
    call(
        &mut code,
        0x2056,
        &[1, 0, f(10.0), f(-1.0), f(4.0), f(2.0), f(40.0)],
    );
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = CameraHost::default();
    for _ in 0..6 {
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
    }
    let before = host.camera(1).unwrap().clone();
    for _ in 0..3 {
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        // Relative depth 20, ratio 2, horizontal/vertical scale 25/100.
        assert_eq!(
            [1, 2, 3].map(|i| vm.state().system_integer(i)),
            [Some(800), Some(1330), Some(0)]
        );
        assert_eq!(host.camera(0), Some(&Default::default()));
        assert_eq!(host.camera(1), Some(&before));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    pending(&mut vm);
    host.try_handle(&mut vm).unwrap();
    assert_eq!(
        [1, 2, 3].map(|i| vm.state().system_integer(i)),
        [Some(i32::MIN as u32), Some(i32::MIN as u32), Some(0)]
    );
    assert_eq!(vm.pc(), code.len());
}

#[test]
fn projection_query_errors_keep_camera_stack_results_and_pending_command() {
    for args in [
        vec![2, 0, 0, 0, 0, 0, 0],
        vec![u32::MAX, 0, 0, 0, 0, 0, 0],
        vec![0, 0, 0, 0, 0, 0],
        vec![],
    ] {
        let mut code = vec![];
        call(&mut code, 0x2056, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 100 + i).unwrap();
        }
        pending(&mut vm);
        let before: Vec<_> = (1..=args.len())
            .map(|i| vm.stack().peek_u32(i * 4).unwrap())
            .collect();
        for _ in 0..2 {
            assert!(host.try_handle(&mut vm).is_err());
            assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x56)));
            assert_eq!(vm.pc(), code.len() - 2);
            assert_eq!(vm.stack().len_bytes(), before.len() * 4);
            for (i, value) in before.iter().enumerate() {
                assert_eq!(vm.stack().peek_u32((i + 1) * 4).unwrap(), *value);
            }
            for i in 0..7 {
                assert_eq!(vm.state().system_integer(i), Some(100 + i));
            }
            for i in 0..2 {
                assert_eq!(host.camera(i), Some(&Default::default()));
            }
        }
    }
}

#[test]
fn camera_move_query_has_no_parameters_and_preserves_stack_cameras_and_other_results() {
    for stack_words in [vec![], vec![0x12345678, u32::MAX, 0]] {
        let mut code = vec![];
        for &word in &stack_words {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(word.to_le_bytes());
        }
        code.extend(0x206au16.to_le_bytes());
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        assert!(!host.try_handle(&mut vm).unwrap());
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 100 + i).unwrap();
        }
        pending(&mut vm);
        assert!(host.try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(0));
        for i in 1..7 {
            assert_eq!(vm.state().system_integer(i), Some(100 + i));
        }
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes()),
            (code.len(), stack_words.len() * 4)
        );
        for (i, word) in stack_words.iter().rev().enumerate() {
            assert_eq!(vm.stack().peek_u32((i + 1) * 4).unwrap(), *word);
        }
        for i in 0..2 {
            assert_eq!(host.camera(i), Some(&Default::default()));
        }
        assert!(vm.pending_system().is_none());
    }
}

fn move_xyz(host: &CameraHost) -> [f32; 3] {
    [1, 2, 3].map(|i| host.camera(0).unwrap().float(i))
}
#[test]
fn camera_motion_axes_quantization_queries_and_unclamped_time_match_original() {
    for (opcode, mode, delta, target, expected) in [
        (
            0x206c,
            0,
            300,
            vec![f(10.0), f(23.0), f(-10.0)],
            [7.5, 18.0, -7.5],
        ),
        (
            0x206c,
            1,
            200,
            vec![f(10.0), f(23.0), f(-10.0)],
            [7.5, 18.0, -7.5],
        ),
        (0x206b, 0, 300, vec![f(10.0), f(-10.0)], [7.5, 3.0, -7.5]),
        (0x206d, 0, 300, vec![f(23.0)], [0.0, 18.0, 0.0]),
    ] {
        let mut code = vec![];
        let mut args = vec![mode];
        args.extend(&target);
        args.push(400);
        call(&mut code, opcode, &args);
        call(&mut code, 0x206a, &[]);
        call(&mut code, 0x206a, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        vm.state_mut().set_system_integer(1, 123).unwrap();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert!(host.moving());
        assert_eq!(move_xyz(&host), [0.0, 3.0, 0.0], "starting does not move");
        host.advance_frame(delta);
        assert_eq!(move_xyz(&host), expected);
        let before = host.camera(0).unwrap().clone();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1));
        assert_eq!(host.camera(0), Some(&before));
        host.advance_frame(400 - delta);
        assert!(!host.moving());
        assert_eq!(
            [23, 24, 25].map(|i| host.camera(0).unwrap().float(i)),
            move_xyz(&host)
        );
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.state().system_integer(1), Some(123));
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
        assert_eq!(host.camera(1), Some(&Default::default()));
    }
}

#[test]
fn camera_motion_restarts_from_current_position_and_stop_selects_cancel_or_finish() {
    for finish in [0, 1] {
        let mut code = vec![];
        call(&mut code, 0x206c, &[0, f(10.0), f(13.0), f(10.0), 400]);
        call(&mut code, 0x206c, &[0, f(20.129), f(23.0), f(20.0), 400]);
        call(&mut code, 0x2068, &[finish]);
        call(&mut code, 0x206d, &[1, f(1.239), 0]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        host.advance_frame(200);
        assert_eq!(move_xyz(&host), [5.0, 8.0, 5.0]);
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        let camera = host.camera(0).unwrap();
        assert_eq!([30, 31, 32].map(|i| camera.float(i)), [5.0, 8.0, 5.0]);
        assert_eq!(camera.float(23), 20.12);
        host.advance_frame(100);
        let current = move_xyz(&host);
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        let expected = if finish == 0 {
            current
        } else {
            [20.12, 23.0, 20.0]
        };
        assert_eq!(move_xyz(&host), expected);
        assert!(!host.moving());
        host.advance_frame(1000);
        assert_eq!(move_xyz(&host), expected);
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert!(host.moving());
        assert_eq!(
            move_xyz(&host),
            expected,
            "zero duration still starts active"
        );
        host.advance_frame(0);
        assert!(!host.moving());
        assert_eq!(move_xyz(&host), [expected[0], 1.23, expected[2]]);
    }
}

#[test]
fn projection_during_camera_motion_distinguishes_current_and_target_positions() {
    let mut code = vec![];
    for (opcode, args) in [
        (0x205f, vec![0, 3]),
        (0x2058, vec![0, f(8.0), f(6.0), f(10.0)]),
        (0x2059, vec![0, f(800.0), f(600.0)]),
        (0x205a, vec![0, f(2.0), f(3.0), f(4.0)]),
        (0x205c, vec![0, f(0.0)]),
        (0x206c, vec![0, f(6.0), f(5.0), f(8.0), 400]),
    ] {
        call(&mut code, opcode, &args);
    }
    for mode in [0, 1] {
        call(
            &mut code,
            0x2056,
            &[0, mode, f(10.0), f(-1.0), f(24.0), f(2.0), f(40.0)],
        );
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = CameraHost::default();
    for _ in 0..6 {
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
    }
    host.advance_frame(200);
    assert_eq!(move_xyz(&host), [4.0, 4.0, 6.0]);
    for expected in [[1140, 1028, 0], [1025, 1362, 0]] {
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert_eq!(
            [1, 2, 3].map(|i| vm.state().system_integer(i)),
            expected.map(Some)
        );
    }
}

#[test]
fn camera_motion_wait_natural_finish_and_skip_follow_input_short_circuit_rules() {
    use cmvs_rs::{fade::SkipGate, input::ActionInput};
    for trigger in 0..6 {
        let mut code = vec![];
        call(&mut code, 0x206c, &[1, f(10.0), f(20.0), f(30.0), 400]);
        call(&mut code, 0x2069, &[u32::from(trigger != 5)]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        let mut input = ActionInput::default();
        input.set(0, true, true, true).unwrap();
        if trigger == 2 || trigger == 3 {
            input.set(7 + trigger - 2, true, true, true).unwrap();
        }
        let gate = SkipGate {
            effects: trigger != 0,
            skip: trigger == 1,
        };
        let mut expected_input = input.clone();
        if trigger == 4 {
            expected_input.consume(0);
        }
        assert_eq!(
            host.try_wait_move(&mut vm, gate, &mut input).unwrap(),
            Some(Step::Yield)
        );
        assert_eq!(input, expected_input);
        if trigger == 5 {
            assert_eq!(
                (vm.pc(), vm.stack().len_bytes(), vm.repeat_count()),
                (pc, 4, 1)
            );
            assert_eq!(vm.state().system_integer(0), Some(0));
            assert_eq!(move_xyz(&host), [0.0, 3.0, 0.0]);
            host.advance_frame(400);
            assert_eq!(
                host.try_wait_move(&mut vm, gate, &mut input).unwrap(),
                Some(Step::Continue)
            );
            assert_eq!(vm.state().system_integer(0), Some(0));
        } else {
            assert!(vm.pending_system().is_none());
            assert_eq!(vm.state().system_integer(0), Some(1));
        }
        assert_eq!(move_xyz(&host), [10.0, 20.0, 30.0]);
        assert!(!host.moving());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
        assert_eq!(host.try_wait_move(&mut vm, gate, &mut input).unwrap(), None);
    }
}

#[test]
fn camera_motion_bad_mode_or_short_parameters_preserve_active_move_stack_and_results() {
    use cmvs_rs::{fade::SkipGate, input::ActionInput};
    for (opcode, args) in [
        (0x206b, vec![2, f(1.0), f(2.0), 100]),
        (0x206c, vec![u32::MAX, 0, 0, 0, 100]),
        (0x206d, vec![2, 0, 100]),
        (0x206b, vec![0, 0]),
        (0x206c, vec![0, 0, 0, 0]),
        (0x206d, vec![0]),
        (0x2068, vec![]),
        (0x2069, vec![]),
    ] {
        let mut code = vec![];
        call(&mut code, 0x206c, &[0, f(10.0), f(20.0), f(30.0), 400]);
        call(&mut code, opcode, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = CameraHost::default();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        host.advance_frame(100);
        let expected = host.camera(0).unwrap().clone();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 123).unwrap();
        let mut input = ActionInput::default();
        input.set(0, true, true, true).unwrap();
        let before_input = input.clone();
        for _ in 0..2 {
            if opcode == 0x2069 {
                assert!(
                    host.try_wait_move(&mut vm, SkipGate::default(), &mut input)
                        .is_err()
                );
            } else {
                assert!(host.try_handle(&mut vm).is_err());
            }
            assert_eq!(host.camera(0), Some(&expected));
            assert_eq!(input, before_input);
            assert_eq!(vm.state().system_integer(0), Some(123));
            assert_eq!(vm.pending_system(), Some((code.len() - 2, opcode - 0x2000)));
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        }
    }
}

mod support;
use cmvs_rs::{
    machine::Machine,
    scene::{RotationTrack, SceneHost},
};
use support::bytecode::{command, pending, script};
fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, count: usize) {
    for _ in 0..count {
        pending(vm);
        assert!(scene.try_handle(vm).unwrap());
    }
}
fn setup(code: &mut Vec<u8>, base: i32) {
    command(code, 0x2020, &[20]);
    command(code, 0x2040, &[20]);
    command(code, 0x21e5, &[20, u32::MAX, base as u32]);
}
fn angle(scene: &SceneHost) -> i32 {
    scene.object(20).unwrap().draw.as_ref().unwrap().words[20] as i32
}
fn track(scene: &SceneHost) -> Option<&RotationTrack> {
    scene.object(20)?.draw.as_ref()?.rotation_track.as_deref()
}
#[test]
fn rotation_directions_wrap_ease_and_equal_angles_make_a_full_turn() {
    for (base, target, mode, expected) in [
        (350, 10, 0, 0),
        (10, 350, 0x8000, 0),
        (0, 90, 0, 4500),
        (0, 90, 1, 6363),
        (90, 0, 0x8001, 2637),
        (30, 30, 0, 21000),
    ] {
        let mut code = vec![];
        setup(&mut code, base);
        command(
            &mut code,
            0x21e6,
            &[20, u32::MAX, 0, u32::MAX, mode, 400, target],
        );
        command(&mut code, 0x21e7, &[20, u32::MAX, 0]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 5);
        assert_eq!(angle(&scene), 0);
        scene.advance_frame(900);
        assert_eq!(angle(&scene), expected, "mode {mode}");
        scene.advance_frame(200);
        assert_eq!(angle(&scene), (target as i32 % 360) * 100);
        assert!(track(&scene).is_none());
    }
}
#[test]
fn sine_period_repeats_and_stop_snaps_to_target() {
    let mut code = vec![];
    setup(&mut code, -370);
    command(&mut code, 0x21e9, &[20, u32::MAX]);
    command(
        &mut code,
        0x21e6,
        &[20, u32::MAX, 7, u32::MAX, 0x80, 400, 30],
    );
    command(&mut code, 0x21e7, &[20, u32::MAX, 7]);
    command(&mut code, 0x21e9, &[20, u32::MAX]);
    command(&mut code, 0x21e8, &[20, u32::MAX]);
    command(&mut code, 0x21e9, &[20, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 4);
    assert_eq!(track(&scene).unwrap().base, -1000);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    run(&mut vm, &mut scene, 3);
    assert_eq!(vm.state().system_integer(0), Some(7));
    for expected in [2000, 35000, 32000, 35000, 2000] {
        scene.advance_frame(100);
        assert_eq!(angle(&scene), expected);
    }
    run(&mut vm, &mut scene, 2);
    assert_eq!(angle(&scene), 3000);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}
#[test]
fn invalid_keys_modes_and_zero_sine_period_keep_pending_command_and_scene() {
    for (key, next, mode, duration) in [
        (8, u32::MAX, 0, 400),
        (0, 8, 0, 400),
        (0, u32::MAX, 2, 400),
        (0, u32::MAX, 0x80, 0),
    ] {
        let mut code = vec![];
        setup(&mut code, 0);
        command(
            &mut code,
            0x21e6,
            &[20, u32::MAX, key, next, mode, duration, 90],
        );
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 3);
        pending(&mut vm);
        let before = scene.object(20).unwrap().clone();
        let pc = vm.pc();
        let sp = vm.stack().len_bytes();
        assert!(scene.try_handle(&mut vm).is_err());
        assert_eq!(scene.object(20).unwrap(), &before);
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc, sp));
        assert!(vm.pending_system().is_some());
    }
}

#[test]
fn child_rotation_minus_two_disposes_only_the_child_and_zero_cycles_are_bounded() {
    let mut code = vec![];
    setup(&mut code, 0);
    for (op, args) in [
        (0x2022, vec![20, 0]),
        (0x2050, vec![20, 0]),
        (0x21e5, vec![20, 0, 0]),
        (0x21e6, vec![20, 0, 0, (-2i32) as u32, 0, 1, 90]),
        (0x21e7, vec![20, 0, 0]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 8);
    scene.advance_frame(1);
    assert!(scene.object(20).unwrap().children.is_empty());
    assert!(track(&scene).is_some());
    let mut code = vec![];
    setup(&mut code, 0);
    command(&mut code, 0x21e6, &[20, u32::MAX, 0, 0, 0, 0, 90]);
    command(&mut code, 0x21e7, &[20, u32::MAX, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 5);
    scene.advance_frame(1000);
    assert_eq!(angle(&scene), 9000);
    assert!(track(&scene).unwrap().playing);
}

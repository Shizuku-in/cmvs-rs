#[path = "support/shared_state.rs"]
mod shared_state;
use shared_state::SharedState;
mod support;
use cmvs_rs::{
    machine::Machine,
    scene::{PositionKey, SceneHost},
};
use support::bytecode::{command, pending, script};
fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, n: usize) {
    for _ in 0..n {
        pending(vm);
        assert!(scene.shared_state(vm).unwrap());
    }
}
fn setup(code: &mut Vec<u8>, base: [u32; 2]) {
    command(code, 0x2020, &[20]);
    command(code, 0x2040, &[20]);
    command(code, 0x2045, &[20, u32::MAX, base[0], base[1]]);
    command(code, 0x21f4, &[20, u32::MAX]);
}
fn xy(scene: &SceneHost) -> [i32; 2] {
    let w = &scene.object(20).unwrap().draw.as_ref().unwrap().words;
    [w[8] as i32, w[9] as i32]
}
#[test]
fn position_motion_constructor_query_child_isolation_stop_and_reset() {
    let mut code = vec![];
    setup(&mut code, [10, 20]);
    for (op, args) in [
        (0x2022, vec![20, 0]),
        (0x2050, vec![20, 0]),
        (0x21f4, vec![20, 0]),
        (0x21f8, vec![20, 0]),
        (0x21f5, vec![20, 0, 15, u32::MAX, 0, 400, 0x8001, 0x7fff]),
        (0x21f6, vec![20, 0, 15]),
        (0x21f8, vec![20, 0]),
        (0x21f7, vec![20, 0]),
        (0x21f8, vec![20, 0]),
        (0x21f4, vec![20, u32::MAX]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 4);
    let motion = scene
        .object(20)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .position_motion
        .as_ref()
        .unwrap();
    assert!(motion.playback.is_none());
    assert_eq!(motion.keys, [PositionKey::default(); 16]);
    run(&mut vm, &mut scene, 4);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    run(&mut vm, &mut scene, 3);
    assert_eq!(vm.state().system_integer(0), Some(15));
    run(&mut vm, &mut scene, 1);
    let child = scene.object(20).unwrap().children[&0]
        .draw
        .as_ref()
        .unwrap();
    assert_eq!(&child.words[8..10], &[(-32767i32) as u32, 32767]);
    assert!(child.position_motion.is_none());
    assert_eq!(xy(&scene), [10, 20]);
    run(&mut vm, &mut scene, 2);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}
#[test]
fn position_motion_interpolation_preserves_integer_rules_and_terminal_target() {
    for (mode, half) in [
        (0, [20, 0]),
        (0x8000, [25, 10]),
        (1, [24, -8]),
        (0x000a0020, [10, 30]),
        (0x070a0021, [17, 30]),
    ] {
        let mut code = vec![];
        setup(&mut code, [10, 20]);
        command(
            &mut code,
            0x21f5,
            &[20, u32::MAX, 0, u32::MAX, mode, 400, 30, (-20i32) as u32],
        );
        command(&mut code, 0x21f6, &[20, u32::MAX, 0]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 6);
        assert_eq!(xy(&scene), [10, 20]);
        scene.advance_frame(900);
        assert_eq!(xy(&scene), half, "mode={mode:x}");
        scene.advance_frame(200);
        assert_eq!(xy(&scene), [30, -20]);
        assert!(
            scene
                .object(20)
                .unwrap()
                .draw
                .as_ref()
                .unwrap()
                .position_motion
                .is_none()
        );
    }
}
#[test]
fn position_motion_short_base_key_boundary_delay_and_minus_two_disposal() {
    let mut code = vec![];
    setup(&mut code, [70000, (-70000i32) as u32]);
    command(
        &mut code,
        0x21f5,
        &[20, u32::MAX, 0, 1, 0, 400, 100, (-100i32) as u32],
    );
    command(
        &mut code,
        0x21f5,
        &[
            20,
            u32::MAX,
            1,
            (-2i32) as u32,
            0,
            200,
            300,
            (-300i32) as u32,
        ],
    );
    command(&mut code, 0x21f6, &[20, u32::MAX, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 7);
    assert_eq!(xy(&scene), [70000, -70000]);
    scene.advance_frame(900);
    assert_eq!(xy(&scene), [2282, -2282]);
    scene.advance_frame(200);
    assert_eq!(xy(&scene), [100, -100]);
    scene.advance_frame(50);
    assert_eq!(xy(&scene), [150, -150]);
    scene.advance_frame(150);
    assert!(scene.object(20).is_none());
}
#[test]
fn position_motion_invalid_arguments_are_transactional_and_absent_track_is_noop() {
    for (op, args) in [
        (0x21f4, vec![21, u32::MAX]),
        (0x21f4, vec![20, 768]),
        (0x21f4, vec![20, 0]),
        (0x21f5, vec![20, u32::MAX, 16, 0, 0, 100, 0, 0]),
        (0x21f5, vec![20, u32::MAX, 0, 16, 0, 100, 0, 0]),
        (0x21f5, vec![20, u32::MAX, 0, 0, 0x8001, 100, 0, 0]),
        (0x21f5, vec![20, u32::MAX, 0, 0, 0, u32::MAX, 0, 0]),
        (0x21f6, vec![20, u32::MAX, 16]),
        (0x21f4, vec![20]),
        (0x21f5, vec![20, u32::MAX, 0]),
        (0x21f6, vec![20, u32::MAX]),
    ] {
        let mut code = vec![];
        setup(&mut code, [10, 20]);
        command(&mut code, op, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 4);
        pending(&mut vm);
        let before = scene.object(20).unwrap().clone();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        for _ in 0..2 {
            assert!(scene.shared_state(&mut vm).is_err());
            assert_eq!(scene.object(20), Some(&before));
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            assert_eq!(vm.pending_system(), Some((code.len() - 2, op - 0x2000)));
            assert_eq!(vm.state().system_integer(0), Some(77));
        }
    }
    let mut code = vec![];
    command(&mut code, 0x2020, &[20]);
    command(&mut code, 0x2040, &[20]);
    command(
        &mut code,
        0x21f5,
        &[20, u32::MAX, u32::MAX, u32::MAX, u32::MAX, u32::MAX, 0, 0],
    );
    command(&mut code, 0x21f6, &[20, u32::MAX, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 4);
    assert!(
        scene
            .object(20)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .position_motion
            .is_none()
    );
}

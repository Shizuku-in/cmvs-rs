#[path = "support/shared_state.rs"]
mod shared_state;
use shared_state::SharedState;
mod support;
use cmvs_rs::{
    machine::Machine,
    scene::{ScaleKey, SceneHost},
};
use support::bytecode::{command, pending, script};
fn f(v: f32) -> u32 {
    v.to_bits()
}
fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, count: usize) {
    for _ in 0..count {
        pending(vm);
        assert!(scene.shared_state(vm).unwrap());
    }
}
fn setup(code: &mut Vec<u8>, base: [u32; 2]) {
    command(code, 0x2020, &[20]);
    command(code, 0x2040, &[20]);
    command(code, 0x21e0, &[20, u32::MAX, base[0], base[1]]);
}
fn xy(scene: &SceneHost) -> [u32; 2] {
    scene.object(20).unwrap().draw.as_ref().unwrap().words[18..20]
        .try_into()
        .unwrap()
}
#[test]
fn scale_constructor_child_queries_reset_stop_and_raw_targets() {
    let mut code = vec![];
    setup(&mut code, [f(2.0), f(3.0)]);
    for (op, args) in [
        (0x2022, vec![20, 0]),
        (0x2050, vec![20, 0]),
        (0x21e0, vec![20, 0, f(0.0), f(-0.0)]),
        (0x21e4, vec![20, 0]),
        (0x21e1, vec![20, 0, 7, u32::MAX, 0, 400, f(4.0), f(-0.0)]),
        (0x21e2, vec![20, 0, 7]),
        (0x21e4, vec![20, 0]),
        (0x21e3, vec![20, 0]),
        (0x21e4, vec![20, 0]),
        (0x21e0, vec![20, u32::MAX, f(5.0), f(6.0)]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 3);
    let m = scene
        .object(20)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .scale_motion
        .as_ref()
        .unwrap();
    assert_eq!(m.base, [f(2.0), f(3.0)]);
    assert!(m.playback.is_none());
    assert_eq!(m.keys, [ScaleKey::default(); 8]);
    assert_eq!(xy(&scene), [f(1.0); 2]);
    run(&mut vm, &mut scene, 4);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    run(&mut vm, &mut scene, 3);
    assert_eq!(vm.state().system_integer(0), Some(7));
    run(&mut vm, &mut scene, 1);
    let child = scene.object(20).unwrap().children[&0]
        .draw
        .as_ref()
        .unwrap();
    assert_eq!(&child.words[18..20], &[f(4.0), f(-0.0)]);
    assert!(child.scale_motion.is_none());
    assert_eq!(xy(&scene), [f(1.0); 2]);
    run(&mut vm, &mut scene, 2);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}
#[test]
fn scale_linear_is_unquantized_and_sine_quantizes_only_the_offset() {
    for (mode, expected) in [(0, [f(1.125), f(-0.375)]), (1, [f(1.535), f(-0.575)])] {
        let mut code = vec![];
        setup(&mut code, [f(0.125), f(0.125)]);
        command(
            &mut code,
            0x21e1,
            &[20, u32::MAX, 0, u32::MAX, mode, 400, f(2.125), f(-0.875)],
        );
        command(&mut code, 0x21e2, &[20, u32::MAX, 0]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 5);
        assert_eq!(xy(&scene), [f(1.0); 2]);
        scene.advance_frame(900);
        assert_eq!(xy(&scene), expected);
        scene.advance_frame(200);
        assert_eq!(xy(&scene), [f(2.125), f(-0.875)]);
        assert!(
            scene
                .object(20)
                .unwrap()
                .draw
                .as_ref()
                .unwrap()
                .scale_motion
                .is_none()
        );
    }
}
#[test]
fn scale_boundary_defers_next_interpolation_restart_keeps_base_and_minus_two_disposes() {
    let mut code = vec![];
    setup(&mut code, [0, 0]);
    command(
        &mut code,
        0x21e1,
        &[20, u32::MAX, 0, 1, 0, 100, f(1.0), f(2.0)],
    );
    command(
        &mut code,
        0x21e1,
        &[20, u32::MAX, 1, (-2i32) as u32, 0, 200, f(3.0), f(4.0)],
    );
    command(&mut code, 0x21e2, &[20, u32::MAX, 0]);
    command(&mut code, 0x21e2, &[20, u32::MAX, 1]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 6);
    scene.advance_frame(150);
    assert_eq!(xy(&scene), [f(1.0), f(2.0)]);
    scene.advance_frame(50);
    assert_eq!(xy(&scene), [f(2.0), f(3.0)]);
    run(&mut vm, &mut scene, 1);
    scene.advance_frame(50);
    assert_eq!(
        xy(&scene),
        [f(1.5), f(2.5)],
        "restart uses track base from the crossed key, not current scale"
    );
    scene.advance_frame(150);
    assert!(scene.object(20).is_none());
}
#[test]
fn scale_invalid_parameters_preserve_pending_command_and_absent_track_is_noop() {
    for (op, args) in [
        (0x21e0, vec![21, u32::MAX, 0, 0]),
        (0x21e0, vec![20, 768, 0, 0]),
        (0x21e0, vec![20, 0, 0, 0]),
        (0x21e0, vec![20, u32::MAX, 0]),
        (0x21e1, vec![20, u32::MAX, 8, 0, 0, 1, 0, 0]),
        (0x21e1, vec![20, u32::MAX, 0, 8, 0, 1, 0, 0]),
        (0x21e1, vec![20, u32::MAX, 0, 0, 2, 1, 0, 0]),
        (0x21e2, vec![20, u32::MAX, 8]),
    ] {
        let mut code = vec![];
        setup(&mut code, [0, 0]);
        command(&mut code, op, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 3);
        pending(&mut vm);
        let old = scene.object(20).unwrap().clone();
        let before = vm.pending_system();
        assert!(scene.shared_state(&mut vm).is_err());
        assert_eq!(scene.object(20), Some(&old));
        assert_eq!(vm.pending_system(), before);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
    let mut code = vec![];
    command(&mut code, 0x2020, &[20]);
    command(&mut code, 0x2040, &[20]);
    command(
        &mut code,
        0x21e1,
        &[20, u32::MAX, u32::MAX, 90, 99, 0, 0, 0],
    );
    command(&mut code, 0x21e2, &[20, u32::MAX, u32::MAX]);
    command(&mut code, 0x21e3, &[20, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 5);
    assert_eq!(vm.stack().len_bytes(), 0);
}

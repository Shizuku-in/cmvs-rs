#[path = "support/shared_state.rs"]
mod shared_state;
use shared_state::SharedState;
mod support;
use cmvs_rs::{
    machine::Machine,
    scene::{AnimationFrame, SceneHost},
};
use support::bytecode::{command, pending, script};

fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, count: usize) {
    for _ in 0..count {
        pending(vm);
        assert!(scene.shared_state(vm).unwrap());
    }
}
fn setup(code: &mut Vec<u8>) {
    command(code, 0x2020, &[20]);
    command(code, 0x2040, &[20]);
}
fn words(scene: &SceneHost) -> &[u32] {
    &scene.object(20).unwrap().draw.as_ref().unwrap().words[2..8]
}

#[test]
fn scene_animation_child_lifecycle_queries_and_low_short_coordinates() {
    let mut code = vec![];
    setup(&mut code);
    for (op, args) in [
        (0x2022, vec![20, 0]),
        (0x2050, vec![20, 0]),
        (0x21ef, vec![20, 0]),
        (0x21f3, vec![20, 0]),
        (
            0x21f0,
            vec![20, 0, 63, u32::MAX, 400, 0x18001, 2, 30, 40, 5, 6],
        ),
        (0x21f1, vec![20, 0, 63]),
        (0x21f3, vec![20, 0]),
        (0x21ef, vec![20, 0]),
        (0x21f2, vec![20, 0]),
        (0x21f3, vec![20, 0]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 6);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX - 1));
    let child = &scene.object(20).unwrap().children[&0];
    assert_eq!(
        child
            .draw
            .as_ref()
            .unwrap()
            .animation
            .as_ref()
            .unwrap()
            .frames,
        [AnimationFrame::default(); 64]
    );
    run(&mut vm, &mut scene, 3);
    assert_eq!(vm.state().system_integer(0), Some(63));
    assert_eq!(
        &scene.object(20).unwrap().children[&0]
            .draw
            .as_ref()
            .unwrap()
            .words[2..8],
        &[(-32767i32) as u32, 2, 30, 40, 5, 6]
    );
    assert!(
        scene
            .object(20)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .animation
            .is_none()
    );
    run(&mut vm, &mut scene, 1);
    assert!(
        !scene.object(20).unwrap().children[&0]
            .draw
            .as_ref()
            .unwrap()
            .animation
            .as_ref()
            .unwrap()
            .playing
    );
    run(&mut vm, &mut scene, 2);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn scene_animation_stop_refreshes_current_key_and_keeps_negative_origins() {
    let mut code = vec![];
    setup(&mut code);
    command(&mut code, 0x21ef, &[20, u32::MAX]);
    command(
        &mut code,
        0x21f0,
        &[20, u32::MAX, 0, u32::MAX, 100, 1, 2, 30, 40, 5, 6],
    );
    command(&mut code, 0x21f1, &[20, u32::MAX, 0]);
    command(
        &mut code,
        0x21f0,
        &[20, u32::MAX, 0, u32::MAX, 100, 7, 8, 90, 100, u32::MAX, 12],
    );
    command(&mut code, 0x21f2, &[20, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 6);
    assert_eq!(words(&scene), &[1, 2, 30, 40, 5, 6]);
    run(&mut vm, &mut scene, 1);
    assert_eq!(words(&scene), &[7, 8, 90, 100, 5, 12]);
    assert!(
        scene
            .object(20)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .animation
            .is_none()
    );
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn scene_animation_advances_with_delta_cap_and_disposes_at_negative_two() {
    let mut code = vec![];
    setup(&mut code);
    command(&mut code, 0x21ef, &[20, u32::MAX]);
    command(
        &mut code,
        0x21f0,
        &[20, u32::MAX, 0, 1, 300, 1, 2, 30, 40, 5, 6],
    );
    command(
        &mut code,
        0x21f0,
        &[
            20,
            u32::MAX,
            1,
            (-2i32) as u32,
            100,
            7,
            8,
            90,
            100,
            u32::MAX,
            u32::MAX,
        ],
    );
    command(&mut code, 0x21f1, &[20, u32::MAX, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 6);
    scene.advance_frame(900);
    assert_eq!(words(&scene), &[1, 2, 30, 40, 5, 6]);
    scene.advance_frame(100);
    assert_eq!(words(&scene), &[7, 8, 90, 100, 5, 6]);
    scene.advance_frame(100);
    assert!(scene.object(20).is_none());
}

#[test]
fn scene_animation_invalid_indices_are_transactional_but_absent_track_is_noop() {
    for (op, args) in [
        (0x21f0, vec![20, u32::MAX, 64, 0, 1, 0, 0, 1, 1, 0, 0]),
        (0x21f0, vec![20, u32::MAX, 0, 64, 1, 0, 0, 1, 1, 0, 0]),
        (0x21f1, vec![20, u32::MAX, 64]),
        (0x21ef, vec![20, 768]),
        (0x21ef, vec![256, u32::MAX]),
    ] {
        let mut code = vec![];
        setup(&mut code);
        command(&mut code, 0x21ef, &[20, u32::MAX]);
        command(&mut code, op, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 3);
        pending(&mut vm);
        let old = scene.object(20).unwrap().clone();
        let pc = vm.pc();
        let stack = vm.stack().len_bytes();
        assert!(scene.shared_state(&mut vm).is_err());
        assert_eq!(scene.object(20), Some(&old));
        assert_eq!(vm.pc(), pc);
        assert_eq!(vm.stack().len_bytes(), stack);
    }
    let mut code = vec![];
    setup(&mut code);
    command(
        &mut code,
        0x21f0,
        &[20, u32::MAX, u32::MAX, 100, 0, 0, 0, 0, 0, 0, 0],
    );
    command(&mut code, 0x21f1, &[20, u32::MAX, u32::MAX]);
    command(&mut code, 0x21f2, &[20, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 5);
    assert_eq!(vm.stack().len_bytes(), 0);
}

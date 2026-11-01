#[allow(dead_code)]
#[path = "support/shared_controls.rs"]
mod shared_controls;
use cmvs_rs::{
    buttons::{Button, ButtonHost, ButtonImage},
    input::ActionInput,
    machine::{Machine, Step},
    ps3::Script,
    scene::SceneHost,
};
use shared_controls::SharedControls;

/// Arguments are listed from SP-4 outward.
fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
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
/// Runs to the next system command and offers it to the scene, then to the buttons.
fn next(vm: &mut Machine<'_>, scene: &mut SceneHost, buttons: &mut ButtonHost) -> bool {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    scene.try_handle(vm).unwrap() || buttons.shared_buttons(vm, scene).unwrap()
}
/// Scene object 10 plus button group 2 bound to it.
fn prelude(code: &mut Vec<u8>) {
    call(code, 0x2020, &[10]);
    call(code, 0x2210, &[2, 10]);
}
fn image(child: i32, base: i16) -> ButtonImage {
    ButtonImage {
        child,
        crop: [base, base + 1, base + 2, base + 3],
        origin: [base + 4, base + 5],
    }
}
fn set_image(code: &mut Vec<u8>, id: u32, state: u32, child: u32, base: u32) {
    let mut args = vec![2, id, state, child];
    args.extend((0..6).map(|i| base + i));
    call(code, 0x2214, &args);
}

#[test]
fn buttons_link_in_insertion_order_and_store_rects_and_state_images() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2212, &[2, 7]);
    call(&mut code, 0x2212, &[2, 9]);
    call(&mut code, 0x2213, &[2, 9, 100, 200, 300, u32::MAX]);
    for state in 0..4 {
        set_image(&mut code, 9, state, state + 1, 10 * state);
    }
    // Unknown ids and state 4 are silent no-ops in the original.
    call(&mut code, 0x2213, &[2, 8, 1, 1, 1, 1]);
    set_image(&mut code, 9, 4, 99, 0);
    // Crop and origin values are truncated to signed 16-bit words.
    set_image(&mut code, 7, 0, 5, 0xfffe);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 0).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..4 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    assert_eq!(vm.state().system_integer(0), Some(1));
    while vm.pc() < code.len() {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    let group = buttons.group(2).unwrap();
    assert_eq!(group.object, 10);
    let expected = Button {
        id: 9,
        enabled: true,
        rect: [100, 200, 300, -1],
        links: [7, -1, -1, -1],
        images: [image(1, 0), image(2, 10), image(3, 20), image(4, 30)],
    };
    assert_eq!(group.buttons[1], expected);
    assert_eq!(group.buttons[0].links, [-1, 9, -1, -1]);
    assert_eq!(group.buttons[0].rect, [0; 4]);
    assert_eq!(
        group.buttons[0].images[0],
        ButtonImage {
            child: 5,
            crop: [-2, -1, 0, 1],
            origin: [2, 3]
        }
    );
}

#[test]
fn merged_state_mode_from_0x2087_shifts_the_state_records() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2212, &[2, 1]);
    call(&mut code, 0x2087, &[5]);
    for state in 0..4 {
        set_image(&mut code, 1, state, state + 1, 10 * state);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    assert!(!buttons.merged_states());
    while vm.pc() < code.len() {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    assert!(buttons.merged_states());
    assert_eq!(
        buttons.group(2).unwrap().buttons[0].images,
        [image(1, 0), image(2, 10), image(2, 10), image(3, 20)]
    );
}

#[test]
fn recreating_or_destroying_a_group_drops_its_buttons() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2212, &[2, 1]);
    call(&mut code, 0x2210, &[2, 10]);
    call(&mut code, 0x2211, &[2]);
    call(&mut code, 0x2211, &[2]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..3 {
        next(&mut vm, &mut scene, &mut buttons);
    }
    next(&mut vm, &mut scene, &mut buttons);
    assert!(buttons.group(2).unwrap().buttons.is_empty());
    next(&mut vm, &mut scene, &mut buttons);
    assert!(buttons.group(2).is_none());
    next(&mut vm, &mut scene, &mut buttons);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn invalid_slots_missing_objects_and_short_stacks_keep_the_command_pending() {
    for (op, args) in [
        (0x2210, vec![6, 10]),
        (0x2210, vec![u32::MAX, 10]),
        (0x2210, vec![0, 11]), // empty scene slot
        (0x2210, vec![0, 256]),
        (0x2212, vec![3, 1]), // empty group
        (0x2213, vec![2, 1, 0, 0, 0]),
        (0x2214, vec![2, 1, 0, 0, 0, 0, 0, 0, 0]),
        (0x2211, vec![6]),
    ] {
        let mut code = vec![];
        prelude(&mut code);
        call(&mut code, op, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        next(&mut vm, &mut scene, &mut buttons);
        next(&mut vm, &mut scene, &mut buttons);
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        assert!(
            buttons.shared_buttons(&mut vm, &mut scene).is_err(),
            "{op:#x}"
        );
        assert_eq!(vm.pending_system(), Some((pc, op - 0x2000)));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(buttons.group(2).unwrap().buttons, vec![]);
        assert!(buttons.group(3).is_none() && buttons.group(0).is_none());
        assert_eq!(vm.state().system_integer(0), None);
    }
}

#[test]
fn unverified_commands_are_left_pending() {
    for op in [0x2218, 0x2219] {
        let mut code = vec![];
        prelude(&mut code);
        call(&mut code, op, &[2]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        next(&mut vm, &mut scene, &mut buttons);
        next(&mut vm, &mut scene, &mut buttons);
        assert!(!next(&mut vm, &mut scene, &mut buttons));
        assert_eq!(vm.pending_system().map(|p| p.1), Some(op - 0x2000));
    }
}

#[test]
fn show_applies_state_zero_crop_and_origin_to_the_child_and_marks_it_shown() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2022, &[10, 3]);
    call(&mut code, 0x2050, &[10, 3]);
    call(&mut code, 0x2212, &[2, 1]);
    set_image(&mut code, 1, 0, 3, 0xfffe);
    set_image(&mut code, 1, 3, 3, 40);
    call(&mut code, 0x2215, &[2]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    while vm.pc() < code.len() {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    let child = &scene.object(10).unwrap().children[&3];
    assert!(child.shown);
    // Sign-extended words from movsx, as the 0x2044/0x2046 setters receive them.
    let crop_origin = [-2, -1, 0, 1, 2, 3].map(|v: i32| v as u32);
    assert_eq!(&child.draw.as_ref().unwrap().words[2..8], &crop_origin);
}

#[test]
fn show_checks_every_target_before_writing() {
    // Button 1 targets a valid child; button 2 targets either a child without a draw
    // state or one that does not exist.
    for (missing_draw, child) in [(true, 4), (false, 9)] {
        let mut code = vec![];
        prelude(&mut code);
        call(&mut code, 0x2022, &[10, 3]);
        call(&mut code, 0x2050, &[10, 3]);
        call(&mut code, 0x2022, &[10, 4]);
        call(&mut code, 0x2212, &[2, 1]);
        set_image(&mut code, 1, 0, 3, 7);
        call(&mut code, 0x2212, &[2, 2]);
        set_image(&mut code, 2, 0, child, 7);
        call(&mut code, 0x2215, &[2]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        for _ in 0..9 {
            next(&mut vm, &mut scene, &mut buttons);
        }
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let before = scene.object(10).unwrap().clone();
        assert!(
            buttons.shared_buttons(&mut vm, &mut scene).is_err(),
            "{missing_draw}"
        );
        assert_eq!(scene.object(10).unwrap(), &before);
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x215));
        assert_eq!(vm.stack().len_bytes(), 4);
    }
}

/// Object 10 with children 3 and 4 (draw states), group 2 with button 7 at (0,0,10,10)
/// drawn by child 3 and button 9 at (20,0,10,10) drawn by child 4. State s of child c uses
/// crop base 10 * s + c, so every applied image is recognizable.
fn interactive(code: &mut Vec<u8>) {
    prelude(code);
    for child in [3, 4] {
        call(code, 0x2022, &[10, child]);
        call(code, 0x2050, &[10, child]);
    }
    for (id, child, x) in [(7, 3, 0), (9, 4, 20)] {
        call(code, 0x2212, &[2, id]);
        call(code, 0x2213, &[2, id, x, 0, 10, 10]);
        for state in 0..4 {
            set_image(code, id, state, child, 10 * state + child);
        }
    }
}
const SETUP: usize = 2 + 4 + 2 * 6;
/// Crop x of a child, which identifies the last applied state image.
fn crop_x(scene: &SceneHost, child: u16) -> i32 {
    scene.object(10).unwrap().children[&child]
        .draw
        .as_ref()
        .unwrap()
        .words[2] as i32
}
/// Runs the next 0x2216/0x2217 with `input` and returns system result 0 as i32.
fn frame(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    buttons: &mut ButtonHost,
    input: &mut ActionInput,
) -> i32 {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    assert!(buttons.shared_button_input(vm, scene, input).unwrap());
    vm.state().system_integer(0).unwrap() as i32
}
fn run_setup(vm: &mut Machine<'_>, scene: &mut SceneHost, buttons: &mut ButtonHost) {
    for _ in 0..SETUP {
        assert!(next(vm, scene, buttons));
    }
}

#[test]
fn pointer_hover_press_and_release_select_the_button_under_it() {
    let mut code = vec![];
    interactive(&mut code);
    for _ in 0..5 {
        call(&mut code, 0x2217, &[2]);
    }
    call(&mut code, 0x2216, &[2]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    run_setup(&mut vm, &mut scene, &mut buttons);
    let mut input = ActionInput::default();
    // No input: the pointer at (0, 0) is inside button 7, which becomes hovered.
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -1);
    assert_eq!(buttons.group(2).unwrap().current, Some(0));
    assert_eq!(crop_x(&scene, 3), 13);
    // Outside every button: hover cleared and image 0 restored.
    input.set_pointer(15, 5);
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -1);
    assert_eq!(buttons.group(2).unwrap().current, None);
    assert_eq!(crop_x(&scene, 3), 3);
    // Press over button 9 shows its pressed image.
    input.set_pointer(25, 5);
    input.set(0, true, true, false).unwrap();
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -1);
    let group = buttons.group(2).unwrap();
    assert_eq!(
        (group.current, group.press_target, group.pressed_visual),
        (Some(1), Some(1), true)
    );
    assert_eq!(crop_x(&scene, 4), 24);
    // Releasing over the same button selects its id and consumes action 0.
    input.set(0, false, false, true).unwrap();
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 9);
    assert!(!input.released(0));
    // Without further input the pressed image returns to the hover image.
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -1);
    assert_eq!(crop_x(&scene, 4), 14);
    assert!(!buttons.group(2).unwrap().pressed_visual);
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 9);
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn release_elsewhere_and_cancel_have_their_own_results() {
    let mut code = vec![];
    interactive(&mut code);
    for _ in 0..3 {
        call(&mut code, 0x2217, &[2]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    run_setup(&mut vm, &mut scene, &mut buttons);
    let mut input = ActionInput::default();
    input.set(0, true, true, false).unwrap();
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -1);
    // Moving to button 9 restores button 7 and hovers button 9; the release is elsewhere.
    input.set_pointer(25, 5);
    input.set(0, false, false, true).unwrap();
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -10);
    assert_eq!((crop_x(&scene, 3), crop_x(&scene, 4)), (3, 14));
    input.set(1, false, false, true).unwrap();
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), -2);
    assert!(!input.released(1));
}

#[test]
fn keyboard_navigation_follows_links_and_warps_the_pointer() {
    let mut code = vec![];
    interactive(&mut code);
    for _ in 0..3 {
        call(&mut code, 0x2217, &[2]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    run_setup(&mut vm, &mut scene, &mut buttons);
    let mut input = ActionInput::default();
    input.set_pointer(50, 50);
    // Action 2 without a current button goes to the last one.
    input.set(2, false, false, true).unwrap();
    frame(&mut vm, &mut scene, &mut buttons, &mut input);
    assert_eq!(buttons.group(2).unwrap().current, Some(1));
    assert_eq!(input.take_warp(), Some([25, 5]));
    assert!(!input.released(2));
    // Button 9 links back to 7 as its previous button.
    input.set(2, false, false, true).unwrap();
    frame(&mut vm, &mut scene, &mut buttons, &mut input);
    assert_eq!(buttons.group(2).unwrap().current, Some(0));
    assert_eq!(input.pointer(), [5, 5]);
    // Button 7 links forward to 9 through action 3.
    input.set(3, false, false, true).unwrap();
    frame(&mut vm, &mut scene, &mut buttons, &mut input);
    assert_eq!(buttons.group(2).unwrap().current, Some(1));
    assert_eq!(input.take_warp(), Some([25, 5]));
}

#[test]
fn missing_state_children_keep_the_frame_uncommitted() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2212, &[2, 7]);
    call(&mut code, 0x2213, &[2, 7, 0, 0, 10, 10]);
    call(&mut code, 0x2217, &[2]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..4 {
        next(&mut vm, &mut scene, &mut buttons);
    }
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    let mut input = ActionInput::default();
    input.set(1, false, false, true).unwrap();
    // The hover image names child 0, which does not exist.
    assert!(
        buttons
            .shared_button_input(&mut vm, &mut scene, &mut input)
            .is_err()
    );
    assert_eq!(buttons.group(2).unwrap().current, None);
    assert!(input.released(1));
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.state().system_integer(0), Some(1));
}

#[test]
fn empty_auxiliary_list_destruction_preserves_regular_groups_and_results() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2212, &[2, 7]);
    for _ in 0..2 {
        for slot in 0..8 {
            call(&mut code, 0x2371, &[slot]);
        }
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..3 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    let group = buttons.group(2).unwrap().clone();
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 100 + i).unwrap();
    }
    for _ in 0..16 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
        assert_eq!(buttons.group(2), Some(&group));
        assert!(scene.object(10).is_some());
        assert_eq!(vm.stack().len_bytes(), 0);
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(100 + i));
        }
    }
}

#[test]
fn auxiliary_list_destruction_rejects_missing_or_invalid_slot_atomically() {
    for args in [vec![], vec![8], vec![u32::MAX]] {
        let mut code = vec![];
        call(&mut code, 0x2371, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        vm.state_mut().set_system_integer(0, 123).unwrap();
        let (pc, sp, pending) = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        let mut buttons = ButtonHost::default();
        let mut scene = SceneHost::default();
        for _ in 0..2 {
            assert!(buttons.shared_buttons(&mut vm, &mut scene).is_err());
            assert_eq!(
                (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
                (pc, sp, pending)
            );
            assert_eq!(vm.state().system_integer(0), Some(123));
        }
    }
}

#[test]
fn auxiliary_list_creation_stays_a_barrier_for_empty_only_destruction() {
    for op in [0x237b] {
        let mut code = vec![];
        call(&mut code, op, &[0, 10, 1280, 720]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let (pc, sp, pending) = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        assert!(
            !ButtonHost::default()
                .shared_buttons(&mut vm, &mut SceneHost::default())
                .unwrap()
        );
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            (pc, sp, pending)
        );
    }
}

#[test]
fn auxiliary_list_records_keep_full_rects_links_and_truncated_state_words() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2370, &[7, 10]);
    for id in [7, 9] {
        call(&mut code, 0x2372, &[7, id]);
    }
    call(&mut code, 0x2373, &[7, 9, 0x80000000, 200, 300, u32::MAX]);
    call(
        &mut code,
        0x2374,
        &[
            7, 9, 2, 5, 0xfffe, 0xffff, 0x10000, 0x10001, 0x10002, 0x10003,
        ],
    );
    call(&mut code, 0x2379, &[7, 9, 1, 2, 3, 4]);
    call(&mut code, 0x237c, &[7, u32::MAX]);
    call(&mut code, 0x237d, &[7, 1, 2, 500, 600]);
    call(&mut code, 0x237e, &[7, 1]);
    call(&mut code, 0x2378, &[7]);
    call(&mut code, 0x2376, &[7]);
    call(&mut code, 0x2371, &[7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..3 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    let group = buttons.list_group(7).unwrap();
    assert_eq!(group.screen, [1280, 720, 1280, 720, 1280, 720]);
    assert_eq!(group.hit_point, None);
    assert!(group.active);
    for _ in 0..10 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    let group = buttons.list_group(7).unwrap();
    assert_eq!(group.buttons[0].links, [-1, 9, -1, -1]);
    let b = &group.buttons[1];
    assert_eq!(b.id, 9);
    assert_eq!(b.rect, [i32::MIN, 200, 300, -1]);
    assert_eq!(b.links, [1, 2, 3, 4]);
    assert_eq!(
        b.images[2],
        ButtonImage {
            child: 5,
            crop: [-2, -1, 0, 1],
            origin: [2, 3]
        }
    );
    assert!(group.keep_input && group.disable_navigation);
    assert!(!group.active);
    assert_eq!(group.hit_area, [1, 2, 500, 600]);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert!(next(&mut vm, &mut scene, &mut buttons));
    assert!(buttons.list_group(7).is_none());
    assert!(buttons.group(2).is_some());
    assert!(scene.object(10).is_some());
}

#[test]
fn auxiliary_list_invalid_and_short_commands_preserve_old_group() {
    for (op, args) in [
        (0x2370, vec![7, 256]),
        (0x2370, vec![7, 11]),
        (0x2372, vec![8, 1]),
        (0x2374, vec![7, 1, 0]),
        (0x237f, vec![7]),
    ] {
        let mut code = vec![];
        prelude(&mut code);
        call(&mut code, 0x2370, &[7, 10]);
        call(&mut code, op, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        for _ in 0..3 {
            assert!(next(&mut vm, &mut scene, &mut buttons));
        }
        let before = buttons.list_group(7).unwrap().clone();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let pending = vm.pending_system();
        let sp = vm.stack().len_bytes();
        assert!(buttons.shared_buttons(&mut vm, &mut scene).is_err());
        assert_eq!(buttons.list_group(7), Some(&before));
        assert_eq!(vm.pending_system(), pending);
        assert_eq!(vm.stack().len_bytes(), sp);
    }
}

#[test]
fn list_input_offsets_hits_by_parent_and_keeps_input_when_configured() {
    for keep in [0, 1] {
        let mut code = vec![];
        prelude(&mut code);
        call(&mut code, 0x2040, &[10]);
        call(&mut code, 0x2045, &[10, u32::MAX, 100, 50]);
        call(&mut code, 0x2022, &[10, 3]);
        call(&mut code, 0x2050, &[10, 3]);
        call(&mut code, 0x2370, &[7, 10]);
        call(&mut code, 0x2372, &[7, 9]);
        call(&mut code, 0x2373, &[7, 9, 10, 20, 30, 40]);
        for state in 0..4 {
            call(
                &mut code,
                0x2374,
                &[7, 9, state, 3, state * 10, 0, 4, 5, 0, 0],
            );
        }
        call(&mut code, 0x2375, &[7]);
        call(&mut code, 0x237c, &[7, keep]);
        let setup = code.len();
        for _ in 0..3 {
            call(&mut code, 0x2377, &[7]);
        }
        call(&mut code, 0x237f, &[7]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        while vm.pc() < setup {
            assert!(next(&mut vm, &mut scene, &mut buttons));
        }
        let mut input = ActionInput::default();
        input.set_pointer(120, 80);
        for (held, pressed, released, expected) in [
            (false, false, false, u32::MAX),
            (true, true, false, u32::MAX),
            (false, false, true, 9),
        ] {
            input.set(0, held, pressed, released).unwrap();
            while !matches!(vm.step().unwrap(), Step::System { .. }) {}
            assert!(
                buttons
                    .shared_list_input(
                        &mut vm,
                        &mut scene,
                        &mut input,
                        &cmvs_rs::gesture::PointerGesture::default()
                    )
                    .unwrap()
            );
            assert_eq!(vm.state().system_integer(0), Some(expected));
            assert_eq!(buttons.list_group(7).unwrap().hit_point, Some([10, 10]));
        }
        assert_eq!(input.released(0), keep != 0);
        assert!(next(&mut vm, &mut scene, &mut buttons));
        assert_eq!(vm.state().system_integer(1), Some(10));
        assert_eq!(vm.state().system_integer(2), Some(10));
    }
}

#[test]
fn list_input_hit_area_is_half_open_and_navigation_can_be_disabled() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2040, &[10]);
    call(&mut code, 0x2370, &[7, 10]);
    call(&mut code, 0x2372, &[7, 9]);
    call(&mut code, 0x2373, &[7, 9, 10, 20, 30, 40]);
    call(&mut code, 0x237d, &[7, 10, 20, 10, 10]);
    call(&mut code, 0x237e, &[7, 1]);
    let setup = code.len();
    for _ in 0..2 {
        call(&mut code, 0x2377, &[7]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    while vm.pc() < setup {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    let mut input = ActionInput::default();
    input.set(2, false, false, true).unwrap();
    for point in [[20, 25], [15, 30]] {
        input.set_pointer(point[0], point[1]);
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        buttons
            .shared_list_input(
                &mut vm,
                &mut scene,
                &mut input,
                &cmvs_rs::gesture::PointerGesture::default(),
            )
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
        assert_eq!(buttons.list_group(7).unwrap().hit_point, Some([-1, -1]));
        assert!(input.released(2));
        assert!(input.take_warp().is_none());
    }
}

#[test]
fn list_input_missing_visual_keeps_group_input_and_pending_command_unchanged() {
    let mut code = vec![];
    prelude(&mut code);
    call(&mut code, 0x2040, &[10]);
    call(&mut code, 0x2370, &[7, 10]);
    call(&mut code, 0x2372, &[7, 9]);
    call(&mut code, 0x2373, &[7, 9, 10, 20, 30, 40]);
    call(&mut code, 0x2377, &[7]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    for _ in 0..6 {
        assert!(next(&mut vm, &mut scene, &mut buttons));
    }
    let mut input = ActionInput::default();
    input.set_pointer(20, 30);
    input.set(0, true, true, false).unwrap();
    let old_input = input.clone();
    let group = buttons.list_group(7).unwrap().clone();
    let object = scene.object(10).unwrap().clone();
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    let pc = vm.pc();
    let sp = vm.stack().len_bytes();
    assert!(
        buttons
            .shared_list_input(
                &mut vm,
                &mut scene,
                &mut input,
                &cmvs_rs::gesture::PointerGesture::default()
            )
            .is_err()
    );
    assert_eq!(buttons.list_group(7), Some(&group));
    assert_eq!(scene.object(10), Some(&object));
    assert_eq!(input, old_input);
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc, sp));
}

#[test]
fn making_a_button_current_swaps_images_and_can_warp_the_pointer() {
    let mut code = vec![];
    interactive(&mut code);
    call(&mut code, 0x221a, &[2, 7, 0]);
    call(&mut code, 0x221a, &[2, 9, 1]);
    call(&mut code, 0x221a, &[2, 9, 1]);
    call(&mut code, 0x221a, &[2, 8, 1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
    run_setup(&mut vm, &mut scene, &mut buttons);
    let mut input = ActionInput::default();
    input.set_pointer(100, 100);
    // Button 7 becomes current with its hover image; the pointer stays without the flag.
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 1);
    assert_eq!(crop_x(&scene, 3), 13);
    assert_eq!(input.take_warp(), None);
    // Button 9: 7 gets image 0 back, 9 its hover image, and the pointer moves to its centre.
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 1);
    assert_eq!((crop_x(&scene, 3), crop_x(&scene, 4)), (3, 14));
    assert_eq!(input.take_warp(), Some([25, 5]));
    assert_eq!(buttons.group(2).unwrap().current, Some(1));
    // Already inside: no warp; an unknown id gives 0 and changes nothing.
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 1);
    assert_eq!(input.take_warp(), None);
    assert_eq!(frame(&mut vm, &mut scene, &mut buttons, &mut input), 0);
    assert_eq!(buttons.group(2).unwrap().current, Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn scaled_group_creation_accepts_the_device_scale_only() {
    for (scale, accepted) in [([1280, 720], true), ([640, 360], false)] {
        let mut code = vec![];
        call(&mut code, 0x2020, &[10]);
        call(&mut code, 0x221b, &[2, 10, scale[0], scale[1]]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let (mut scene, mut buttons) = (SceneHost::default(), ButtonHost::default());
        assert!(next(&mut vm, &mut scene, &mut buttons));
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert_eq!(
            buttons.shared_buttons(&mut vm, &mut scene).is_ok(),
            accepted
        );
        assert_eq!(buttons.group(2).is_some(), accepted);
        assert_eq!(vm.stack().len_bytes(), if accepted { 0 } else { 16 });
    }
}

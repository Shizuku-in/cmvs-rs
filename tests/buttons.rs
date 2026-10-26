use cmvs_rs::{
    buttons::{Button, ButtonHost, ButtonImage},
    machine::{Machine, Step},
    ps3::Script,
    scene::SceneHost,
};

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
    scene.try_handle(vm).unwrap() || buttons.try_handle(vm, scene).unwrap()
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
        assert!(buttons.try_handle(&mut vm, &mut scene).is_err(), "{op:#x}");
        assert_eq!(vm.pending_system(), Some((pc, op - 0x2000)));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(buttons.group(2).unwrap().buttons, vec![]);
        assert!(buttons.group(3).is_none() && buttons.group(0).is_none());
        assert_eq!(vm.state().system_integer(0), None);
    }
}

#[test]
fn display_and_input_commands_are_left_pending() {
    for op in [0x2216, 0x2217, 0x2219] {
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
            buttons.try_handle(&mut vm, &mut scene).is_err(),
            "{missing_draw}"
        );
        assert_eq!(scene.object(10).unwrap(), &before);
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x215));
        assert_eq!(vm.stack().len_bytes(), 4);
    }
}

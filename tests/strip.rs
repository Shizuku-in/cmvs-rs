use cmvs_rs::{
    fade::SkipGate,
    input::ActionInput,
    machine::{Machine, Step},
    ps3::Script,
    scene::{SceneHost, StripWipe},
    scene_render,
};

fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    let mut raw = b"PS2A".to_vec();
    for value in [
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
        raw.extend(value.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
/// Arguments are listed from SP-4 outward.
fn command(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
/// 0x222f arguments: mode, color, size, duration, layer.
fn wipe(code: &mut Vec<u8>, mode: u32, size: u32, duration: u32) {
    command(code, 0x222f, &[mode, 0x12_3456, size, duration, 9]);
}
/// A scene after creating one wipe.
fn scene_with(mode: u32, size: u32, duration: u32) -> SceneHost {
    let mut code = vec![];
    wipe(&mut code, mode, size, duration);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    pending(&mut vm);
    assert!(scene.try_handle_strip(&mut vm).unwrap());
    scene
}
/// (crop width, crop height, x, y) of strip `index`.
fn strip(wipe: &StripWipe, index: usize) -> [u32; 4] {
    let words = &wipe.strips[index].object.draw.as_ref().unwrap().words;
    [words[4], words[5], words[8], words[9]]
}

#[test]
fn creation_lays_out_solid_strips_and_replaces_the_previous_wipe() {
    let mut code = vec![];
    wipe(&mut code, 0, 20, 1250);
    wipe(&mut code, 2, 50, 300);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    pending(&mut vm);
    let pc = vm.pc();
    vm.state_mut().set_system_integer(0, 7).unwrap();
    assert!(scene.try_handle_strip(&mut vm).unwrap());
    assert_eq!((vm.pending_system(), vm.pc()), (None, pc + 2));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    let columns = scene.strip_wipe().unwrap();
    assert_eq!(
        (columns.mode, columns.color, columns.size, columns.duration),
        (0, 0x12_3456, 20, 1250)
    );
    assert_eq!((columns.layer, columns.screen), (9, [1280, 720]));
    assert!(!columns.finished && columns.object.draw.is_none());
    assert_eq!(columns.strips.len(), 64);
    for (i, s) in columns.strips.iter().enumerate() {
        let x = 20 * i as u32;
        assert_eq!((s.id, s.base, s.full), (i as u32, [x, 0], [20, 720]));
        assert_eq!(s.object.flags, 0);
        let words = &s.object.draw.as_ref().unwrap().words;
        assert_eq!(&words[2..11], &[0, 0, 0, 720, 0, 0, x, 0, 9]);
        assert_eq!(words[14], 0xff12_3456);
        assert_eq!(words[17], 255);
    }
    // 720 / 50 leaves a remainder, so a 15th row extends past the screen.
    pending(&mut vm);
    assert!(scene.try_handle_strip(&mut vm).unwrap());
    let rows = scene.strip_wipe().unwrap();
    assert_eq!((rows.mode, rows.strips.len()), (2, 15));
    assert_eq!(rows.strips[14].base, [0, 700]);
    assert_eq!(rows.strips[14].full, [1280, 50]);
    assert_eq!(strip(rows, 14), [1280, 0, 0, 700]);
    // Revealing layouts start with full strips.
    assert_eq!(
        strip(scene_with(10, 300, 1).strip_wipe().unwrap(), 4),
        [300, 720, 1200, 0]
    );
    assert_eq!(
        strip(scene_with(12, 40, 1).strip_wipe().unwrap(), 17),
        [1280, 40, 0, 680]
    );
}

#[test]
fn covering_strips_grow_in_turn_and_finish_as_a_full_screen_cover() {
    let mut scene = scene_with(0, 20, 1000);
    // progress = elapsed × (64 + 20) / 1000.
    scene.advance_frame(100);
    let wipe = scene.strip_wipe().unwrap();
    assert_eq!((wipe.elapsed, strip(wipe, 0)), (100, [8, 720, 0, 0]));
    assert_eq!(strip(wipe, 7), [1, 720, 140, 0]);
    assert_eq!(strip(wipe, 8), [0, 720, 160, 0]);
    scene.advance_frame(400);
    let wipe = scene.strip_wipe().unwrap();
    assert_eq!(strip(wipe, 0)[0], 20);
    assert_eq!(strip(wipe, 30)[0], 12);
    assert_eq!(strip(wipe, 41)[0], 1);
    assert_eq!(strip(wipe, 42)[0], 0);
    assert_eq!(wipe.remaining(), 500);
    scene.advance_frame(500);
    let wipe = scene.strip_wipe().unwrap();
    assert!(wipe.finished && wipe.strips.is_empty());
    let words = &wipe.object.draw.as_ref().unwrap().words;
    assert_eq!(&words[2..11], &[0, 0, 1280, 720, 0, 0, 0, 0, 9]);
    assert_eq!(words[14], 0xff12_3456);
    assert_eq!(wipe.remaining(), u32::MAX);
    let before = wipe.clone();
    scene.advance_frame(16);
    assert_eq!(scene.strip_wipe(), Some(&before));
}

#[test]
fn revealing_strips_shrink_towards_their_far_edge_and_free_the_wipe() {
    let mut columns = scene_with(10, 20, 1000);
    columns.advance_frame(100);
    let wipe = columns.strip_wipe().unwrap();
    assert_eq!(strip(wipe, 0), [12, 720, 8, 0]);
    assert_eq!(strip(wipe, 1), [13, 720, 27, 0]);
    assert_eq!(strip(wipe, 8), [20, 720, 160, 0]);
    // progress = elapsed × (24 + 30) / 1000 for 24 rows of 30.
    let mut rows = scene_with(12, 30, 1000);
    rows.advance_frame(100);
    let wipe = rows.strip_wipe().unwrap();
    assert_eq!(strip(wipe, 0), [1280, 25, 0, 5]);
    assert_eq!(strip(wipe, 4), [1280, 29, 0, 121]);
    assert_eq!(strip(wipe, 5), [1280, 30, 0, 150]);
    // Frame deltas are not clamped; the end frees the wipe outright.
    rows.advance_frame(900);
    assert!(rows.strip_wipe().is_none());
    let mut instant = scene_with(10, 20, 0);
    instant.advance_frame(0);
    assert!(instant.strip_wipe().is_none());
}

#[test]
fn finish_free_and_query_commands_pop_nothing() {
    let mut code = vec![];
    command(&mut code, 0x2232, &[]);
    command(&mut code, 0x2230, &[]);
    wipe(&mut code, 0, 20, 1250);
    command(&mut code, 0x2232, &[]);
    command(&mut code, 0x2230, &[]);
    command(&mut code, 0x2232, &[]);
    command(&mut code, 0x2231, &[]);
    wipe(&mut code, 10, 20, 1250);
    command(&mut code, 0x2230, &[]);
    command(&mut code, 0x2232, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let next = |vm: &mut Machine<'_>, scene: &mut SceneHost| {
        pending(vm);
        vm.state_mut().set_system_integer(0, 7).unwrap();
        assert!(scene.try_handle_strip(vm).unwrap());
        assert_eq!(vm.stack().len_bytes(), 0);
        vm.state().system_integer(0)
    };
    assert_eq!(next(&mut vm, &mut scene), Some(u32::MAX));
    assert_eq!(next(&mut vm, &mut scene), Some(7));
    assert!(scene.strip_wipe().is_none());
    assert_eq!(next(&mut vm, &mut scene), Some(1));
    scene.advance_frame(300);
    assert_eq!(next(&mut vm, &mut scene), Some(950));
    // Finishing a covering wipe keeps it as the cover; freeing removes it.
    assert_eq!(next(&mut vm, &mut scene), Some(7));
    let cover = scene.strip_wipe().unwrap();
    assert!(cover.finished && cover.strips.is_empty() && cover.object.draw.is_some());
    assert_eq!(next(&mut vm, &mut scene), Some(u32::MAX));
    assert_eq!(next(&mut vm, &mut scene), Some(7));
    assert!(scene.strip_wipe().is_none());
    assert_eq!(next(&mut vm, &mut scene), Some(1));
    assert_eq!(next(&mut vm, &mut scene), Some(7));
    assert!(scene.strip_wipe().is_none());
    assert_eq!(next(&mut vm, &mut scene), Some(u32::MAX));
}

/// Creates a wipe, then runs 0x2233 once: (step, stack bytes, system result 0) and the scene.
fn wait_once(
    mode: u32,
    skippable: u32,
    gate: SkipGate,
    input: &mut ActionInput,
) -> ((Option<Step>, usize, Option<u32>), SceneHost) {
    let mut code = vec![];
    wipe(&mut code, mode, 20, 1250);
    command(&mut code, 0x2233, &[skippable]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    pending(&mut vm);
    assert!(scene.try_handle_strip(&mut vm).unwrap());
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 7).unwrap();
    let step = scene.try_wait_strip(&mut vm, gate, input).unwrap();
    let result = (step, vm.stack().len_bytes(), vm.state().system_integer(0));
    (result, scene)
}
/// Runs a lone 0x2233 with SP-4 = 1 on `scene`.
fn lone_wait(scene: &mut SceneHost) -> (Option<Step>, usize, Option<u32>) {
    let mut code = vec![];
    command(&mut code, 0x2233, &[1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 7).unwrap();
    let step = scene
        .try_wait_strip(&mut vm, SkipGate::default(), &mut ActionInput::default())
        .unwrap();
    (step, vm.stack().len_bytes(), vm.state().system_integer(0))
}

#[test]
fn wait_yields_until_the_wipe_ends_and_skips_like_other_effects() {
    let gate = SkipGate::default();
    for skippable in [0, 1] {
        let mut input = ActionInput::default();
        input.set(0, true, true, false).unwrap();
        let before = input.clone();
        let (result, scene) = wait_once(0, skippable, gate, &mut input);
        assert_eq!(result, (Some(Step::Yield), 4, Some(0)));
        assert_eq!(input, before);
        assert!(!scene.strip_wipe().unwrap().finished);
    }
    // A released action 0 is consumed and ends the wait; a covering wipe stays as the cover.
    let mut input = ActionInput::default();
    input.set(0, false, false, true).unwrap();
    input.set(1, false, false, true).unwrap();
    let (result, mut scene) = wait_once(0, 1, gate, &mut input);
    assert_eq!(result, (Some(Step::Yield), 0, Some(1)));
    assert!(!input.released(0) && input.released(1));
    assert!(scene.strip_wipe().unwrap().finished);
    // The finished cover no longer blocks: 0x4004 with result 0.
    assert_eq!(lone_wait(&mut scene), (Some(Step::Continue), 0, Some(0)));
    assert!(scene.strip_wipe().is_some());
    // Unskippable waits ignore released clicks.
    let mut input = ActionInput::default();
    input.set(0, false, false, true).unwrap();
    let (result, _) = wait_once(10, 0, gate, &mut input);
    assert_eq!(result, (Some(Step::Yield), 4, Some(0)));
    assert!(input.released(0));
    // Held skip actions, the skip flag and disabled effects end it without consuming; a
    // revealing wipe is freed.
    let held = || {
        let mut input = ActionInput::default();
        input.set(7, true, false, false).unwrap();
        input.set(0, false, false, true).unwrap();
        input
    };
    let off = SkipGate {
        effects: false,
        skip: false,
    };
    let flag = SkipGate {
        effects: true,
        skip: true,
    };
    for (gate, mut input) in [
        (gate, held()),
        (off, ActionInput::default()),
        (flag, held()),
    ] {
        let before = input.clone();
        let (result, scene) = wait_once(10, 1, gate, &mut input);
        assert_eq!(result, (Some(Step::Yield), 0, Some(1)));
        assert_eq!(input, before);
        assert!(scene.strip_wipe().is_none());
    }
    // Without a wipe the wait completes at once.
    assert_eq!(
        lone_wait(&mut SceneHost::default()),
        (Some(Step::Continue), 0, Some(0))
    );
}

#[test]
fn unverified_layouts_and_short_stacks_keep_the_command_pending() {
    for args in [
        vec![1, 0, 20, 1250, 9],
        vec![0x6e, 0, 20, 1250, 9],
        vec![0, 0, 0, 1250, 9],
        vec![0, 0, 20, 1250],
    ] {
        let mut code = vec![];
        wipe(&mut code, 10, 20, 1250);
        command(&mut code, 0x222f, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        pending(&mut vm);
        assert!(scene.try_handle_strip(&mut vm).unwrap());
        let before = scene.strip_wipe().cloned();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(scene.try_handle_strip(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x22f)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(scene.strip_wipe().cloned(), before);
    }
    let mut code = vec![];
    command(&mut code, 0x2233, &[]);
    let short = script(&code);
    let mut vm = Machine::new(&short).unwrap();
    pending(&mut vm);
    let mut scene = scene_with(0, 20, 1250);
    assert!(
        scene
            .try_wait_strip(&mut vm, SkipGate::default(), &mut ActionInput::default())
            .is_err()
    );
    assert!(vm.pending_system().is_some());
    // 0x2247 (slot-object wipe) and the neighbours stay unhandled; 0x2233 is the wait.
    for op in [0x222e, 0x2233, 0x2234, 0x2247] {
        let mut code = vec![];
        command(&mut code, op, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(!SceneHost::default().try_handle_strip(&mut vm).unwrap());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(op - 0x2000));
    }
}

#[test]
fn the_software_renderer_rejects_an_active_wipe() {
    let mut scene = scene_with(0, 20, 1250);
    assert!(scene_render::render(&scene, 1280, 720).is_err());
    scene.advance_frame(1250);
    assert!(scene_render::render(&scene, 1280, 720).is_err());
    assert!(scene_render::render(&SceneHost::default(), 1280, 720).is_ok());
}

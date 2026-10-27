use cmvs_rs::{
    fade::SkipGate,
    image::ImageSource,
    input::ActionInput,
    machine::{Machine, Step},
    ps3::Script,
    scene::SceneHost,
    scene_render,
};
use std::{collections::BTreeMap, error::Error};

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
/// 2x1 PB3 type 1 of solid 1x1 tiles; planes are stored B, G, R.
fn pb3(planes: &[[u8; 2]; 3]) -> Vec<u8> {
    let mut b = vec![0; 52];
    b[..4].copy_from_slice(b"PB3B");
    put(&mut b, 8, 1);
    put(&mut b, 24, 1);
    b[28] = 1;
    b[30] = 2;
    b[32] = 1;
    b[34] = 24;
    put(&mut b, 44, 52);
    put(&mut b, 48, 52 + 19 * 3);
    for _ in planes {
        b.extend(15u32.to_le_bytes());
    }
    for plane in planes {
        for v in [1u32, 2, 0] {
            b.extend(v.to_le_bytes());
        }
        b.push(0xc0);
        b.extend(plane);
    }
    for _ in planes {
        b.extend(0u32.to_le_bytes());
    }
    let len = b.len() as u32;
    put(&mut b, 4, len);
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
    b
}

/// Text offsets: white object 0, mask 6 (green 0 then 255), missing name 12.
const TEXT: &[u8] = b"w.pb3\0m.pb3\0x.pb3\0";
const MASK: u32 = 6;

fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    payload.extend(TEXT);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        TEXT.len() as u32,
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
/// A drawn 2x1 white object in `slot`.
fn object(code: &mut Vec<u8>, slot: u32) {
    command(code, 0x2020, &[slot]);
    command(code, 0x2030, &[slot, 0]);
    command(code, 0x2040, &[slot]);
    command(code, 0x2044, &[slot, u32::MAX, 0, 0, 2, 1]);
}
fn transition(code: &mut Vec<u8>, slot: u32, duration: u32, put: u32) {
    command(code, 0x222a, &[MASK, slot, duration, duration / 2, put]);
}

struct Files(BTreeMap<Vec<u8>, Vec<u8>>);
impl ImageSource for Files {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(self.0.get(name).cloned())
    }
}
fn files() -> Files {
    let mut files = BTreeMap::new();
    files.insert(b"w.pb3".to_vec(), pb3(&[[255; 2]; 3]));
    files.insert(b"m.pb3".to_vec(), pb3(&[[9, 9], [0, 255], [9, 9]]));
    Files(files)
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
/// Runs `commands` scene commands, including mask creation and finishing.
fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, commands: usize) {
    let mut files = files();
    for _ in 0..commands {
        pending(vm);
        assert!(
            scene.try_handle(vm).unwrap()
                || scene.try_handle_image(vm, &mut files).unwrap()
                || scene.try_handle_mask(vm, &mut files).unwrap()
        );
    }
}
/// Red channel of each pixel: white over the black clear color, so the drawn alpha.
fn alphas(scene: &SceneHost) -> Vec<u8> {
    let frame = scene_render::render(scene, 2, 1).unwrap();
    frame.pixels().chunks_exact(4).map(|p| p[0]).collect()
}

#[test]
fn put_transition_reveals_by_mask_green_and_keeps_the_object() {
    let mut code = vec![];
    object(&mut code, 3);
    transition(&mut code, 3, 100, 1);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 4);
    assert_eq!(alphas(&scene), [255, 255]);
    vm.state_mut().set_system_integer(0, 9).unwrap();
    run(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);
    let mask = scene.mask().unwrap();
    assert_eq!(
        (mask.put, mask.slot, mask.ramp, mask.width, mask.elapsed),
        (true, 3, 50, 50, 0)
    );
    assert_eq!(mask.mask.name, b"m.pb3");
    // count = 2 * elapsed / duration; mask green 0 leads, green 255 trails by one unit.
    for (delta, expected) in [(0, [0, 0]), (25, [128, 0]), (50, [255, 128])] {
        scene.advance_frame(delta);
        assert_eq!(alphas(&scene), expected, "after {delta} ms");
    }
    scene.advance_frame(25);
    assert!(scene.mask().is_none());
    assert_eq!(alphas(&scene), [255, 255]);
}

#[test]
fn cut_transition_hides_by_inverted_mask_and_destroys_the_object() {
    let mut code = vec![];
    object(&mut code, 3);
    transition(&mut code, 3, 100, 0);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 5);
    assert_eq!(alphas(&scene), [255, 255]);
    scene.advance_frame(50);
    assert_eq!(alphas(&scene), [0, 255]);
    scene.advance_frame(25);
    assert_eq!(alphas(&scene), [0, 128]);
    // Elapsed time past the end is clamped before the object is destroyed.
    scene.advance_frame(1000);
    assert!(scene.mask().is_none() && scene.object(3).is_none());
}

#[test]
fn zero_duration_starts_at_the_end_value_and_finishes_on_the_next_frame() {
    for put in [1, 0] {
        let mut code = vec![];
        object(&mut code, 3);
        transition(&mut code, 3, 0, put);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 5);
        let expected = if put == 1 { 2.0 } else { 0.0 };
        assert_eq!(scene.mask().unwrap().progress(), expected);
        scene.advance_frame(0);
        assert!(scene.mask().is_none());
        assert_eq!(scene.object(3).is_some(), put == 1);
    }
}

fn wait(scene: &mut SceneHost, skippable: u32, gate: SkipGate, input: &mut ActionInput) -> Step {
    let mut code = vec![];
    command(&mut code, 0x222b, &[skippable]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    pending(&mut vm);
    let step = scene.try_wait_mask(&mut vm, gate, input).unwrap().unwrap();
    let done = vm.pending_system().is_none();
    assert_eq!(vm.stack().len_bytes(), if done { 0 } else { 4 });
    let result = vm.state().system_integer(0).unwrap();
    assert_eq!(
        result,
        u32::from(done && scene.mask().is_none() && step == Step::Yield)
    );
    step
}
fn active(put: u32) -> SceneHost {
    let mut code = vec![];
    object(&mut code, 3);
    transition(&mut code, 3, 100, put);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 5);
    scene
}

#[test]
fn wait_yields_until_skipped_by_settings_actions_or_a_consumed_release() {
    let gate = SkipGate::default();
    let mut released = ActionInput::default();
    released.set(0, false, false, true).unwrap();
    // Without a transition the wait completes without yielding and reports 0.
    let mut empty = SceneHost::default();
    assert_eq!(wait(&mut empty, 1, gate, &mut released), Step::Continue);
    // A non-skippable wait ignores input until the frame update frees the transition.
    let mut scene = active(1);
    assert_eq!(wait(&mut scene, 0, gate, &mut released), Step::Yield);
    assert!(released.released(0) && scene.mask().is_some());
    let mut idle = ActionInput::default();
    assert_eq!(wait(&mut scene, 1, gate, &mut idle), Step::Yield);
    assert!(scene.mask().is_some());
    assert_eq!(wait(&mut scene, 1, gate, &mut released), Step::Yield);
    assert!(!released.released(0) && scene.mask().is_none());
    assert_eq!(alphas(&scene), [255, 255]);
    // Settings and held skip actions finish at once without consuming input.
    let mut held = ActionInput::default();
    held.set(8, true, true, true).unwrap();
    held.set(0, false, false, true).unwrap();
    for (gate, input) in [
        (
            SkipGate {
                effects: false,
                skip: false,
            },
            ActionInput::default(),
        ),
        (
            SkipGate {
                effects: true,
                skip: true,
            },
            ActionInput::default(),
        ),
        (gate, held),
    ] {
        let mut scene = active(0);
        let mut input = input;
        let before = input.clone();
        assert_eq!(wait(&mut scene, 1, gate, &mut input), Step::Yield);
        assert_eq!(input, before);
        assert!(scene.mask().is_none() && scene.object(3).is_none());
    }
}

#[test]
fn finishing_command_and_replacement_keep_previous_cut_objects() {
    let mut code = vec![];
    object(&mut code, 3);
    object(&mut code, 4);
    transition(&mut code, 3, 100, 0);
    transition(&mut code, 4, 100, 0);
    command(&mut code, 0x222d, &[]);
    command(&mut code, 0x222d, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 10);
    // The replaced Cut transition is dropped without destroying slot 3.
    assert_eq!(scene.mask().unwrap().slot, 4);
    vm.state_mut().set_system_integer(0, 9).unwrap();
    run(&mut vm, &mut scene, 1);
    assert!(scene.mask().is_none());
    assert!(scene.object(3).is_some() && scene.object(4).is_none());
    // Without a transition 0x222d is a no-op; neither pops nor sets a result.
    run(&mut vm, &mut scene, 1);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn creation_rejects_missing_objects_masks_and_arguments_transactionally() {
    let mut cases = vec![];
    for (name, slot) in [
        (MASK, 5),
        (MASK, 256),
        (MASK, u32::MAX),
        (12, 3),
        (0x8000_0001, 3),
    ] {
        let mut code = vec![];
        object(&mut code, 3);
        transition(&mut code, 3, 100, 1);
        command(&mut code, 0x222a, &[name, slot, 100, 50, 1]);
        cases.push(code);
    }
    let mut short = vec![];
    object(&mut short, 3);
    transition(&mut short, 3, 100, 1);
    command(&mut short, 0x222a, &[MASK, 3, 100, 50]);
    cases.push(short);
    for code in cases {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, 5);
        let before = scene.mask().cloned();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(scene.try_handle_mask(&mut vm, &mut files()).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(scene.mask().cloned(), before);
        assert_eq!(vm.state().system_integer(0), Some(9));
    }
}

#[test]
fn render_rejects_masks_on_cropped_images() {
    let mut code = vec![];
    object(&mut code, 3);
    command(&mut code, 0x2044, &[3, u32::MAX, 0, 0, 1, 1]);
    transition(&mut code, 3, 100, 1);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 6);
    assert!(scene_render::render(&scene, 2, 1).is_err());
    scene.advance_frame(100);
    assert!(scene_render::render(&scene, 2, 1).is_ok());
}

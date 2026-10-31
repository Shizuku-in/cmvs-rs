use cmvs_rs::{
    image::ImageSource,
    machine::{Machine, Step},
    ps3::Script,
    raster::RgbaImage,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
    scene_render,
};
use std::error::Error;

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
/// 2x1 white PB3 type 1 of solid 1x1 tiles.
fn white() -> Vec<u8> {
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
    for _ in 0..3 {
        b.extend(15u32.to_le_bytes());
    }
    for _ in 0..3 {
        for v in [1u32, 2, 0] {
            b.extend(v.to_le_bytes());
        }
        b.push(0xc0);
        b.extend([255, 255]);
    }
    for _ in 0..3 {
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
struct White;
impl ImageSource for White {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(Some(white()))
    }
}

const TEXT: &[u8] = b"w.pb3\0";
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
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
fn run(vm: &mut Machine<'_>, scene: &mut SceneHost, commands: usize) {
    for _ in 0..commands {
        pending(vm);
        assert!(
            scene.try_handle(vm).unwrap()
                || scene.try_handle_quake(vm).unwrap()
                || scene.try_handle_image(vm, &mut White).unwrap()
        );
    }
}
/// A scene whose only commands are `code`.
fn scene_of(code: &[u8], commands: usize) -> SceneHost {
    let script = script(code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, commands);
    scene
}
fn quake_code(mode: u32, amplitude: u32, period: u32, count: u32) -> Vec<u8> {
    let mut code = vec![];
    command(&mut code, 0x2225, &[mode, amplitude, period, count]);
    code
}

#[test]
fn damped_cosine_truncates_like_the_original_and_ends_at_the_duration() {
    // 0x201: 40 px, 360 ms, 3 cycles, horizontal. Angle = 3 × elapsed; cos(angle + 90°).
    let mut scene = scene_of(&quake_code(0x201, 40, 360, 3), 1);
    assert_eq!(scene.quake().unwrap().offset, [0, 0]);
    // cos(120°) × 40 = −19.99… truncates to −19, then 350 × −19 / 360 → −18.
    // cos(180°) gives −40 and 330 × −40 / 360 → −36; 270° rounds to 0; 0° is exactly 1.
    for (delta, expected, steps) in [
        (10, -18, 0),
        (20, -36, 0),
        (30, 0, 0),
        (30, 30, 0),
        (30, 0, 1),
    ] {
        scene.advance_frame(delta);
        let quake = scene.quake().unwrap();
        assert_eq!((quake.offset, quake.steps), ([expected, 0], steps));
    }
    scene.advance_frame(239);
    assert_eq!(scene.quake().unwrap().offset[0], 0);
    scene.advance_frame(1);
    assert!(scene.quake().is_none());
    // 0x211 shakes vertically.
    let mut scene = scene_of(&quake_code(0x211, 40, 360, 3), 1);
    scene.advance_frame(30);
    assert_eq!(scene.quake().unwrap().offset, [0, -36]);
}

#[test]
fn step_quakes_toggle_or_rotate_and_run_count_plus_two_steps() {
    // Rotating corners, 2 px every 10 ms, count 1: the step index is compared with the
    // count before it increments, so the third step (index 2) ends and frees the quake.
    let mut scene = scene_of(&quake_code(0x30, 2, 10, 1), 1);
    scene.advance_frame(9);
    assert_eq!(scene.quake().unwrap().offset, [0, 0]);
    scene.advance_frame(1);
    assert_eq!(scene.quake().unwrap().offset, [-2, -2]);
    scene.advance_frame(10);
    let state = scene.quake().unwrap();
    assert_eq!((state.offset, state.steps), ([2, 2], 2));
    scene.advance_frame(10);
    assert!(scene.quake().is_none());
    // Two periods in one frame step twice and keep the remainder.
    let mut scene = scene_of(&quake_code(0x30, 2, 10, 5), 1);
    scene.advance_frame(25);
    let state = scene.quake().unwrap();
    assert_eq!((state.offset, state.steps, state.elapsed), ([2, 2], 2, 5));
    // Horizontal steps toggle between the amplitude and its negation.
    let mut scene = scene_of(&quake_code(0x00, 3, 10, u32::MAX), 1);
    let mut seen = vec![];
    for _ in 0..4 {
        scene.advance_frame(10);
        seen.push(scene.quake().unwrap().offset);
    }
    assert_eq!(seen, [[3, 0], [-3, 0], [3, 0], [-3, 0]]);
}

/// 4x1 frame of a white 1x1 slot object at x = 1 and an extra white 1x1 object at x = 2
/// (its flags given), black elsewhere.
fn frame(scene: &SceneHost, extra_flags: u32) -> Vec<u8> {
    let mut draw = DrawState::default();
    draw.words[4] = 1;
    draw.words[5] = 1;
    draw.words[8] = 2;
    let extra = SceneObject {
        image: Some(ObjectImage {
            filter: None,
            name: vec![],
            pixels: RgbaImage::new(1, 1, vec![255; 4]).unwrap(),
            alpha: false,
            surface: false,
        }),
        draw: Some(draw),
        flags: extra_flags,
        ..SceneObject::default()
    };
    let frame = scene_render::render_with_objects(scene, &[&extra], 4, 1).unwrap();
    frame
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[0])
        .collect()
}
fn shaken_scene(mode: u32) -> SceneHost {
    let mut code = vec![];
    command(&mut code, 0x2020, &[3]);
    command(&mut code, 0x2030, &[3, 0]);
    command(&mut code, 0x2040, &[3]);
    command(&mut code, 0x2044, &[3, u32::MAX, 0, 0, 1, 1]);
    command(&mut code, 0x2045, &[3, u32::MAX, 1, 0]);
    code.extend(quake_code(mode, 1, 10, u32::MAX));
    let mut scene = scene_of(&code, 6);
    scene.advance_frame(10);
    scene
}

#[test]
fn render_offsets_follow_the_layer_nibble_and_object_bit() {
    // Without a quake only the extra object is drawn, at 2.
    assert_eq!(frame(&SceneHost::default(), 0x7fff_ffff), [0, 0, 255, 0]);
    // The step quake moves by 1 px. Nibble 1 moves only the slot object (1 → 2), nibble 2
    // only the extra layer (2 → 3), nibble 0 both.
    assert_eq!(frame(&shaken_scene(0x01), 0x7fff_ffff), [0, 0, 255, 0]);
    assert_eq!(frame(&shaken_scene(0x02), 0x7fff_ffff), [0, 255, 0, 255]);
    assert_eq!(frame(&shaken_scene(0x00), 0x7fff_ffff), [0, 0, 255, 255]);
    // Without +6248 bit 0x100 the extra object stays put.
    assert_eq!(frame(&shaken_scene(0x00), 0x7fff_feff), [0, 0, 255, 0]);
    assert_eq!(frame(&shaken_scene(0x02), 0x7fff_feff), [0, 255, 255, 0]);
}

#[test]
fn queries_pause_stop_and_wait_follow_the_original_results() {
    // 0x2226: count − steps; −1 for endless or missing quakes.
    for (mut code, commands, expected) in [
        (quake_code(0x00, 3, 10, 5), 2, 5),
        (quake_code(0x00, 3, 10, u32::MAX), 2, u32::MAX),
        (vec![], 1, u32::MAX),
    ] {
        command(&mut code, 0x2226, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        run(&mut vm, &mut scene, commands);
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    // 0x2227 pauses: time and offset stand still and the offset reads as zero.
    let mut code = quake_code(0x00, 3, 10, u32::MAX);
    command(&mut code, 0x2227, &[1]);
    command(&mut code, 0x2227, &[0]);
    command(&mut code, 0x2229, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    run(&mut vm, &mut scene, 1);
    scene.advance_frame(10);
    run(&mut vm, &mut scene, 1);
    scene.advance_frame(10);
    let quake_state = scene.quake().unwrap();
    assert_eq!((quake_state.elapsed, quake_state.offset), (0, [3, 0]));
    assert_eq!(quake_state.offset(false), [0, 0]);
    run(&mut vm, &mut scene, 1);
    scene.advance_frame(10);
    assert_eq!(scene.quake().unwrap().offset, [-3, 0]);
    run(&mut vm, &mut scene, 1);
    assert!(scene.quake().is_none());
    assert_eq!(vm.stack().len_bytes(), 0);
}

fn wait(scene: &mut SceneHost, value: u32) -> (Step, u32, usize) {
    let mut code = vec![];
    command(&mut code, 0x2228, &[value]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    pending(&mut vm);
    let step = scene.try_wait_quake(&mut vm).unwrap().unwrap();
    let result = vm.state().system_integer(0).unwrap();
    (step, result, vm.stack().len_bytes())
}

#[test]
fn wait_yields_while_shaking_or_stops_at_once_without_input() {
    let mut idle = SceneHost::default();
    assert_eq!(wait(&mut idle, 1), (Step::Continue, 1, 0));
    let mut scene = scene_of(&quake_code(0x201, 40, 360, 3), 1);
    assert_eq!(wait(&mut scene, 1), (Step::Yield, 0, 4));
    assert!(scene.quake().is_some());
    assert_eq!(wait(&mut scene, 0), (Step::Yield, 1, 0));
    assert!(scene.quake().is_none());
}

#[test]
fn unmodeled_waveforms_bad_arguments_and_fades_under_offsets_are_errors() {
    for code in [
        quake_code(0x20, 3, 10, 1),
        quake_code(0x100, 3, 10, 1),
        quake_code(0x300, 3, 10, 1),
        {
            let mut short = vec![];
            command(&mut short, 0x2225, &[0, 3, 10]);
            short
        },
    ] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        pending(&mut vm);
        let (command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        assert!(scene.try_handle_quake(&mut vm).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert!(scene.quake().is_none());
    }
    // A colour fade is a layer 0 item whose shaken extent is unverified.
    let mut code = vec![];
    command(&mut code, 0x2220, &[0, 0, 100, 0]);
    code.extend(quake_code(0x01, 1, 10, u32::MAX));
    let mut scene = scene_of(&code, 2);
    assert!(scene_render::render(&scene, 4, 1).is_ok());
    scene.advance_frame(10);
    assert!(scene_render::render(&scene, 4, 1).is_err());
}

#[test]
fn pause_query_reports_a_paused_quake_only() {
    let mut code = quake_code(0, 4, 10, u32::MAX);
    command(&mut code, 0x220c, &[]);
    command(&mut code, 0x2227, &[1]);
    command(&mut code, 0x220c, &[]);
    command(&mut code, 0x2227, &[0]);
    command(&mut code, 0x220c, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let mut results = vec![];
    for _ in 0..6 {
        run(&mut vm, &mut scene, 1);
        results.push(vm.state().system_integer(0));
    }
    assert_eq!(results[1], Some(0));
    assert_eq!(results[3], Some(1));
    assert_eq!(results[5], Some(0));
    assert_eq!(vm.stack().len_bytes(), 0);
}

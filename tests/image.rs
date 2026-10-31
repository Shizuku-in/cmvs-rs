use cmvs_rs::{
    cpz::{Cpz6Index, data_checksum},
    fade::SkipGate,
    image::{DecodedImage, ImageArchives, ImageSource, decode},
    input::ActionInput,
    machine::{Machine, Step},
    message::MessageHost,
    ps3::Script,
    raster::RgbaImage,
    scene::{ObjectImage, SceneHost},
};
use std::{collections::BTreeMap, error::Error, fs, io::Cursor, path::PathBuf};

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
/// 2x1 PB3 of all-solid 1x1 tiles; planes are stored B, G, R and optionally A.
fn pb3(kind: u8, planes: &[[u8; 2]]) -> Vec<u8> {
    let n = planes.len();
    let mut b = vec![0; 52];
    b[..4].copy_from_slice(b"PB3B");
    put(&mut b, 8, 1);
    put(&mut b, 24, 1);
    b[28] = kind;
    b[30] = 2;
    b[32] = 1;
    b[34] = (n * 8) as u8;
    put(&mut b, 44, 52);
    put(&mut b, 48, (52 + 19 * n) as u32);
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
fn opaque() -> Vec<u8> {
    pb3(1, &[[1, 2], [3, 4], [5, 6]])
}
fn opaque_pixels() -> RgbaImage {
    RgbaImage::new(2, 1, vec![5, 3, 1, 255, 6, 4, 2, 255]).unwrap()
}

fn script(code: &[u8], text: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
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
fn push(code: &mut Vec<u8>, value: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(value.to_le_bytes());
}
fn op(code: &mut Vec<u8>, opcode: u16) {
    code.extend(opcode.to_le_bytes());
}
/// 0x2020 builds `slot`, then 0x2030 loads each text offset into it.
fn loads(slot: u32, names: &[u32]) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, slot);
    op(&mut code, 0x2020);
    for &name in names {
        push(&mut code, name);
        push(&mut code, slot);
        op(&mut code, 0x2030);
    }
    code
}
fn run_to_system(vm: &mut Machine<'_>) -> u16 {
    loop {
        if let Step::System { command, .. } = vm.step().unwrap() {
            return command;
        }
    }
}

#[derive(Default)]
struct Files(BTreeMap<Vec<u8>, Vec<u8>>);
impl ImageSource for Files {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(self.0.get(name).cloned())
    }
}

fn replacement_setup(code: &mut Vec<u8>, duration: u32, manager: u32) {
    code.extend(loads(20, &[0]));
    scene_command(code, 0x2040, &[20]);
    scene_command(code, 0x2044, &[20, u32::MAX, 0, 0, 2, 1]);
    scene_command(code, 0x2045, &[20, u32::MAX, 10, 20]);
    scene_command(code, 0x21ea, &[20, u32::MAX, 200]);
    scene_command(code, 0x21f9, &[20, u32::MAX]);
    scene_command(code, 0x2022, &[20, 0]);
    scene_command(code, 0x2050, &[20, 0]);
    scene_command(code, 0x22c4, &[20, 6, duration, manager]);
}

fn prepare_replacement<'a>(vm: &mut Machine<'a>, scene: &mut SceneHost, files: &mut Files) {
    for _ in 0..9 {
        let command = run_to_system(vm);
        if command == 0x30 {
            assert!(scene.try_handle_image(vm, files).unwrap());
        } else {
            assert!(scene.try_handle(vm).unwrap());
        }
    }
    assert_eq!(run_to_system(vm), 0x2c4);
}

#[test]
fn replacement_retains_old_children_moves_effects_and_finishes_on_frame_time() {
    let mut code = vec![];
    replacement_setup(&mut code, 500, 0);
    scene_command(&mut code, 0x2045, &[20, u32::MAX, 30, 40]);
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files(BTreeMap::from([
        (b"a.pb3".to_vec(), opaque()),
        (
            b"b.pb3".to_vec(),
            pb3(1, &[[9, 10], [11, 12], [13, 14], [255, 128]]),
        ),
    ]));
    prepare_replacement(&mut vm, &mut scene, &mut files);
    let mut previous = scene.object(20).unwrap().clone();
    vm.state_mut().set_system_integer(1, 789).unwrap();
    assert!(scene.try_change_image(&mut vm, &mut files).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.state().system_integer(1), Some(789));
    let object = scene.object(20).unwrap();
    let change = object.change.as_ref().unwrap();
    assert_eq!((change.duration, change.elapsed), (500, 0));
    previous.draw.as_mut().unwrap().opacity_track = None;
    previous.draw.as_mut().unwrap().world_motion = None;
    assert_eq!(&*change.previous, &previous);
    assert!(object.draw.as_ref().unwrap().opacity_track.is_some());
    assert!(object.draw.as_ref().unwrap().world_motion.is_some());
    assert_eq!(object.draw.as_ref().unwrap().words[21], 0);
    assert!(object.children.is_empty());
    assert_eq!(object.flags, 0x7fffffff);
    assert_eq!(
        change.common.image.as_ref().unwrap().pixels.pixels(),
        &[13, 11, 9, 255, 0, 0, 0, 0]
    );
    assert!(cmvs_rs::scene_render::render(&scene, 32, 32).is_ok());
    assert_eq!(run_to_system(&mut vm), 0x45);
    scene.try_handle(&mut vm).unwrap();
    scene.advance_frame(250);
    let object = scene.object(20).unwrap();
    let change = object.change.as_ref().unwrap();
    for retained in [&change.previous, &change.common] {
        assert_eq!(&retained.draw.as_ref().unwrap().words[8..10], &[30, 40]);
    }
    assert_eq!(object.draw.as_ref().unwrap().words[21], 128);
    assert_eq!(change.previous.draw.as_ref().unwrap().words[21], 128);
    scene.advance_frame(249);
    assert!(scene.object(20).unwrap().change.is_some());
    scene.advance_frame(1);
    let object = scene.object(20).unwrap();
    assert!(object.change.is_none());
    assert_eq!(object.draw.as_ref().unwrap().words[21], 256);
    assert_eq!(object.image.as_ref().unwrap().name, b"b.pb3");
    assert!(cmvs_rs::scene_render::render(&scene, 32, 32).is_ok());
}

#[test]
fn immediate_and_repeated_replacements_drop_retained_objects_without_cloning_effects() {
    for duration in [0, 500] {
        let mut code = vec![];
        replacement_setup(&mut code, duration, 0);
        scene_command(&mut code, 0x22c4, &[20, 0, 500, 0]);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = Files(BTreeMap::from([
            (b"a.pb3".to_vec(), opaque()),
            (b"b.pb3".to_vec(), opaque()),
        ]));
        prepare_replacement(&mut vm, &mut scene, &mut files);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        assert_eq!(scene.object(20).unwrap().change.is_some(), duration != 0);
        scene.advance_frame(100);
        assert_eq!(run_to_system(&mut vm), 0x2c4);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        let object = scene.object(20).unwrap();
        let previous = &object.change.as_ref().unwrap().previous;
        assert!(previous.change.is_none());
        assert!(previous.children.is_empty());
        assert_eq!(previous.draw.as_ref().unwrap().words[21], 256);
        assert!(previous.draw.as_ref().unwrap().opacity_track.is_none());
        assert!(previous.draw.as_ref().unwrap().world_motion.is_none());
        assert!(object.draw.as_ref().unwrap().opacity_track.is_some());
        assert!(object.draw.as_ref().unwrap().world_motion.is_some());
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn replacement_failures_preserve_pending_command_scene_stack_and_results() {
    for manager in [0, 1] {
        for bytes in [None, Some(b"unknown image codec".to_vec())] {
            let mut code = vec![];
            replacement_setup(&mut code, 500, manager);
            let program = script(&code, b"a.pb3\0b.pb3\0");
            let mut vm = Machine::new(&program).unwrap();
            let mut scene = SceneHost::default();
            let mut files = Files(BTreeMap::from([(b"a.pb3".to_vec(), opaque())]));
            prepare_replacement(&mut vm, &mut scene, &mut files);
            if let Some(bytes) = bytes {
                files.0.insert(b"b.pb3".to_vec(), bytes);
            }
            let old = scene.object(20).unwrap().clone();
            let pending = vm.pending_system();
            vm.state_mut().set_system_integer(0, 123).unwrap();
            for _ in 0..2 {
                assert!(scene.try_change_image(&mut vm, &mut files).is_err());
                assert_eq!(vm.pending_system(), pending);
                assert_eq!(vm.stack().len_bytes(), 16);
                assert_eq!(vm.state().system_integer(0), Some(123));
                assert_eq!(scene.object(20), Some(&old));
            }
        }
    }
    for args in [vec![256, 0, 0, 0], vec![u32::MAX, 0, 0, 0], vec![0, 0, 0]] {
        let mut code = vec![];
        scene_command(&mut code, 0x22c4, &args);
        let program = script(&code, b"a.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        run_to_system(&mut vm);
        assert!(
            SceneHost::default()
                .try_change_image(&mut vm, &mut Files::default())
                .is_err()
        );
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x2c4)));
    }
}

#[test]
fn missing_object_returns_false_without_reading_a_replacement() {
    let mut code = vec![];
    scene_command(&mut code, 0x22c4, &[255, 0, 500, 1]);
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    run_to_system(&mut vm);
    let mut scene = SceneHost::default();
    assert!(
        scene
            .try_change_image(&mut vm, &mut Files::default())
            .unwrap()
    );
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert!(scene.objects().is_empty());
}

#[test]
fn decode_dispatches_on_magic_and_pb3_type_like_0x140036700() {
    let decoded = decode(&opaque()).unwrap().unwrap();
    assert_eq!(
        decoded,
        DecodedImage {
            pixels: opaque_pixels(),
            alpha: false
        }
    );
    let alpha = decode(&pb3(1, &[[1, 2], [3, 4], [5, 6], [7, 8]]))
        .unwrap()
        .unwrap();
    assert!(alpha.alpha);
    assert_eq!(alpha.pixels.pixels(), [5, 3, 1, 7, 6, 4, 2, 8]);
    for kind in [2, 3, 6, 8] {
        assert!(decode(&pb3(kind, &[[0; 2]; 3])).is_err(), "type {kind}");
    }
    for kind in [0, 9] {
        assert_eq!(
            decode(&pb3(kind, &[[0; 2]; 3])).unwrap(),
            None,
            "type {kind}"
        );
    }
    for magic in [&b"\x89PNG"[..], b"BMxx", b"MSK0", b"PB2A"] {
        assert!(decode(magic).is_err());
    }
    assert_eq!(decode(b"XYZW").unwrap(), None);
    assert!(decode(b"PB3").is_err());
}

#[test]
fn image_load_sets_result_and_rebuilds_the_object_image() {
    let text = b"a.pb3\0bad.pb3\0missing.pb3\0";
    let program = script(&loads(3, &[0, 6, 0, 14]), text);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    files
        .0
        .insert(b"bad.pb3".to_vec(), b"XYZW unknown magic".to_vec());
    assert_eq!(run_to_system(&mut vm), 0x20);
    assert!(!scene.try_handle_image(&mut vm, &mut files).unwrap());
    scene.try_handle(&mut vm).unwrap();
    let expected = ObjectImage {
        filter: None,
        name: b"a.pb3".to_vec(),
        pixels: opaque_pixels(),
        alpha: false,
        surface: false,
    };
    // A quiet failure destroys the previous image and reports 0.
    for (result, image) in [(1, Some(&expected)), (0, None), (1, Some(&expected))] {
        assert_eq!(run_to_system(&mut vm), 0x30);
        assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(result));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(scene.object(3).unwrap().image.as_ref(), image);
    }
    // Not found shows a modal MessageBox in the original: explicit error, nothing changes.
    assert_eq!(run_to_system(&mut vm), 0x30);
    assert!(scene.try_handle_image(&mut vm, &mut files).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x30));
    assert_eq!(vm.stack().len_bytes(), 8);
    assert_eq!(scene.object(3).unwrap().image.as_ref(), Some(&expected));
}

#[test]
fn filtered_image_load_scales_rgb_with_saturation_and_preserves_alpha_and_object() {
    let mut code = loads(20, &[0]);
    scene_command(&mut code, 0x2040, &[20]);
    scene_command(&mut code, 0x2045, &[20, u32::MAX, 30, 40]);
    scene_command(&mut code, 0x2022, &[20, 0]);
    scene_command(&mut code, 0x21ef, &[20, u32::MAX]);
    scene_command(&mut code, 0x203b, &[20, 0, 2, 0x55ff8000, u32::MAX]);
    scene_command(&mut code, 0x2030, &[20, 0]);
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(
        b"a.pb3".to_vec(),
        pb3(1, &[[200, 255], [128, 1], [240, 64], [0, 17]]),
    );
    for _ in 0..6 {
        run_to_system(&mut vm);
        if !scene.try_handle(&mut vm).unwrap() {
            scene.try_handle_image(&mut vm, &mut files).unwrap();
        }
    }
    let mut expected = scene.object(20).unwrap().clone();
    run_to_system(&mut vm);
    assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    let image = scene.object(20).unwrap().image.as_ref().unwrap();
    assert_eq!(image.name, b"a.pb3");
    assert!(image.alpha);
    assert_eq!(image.pixels.pixels(), &[255, 128, 0, 0, 127, 1, 0, 17]);
    expected.image = Some(image.clone());
    assert_eq!(scene.object(20), Some(&expected));
    // A normal reload reads original bytes, rather than filtering the old pixels again.
    run_to_system(&mut vm);
    scene.try_handle_image(&mut vm, &mut files).unwrap();
    assert_eq!(
        scene
            .object(20)
            .unwrap()
            .image
            .as_ref()
            .unwrap()
            .pixels
            .pixels(),
        &[240, 128, 200, 0, 64, 1, 255, 17]
    );
}

#[test]
fn filtered_image_load_uses_128_as_identity_and_quiet_failure_clears_image() {
    let mut code = loads(20, &[0]);
    for args in [
        [20, 0, 2, 0x808080, 0],
        [20, 0, 2, 0, 0],
        [20, 6, 2, 0x808080, 0],
    ] {
        scene_command(&mut code, 0x203b, &args);
    }
    let program = script(&code, b"a.pb3\0bad\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    files.0.insert(b"bad".to_vec(), b"XYZW".to_vec());
    run_to_system(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    for expected in [
        Some(opaque_pixels()),
        Some(opaque_pixels()),
        Some(RgbaImage::new(2, 1, vec![0, 0, 0, 255, 0, 0, 0, 255]).unwrap()),
        None,
    ] {
        run_to_system(&mut vm);
        scene.try_handle_image(&mut vm, &mut files).unwrap();
        assert_eq!(
            vm.state().system_integer(0),
            Some(u32::from(expected.is_some()))
        );
        assert_eq!(
            scene
                .object(20)
                .unwrap()
                .image
                .as_ref()
                .map(|im| &im.pixels),
            expected.as_ref()
        );
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn filtered_image_load_rejects_unknown_modes_missing_args_and_resource_failures_transactionally() {
    for args in [
        vec![20, 0, 1, 0x808080, 0],
        vec![20, 0, 2, 0x808080],
        vec![20, 6, 2, 0x808080, 0],
        vec![20, 14, 2, 0x808080, 0],
        vec![256, 0, 2, 0x808080, 0],
        vec![21, 0, 2, 0x808080, 0],
    ] {
        let mut code = loads(20, &[0]);
        scene_command(&mut code, 0x203b, &args);
        let program = script(&code, b"a.pb3\0png.pb3\0missing\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = Files::default();
        files.0.insert(b"a.pb3".to_vec(), opaque());
        files.0.insert(b"png.pb3".to_vec(), b"\x89PNG".to_vec());
        run_to_system(&mut vm);
        scene.try_handle(&mut vm).unwrap();
        run_to_system(&mut vm);
        scene.try_handle_image(&mut vm, &mut files).unwrap();
        let old = scene.object(20).unwrap().clone();
        run_to_system(&mut vm);
        let pending = vm.pending_system();
        for _ in 0..2 {
            assert!(scene.try_handle_image(&mut vm, &mut files).is_err());
            assert_eq!(scene.object(20), Some(&old));
            assert_eq!(vm.pending_system(), pending);
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            assert_eq!(vm.state().system_integer(0), Some(1));
        }
    }
}

#[test]
fn replacement_inherits_filter_and_filters_new_source_once_before_common_area() {
    for duration in [0, 100] {
        let mut code = loads(20, &[0]);
        scene_command(&mut code, 0x2040, &[20]);
        scene_command(&mut code, 0x203b, &[20, 0, 2, 0x404080, 55]);
        scene_command(&mut code, 0x22c4, &[20, 6, duration, 0]);
        scene_command(&mut code, 0x22c4, &[20, 0, 0, 0]);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = Files::default();
        files.0.insert(b"a.pb3".to_vec(), opaque());
        files.0.insert(
            b"b.pb3".to_vec(),
            pb3(1, &[[100, 200], [128, 255], [255, 128], [10, 255]]),
        );
        for _ in 0..4 {
            run_to_system(&mut vm);
            if !scene.try_handle(&mut vm).unwrap() {
                scene.try_handle_image(&mut vm, &mut files).unwrap();
            }
        }
        run_to_system(&mut vm);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        let object = scene.object(20).unwrap();
        let image = object.image.as_ref().unwrap();
        assert_eq!(image.filter, Some([2, 0x404080, 55]));
        assert_eq!(
            image.pixels.pixels(),
            &[127, 64, 100, 10, 64, 127, 200, 255]
        );
        if duration > 0 {
            let change = object.change.as_ref().unwrap();
            assert_eq!(change.previous.image.as_ref().unwrap().filter, image.filter);
            let common = change.common.image.as_ref().unwrap();
            assert_eq!(common.filter, None);
            assert_eq!(common.pixels.pixels(), &[0, 0, 0, 0, 64, 127, 200, 255]);
        } else {
            assert!(object.change.is_none());
        }
        run_to_system(&mut vm);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        let image = scene.object(20).unwrap().image.as_ref().unwrap();
        assert_eq!(image.filter, Some([2, 0x404080, 55]));
        assert_eq!(image.pixels.pixels(), &[2, 1, 1, 255, 3, 2, 2, 255]);
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn image_load_rejects_unmodeled_slots_codecs_and_long_names() {
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    files.0.insert(b"png.pb3".to_vec(), b"\x89PNG....".to_vec());
    let long = [b'x'; 64];
    files.0.insert(long.to_vec(), opaque());
    let mut text = b"a.pb3\0png.pb3\0".to_vec();
    text.extend(long);
    text.push(0);
    for (slot, built, name) in [(4, false, 0), (256, false, 0), (3, true, 6), (3, true, 14)] {
        let mut code = vec![];
        if built {
            push(&mut code, slot);
            op(&mut code, 0x2020);
        }
        push(&mut code, name);
        push(&mut code, slot);
        op(&mut code, 0x2030);
        let program = script(&code, &text);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        if built {
            run_to_system(&mut vm);
            scene.try_handle(&mut vm).unwrap();
        }
        assert_eq!(run_to_system(&mut vm), 0x30);
        assert!(scene.try_handle_image(&mut vm, &mut files).is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x30));
        assert_eq!(vm.state().system_integer(0), None);
        assert!(
            scene
                .objects()
                .values()
                .all(|object| object.image.is_none())
        );
    }
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("cmvs-image-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
fn registered(entries: &[(u32, &[u8])]) -> BTreeMap<(u16, u32), Vec<u8>> {
    entries
        .iter()
        .map(|&(slot, path)| ((0x2011, slot), path.to_vec()))
        .collect()
}

#[test]
fn registered_entries_are_searched_in_slot_order_and_misses_skip() {
    let root = temp_root("order");
    for (dir, name, bytes) in [
        ("a", "x.pb3", "from a"),
        ("b", "x.pb3", "from b"),
        ("b", "y.pb3", "only b"),
        ("c", "z.pb3", "other command"),
    ] {
        fs::create_dir_all(root.join(dir)).unwrap();
        fs::write(root.join(dir).join(name), bytes).unwrap();
    }
    let mut paths = registered(&[(0, b"missing.cpz"), (1, b""), (2, b"a\\"), (5, b"b\\")]);
    paths.insert((0x2010, 0), b"c\\".to_vec());
    let mut archives = ImageArchives::new(&root).unwrap();
    let mut source = archives.with_paths(&paths);
    assert_eq!(source.read(b"x.pb3").unwrap().unwrap(), b"from a");
    assert_eq!(source.read(b"y.pb3").unwrap().unwrap(), b"only b");
    assert_eq!(source.read(b"z.pb3").unwrap(), None);
    let escaping = registered(&[(0, b"a\\"), (9, b"..\\")]);
    let mut source = archives.with_paths(&escaping);
    assert_eq!(source.read(b"x.pb3").unwrap().unwrap(), b"from a");
    assert!(source.read(b"z.pb3").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_entries_fold_case_verify_checksums_and_reopen_on_reregistration() {
    let root = temp_root("archive");
    let bytes: Vec<u8> = include_str!("fixtures/cpz6-multiple.hex")
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    fs::write(root.join("zero.cpz"), &bytes).unwrap();
    // Make the first entry's encrypted bytes sum to its stored zero checksum.
    let index = Cpz6Index::read(&mut Cursor::new(&bytes)).unwrap();
    let entry = &index.entries()[0];
    let start = entry.offset as usize;
    let sum = data_checksum(&bytes[start..start + entry.size as usize]);
    let mut fixed = bytes.clone();
    let first = u32::from_le_bytes(fixed[start..start + 4].try_into().unwrap());
    fixed[start..start + 4].copy_from_slice(&first.wrapping_sub(sum).to_le_bytes());
    fs::write(root.join("fixed.cpz"), &fixed).unwrap();
    let expected = index.read_entry(&mut Cursor::new(&fixed), 0).unwrap();

    let mut archives = ImageArchives::new(&root).unwrap();
    let zero = registered(&[(0, b"zero.cpz")]);
    let error = archives.with_paths(&zero).read(b"FIXTURE.BIN").unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"));
    let fixed_paths = registered(&[(0, b"fixed.cpz")]);
    let mut source = archives.with_paths(&fixed_paths);
    assert_eq!(source.read(b"Fixture.Bin").unwrap().unwrap(), expected);
    assert_eq!(source.read(b"absent.bin").unwrap(), None);
    assert!(source.read(&[b'a'; 64]).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn scene_command(code: &mut Vec<u8>, opcode: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        push(code, arg);
    }
    op(code, opcode);
}
fn build_scene(code: &[u8]) -> SceneHost {
    let program = script(code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    while vm.pc() < code.len() {
        run_to_system(&mut vm);
        assert!(
            scene.try_handle(&mut vm).unwrap()
                || scene.try_handle_image(&mut vm, &mut files).unwrap()
        );
    }
    scene
}

#[test]
fn child_image_loading_uses_child_argument_and_preserves_parent_image() {
    let mut code = loads(3, &[0]);
    scene_command(&mut code, 0x2022, &[3, 7]);
    scene_command(&mut code, 0x2038, &[3, 7, 0]);
    let scene = build_scene(&code);
    let parent = scene.object(3).unwrap();
    assert_eq!(parent.image, parent.children[&7].image);
    assert!(parent.image.is_some());
    // Without 0x2040 the parent is never enqueued, so its children are not visited either.
    assert_eq!(
        cmvs_rs::scene_render::render(&scene, 2, 1)
            .unwrap()
            .pixels(),
        &[0, 0, 0, 255, 0, 0, 0, 255]
    );
}

#[test]
fn static_scene_uses_script_crop_origin_position_and_clear_color() {
    let mut code = loads(3, &[0]);
    // Loading alone is invisible.
    let scene = build_scene(&code);
    assert_eq!(
        cmvs_rs::scene_render::render(&scene, 2, 1)
            .unwrap()
            .pixels(),
        &[0, 0, 0, 255, 0, 0, 0, 255]
    );
    scene_command(&mut code, 0x22cb, &[0x102030]);
    scene_command(&mut code, 0x2040, &[3]);
    scene_command(&mut code, 0x2044, &[3, u32::MAX, 0, 0, 2, 1]);
    scene_command(&mut code, 0x2046, &[3, u32::MAX, 1, 0]);
    scene_command(&mut code, 0x2045, &[3, u32::MAX, 0, 0]);
    let scene = build_scene(&code);
    let frame = cmvs_rs::scene_render::render(&scene, 2, 1).unwrap();
    assert_eq!(&frame.pixels()[..4], &opaque_pixels().pixels()[4..]);
    assert_eq!(&frame.pixels()[4..], &[0x10, 0x20, 0x30, 255]);
    assert!(cmvs_rs::scene_render::render(&scene, u32::MAX, u32::MAX).is_err());
}

#[test]
fn texture_subtract_keeps_texel_alpha_opacity_and_device_blending() {
    use cmvs_rs::{
        scene::{DrawState, SceneObject},
        scene_render,
    };
    let scene = build_scene(&{
        let mut code = vec![];
        scene_command(&mut code, 0x22cb, &[0x806040]);
        code
    });
    for (alpha_image, blend, opacity, expected) in [
        (true, 1, 255, [64, 48, 32]),
        (true, 1, 128, [96, 72, 48]),
        (true, 2, 255, [128, 96, 64]),
        (false, 2, 255, [0, 0, 0]),
    ] {
        let mut draw = DrawState {
            texture_subtract: 1,
            ..DrawState::default()
        };
        draw.words[1] = blend;
        draw.words[4..6].copy_from_slice(&[2, 1]);
        draw.words[17] = opacity;
        let object = SceneObject {
            image: Some(ObjectImage {
                name: vec![],
                filter: None,
                alpha: alpha_image,
                surface: false,
                pixels: RgbaImage::new(2, 1, vec![200, 120, 80, 128, 255, 255, 255, 0]).unwrap(),
            }),
            draw: Some(draw),
            ..SceneObject::default()
        };
        let frame = scene_render::render_with_objects(&scene, &[&object], 2, 1).unwrap();
        assert_eq!(&frame.pixels()[..3], &expected);
        assert_eq!(
            &frame.pixels()[4..7],
            if alpha_image {
                &[128, 96, 64]
            } else {
                &[0, 0, 0]
            }
        );
    }
}

#[test]
fn texture_subtract_is_local_to_each_draw_including_inherited_children() {
    for (parent, child, expected) in [
        (1, 0, vec![5, 3, 1, 255, 0, 0, 0, 255]),
        (0, 1, vec![0, 0, 0, 255, 6, 4, 2, 255]),
    ] {
        let mut code = loads(1, &[0]);
        for (op, args) in [
            (0x2040, vec![1]),
            (0x2044, vec![1, u32::MAX, 0, 0, 2, 1]),
            (0x2022, vec![1, 2]),
            (0x2050, vec![1, 2]),
            (0x2044, vec![1, 2, 0, 0, 1, 1]),
            (0x2322, vec![1, u32::MAX, parent]),
            (0x2322, vec![1, 2, child]),
        ] {
            scene_command(&mut code, op, &args);
        }
        let frame = cmvs_rs::scene_render::render(&build_scene(&code), 2, 1).unwrap();
        assert_eq!(frame.pixels(), expected);
    }
}

#[test]
fn texture_subtract_survives_immediate_and_crossfade_replacements() {
    for duration in [0, 100] {
        let mut code = loads(20, &[0]);
        scene_command(&mut code, 0x2040, &[20]);
        scene_command(&mut code, 0x2322, &[20, u32::MAX, 7]);
        scene_command(&mut code, 0x22c4, &[20, 6, duration, 0]);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = Files(BTreeMap::from([
            (b"a.pb3".to_vec(), opaque()),
            (b"b.pb3".to_vec(), opaque()),
        ]));
        for _ in 0..4 {
            run_to_system(&mut vm);
            assert!(
                scene.try_handle(&mut vm).unwrap()
                    || scene.try_handle_image(&mut vm, &mut files).unwrap()
            );
        }
        assert_eq!(run_to_system(&mut vm), 0x2c4);
        assert!(scene.try_change_image(&mut vm, &mut files).unwrap());
        let object = scene.object(20).unwrap();
        assert_eq!(object.draw.as_ref().unwrap().texture_subtract, 1);
        if let Some(change) = &object.change {
            for retained in [&change.previous, &change.common] {
                assert_eq!(retained.draw.as_ref().unwrap().texture_subtract, 1);
            }
        }
    }
}

#[test]
fn static_scene_sorts_back_to_front_and_equal_orders_keep_slot_order() {
    for second_order in [10, 0, 20] {
        let mut code = vec![];
        for (slot, crop, order) in [(1, 0, 10), (2, 1, second_order)] {
            code.extend(loads(slot, &[0]));
            scene_command(&mut code, 0x2040, &[slot]);
            scene_command(&mut code, 0x2044, &[slot, u32::MAX, crop, 0, 1, 1]);
            scene_command(&mut code, 0x2047, &[slot, u32::MAX, order]);
        }
        let frame = cmvs_rs::scene_render::render(&build_scene(&code), 1, 1).unwrap();
        let pixels = opaque_pixels();
        assert_eq!(
            frame.pixels(),
            if second_order > 10 {
                &pixels.pixels()[..4]
            } else {
                &pixels.pixels()[4..]
            }
        );
    }
}

#[test]
fn static_scene_opacity_and_unsupported_blend_are_explicit() {
    let mut code = loads(1, &[0]);
    scene_command(&mut code, 0x2040, &[1]);
    scene_command(&mut code, 0x2044, &[1, u32::MAX, 0, 0, 2, 1]);
    scene_command(&mut code, 0x2048, &[1, u32::MAX, 128]);
    let frame = cmvs_rs::scene_render::render(&build_scene(&code), 2, 1).unwrap();
    for (actual, source) in frame
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(opaque_pixels().pixels().as_chunks::<4>().0.iter())
    {
        for c in 0..3 {
            assert_eq!(actual[c], ((u32::from(source[c]) * 128 + 127) / 255) as u8);
        }
    }
    // Modes 0 and 3 are unused by the title and stay unverified.
    for mode in [0, 3] {
        let mut code = code.clone();
        scene_command(&mut code, 0x204b, &[1, u32::MAX, mode]);
        assert!(cmvs_rs::scene_render::render(&build_scene(&code), 2, 1).is_err());
    }
}

/// The 2x1 opaque image in slot 1 with blend mode 2 and `opacity`, over `clear`.
fn additive(clear: u32, opacity: u32) -> Vec<u8> {
    let mut code = vec![];
    scene_command(&mut code, 0x22cb, &[clear]);
    code.extend(loads(1, &[0]));
    scene_command(&mut code, 0x2040, &[1]);
    scene_command(&mut code, 0x2044, &[1, u32::MAX, 0, 0, 2, 1]);
    scene_command(&mut code, 0x204b, &[1, u32::MAX, 2]);
    scene_command(&mut code, 0x2048, &[1, u32::MAX, opacity]);
    code
}

#[test]
fn additive_blend_adds_scaled_texels_and_opaque_images_at_full_opacity_replace() {
    let source = opaque_pixels();
    let frame =
        cmvs_rs::scene_render::render(&build_scene(&additive(0x40_4040, 128)), 2, 1).unwrap();
    for (actual, source) in frame
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(source.pixels().as_chunks::<4>().0.iter())
    {
        for c in 0..3 {
            let added = (u32::from(source[c]) * 128 + 127) / 255;
            assert_eq!(u32::from(actual[c]), 0x40 + added);
        }
    }
    // Without an alpha channel at full opacity 0x140020340 selects device mode 0.
    let frame =
        cmvs_rs::scene_render::render(&build_scene(&additive(0x40_4040, 255)), 2, 1).unwrap();
    assert_eq!(frame.pixels(), source.pixels());
    // The 8-bit target saturates.
    let frame =
        cmvs_rs::scene_render::render(&build_scene(&additive(0xfe_fefe, 254)), 2, 1).unwrap();
    assert!(frame.pixels().iter().all(|&v| v == 255));
    // An image with an alpha channel stays additive at full opacity.
    let mut draw = cmvs_rs::scene::DrawState::default();
    draw.words[1] = 2;
    draw.words[4..6].copy_from_slice(&[1, 1]);
    let glow = cmvs_rs::scene::SceneObject {
        image: Some(ObjectImage {
            filter: None,
            name: vec![],
            pixels: RgbaImage::new(1, 1, vec![10, 20, 30, 255]).unwrap(),
            alpha: true,
            surface: false,
        }),
        draw: Some(draw),
        ..cmvs_rs::scene::SceneObject::default()
    };
    let scene = build_scene(&additive(0x40_4040, 0));
    let frame = cmvs_rs::scene_render::render_with_objects(&scene, &[&glow], 1, 1).unwrap();
    assert_eq!(&frame.pixels()[..3], &[0x4a, 0x54, 0x5e]);
}

#[test]
fn fade_joins_object_order_and_equal_keys_follow_slot_objects() {
    for (order, covered) in [(11, false), (10, true), (9, true)] {
        let mut code = loads(1, &[0]);
        scene_command(&mut code, 0x2040, &[1]);
        scene_command(&mut code, 0x2044, &[1, u32::MAX, 0, 0, 2, 1]);
        scene_command(&mut code, 0x2047, &[1, u32::MAX, 10]);
        scene_command(&mut code, 0x2220, &[1, 0, 2000, order]);
        let scene = build_scene(&code);
        let frame = cmvs_rs::scene_render::render(&scene, 2, 1).unwrap();
        if covered {
            assert_eq!(frame.pixels(), &[0, 0, 0, 255, 0, 0, 0, 255]);
        } else {
            assert_eq!(frame, opaque_pixels());
        }
    }
}

/// Parent slot 1 shows image pixel 0 at `parent_x` (order 10); child 5 shows pixel 1 at
/// `child_x`. Creating the child before 0x2040 leaves its inheritance flags clear.
fn parent_and_child(early_child: bool, parent_x: u32, child_x: u32, child_order: u32) -> Vec<u8> {
    let mut code = loads(1, &[0]);
    if early_child {
        scene_command(&mut code, 0x2022, &[1, 5]);
    }
    scene_command(&mut code, 0x2040, &[1]);
    if !early_child {
        scene_command(&mut code, 0x2022, &[1, 5]);
    }
    scene_command(&mut code, 0x2044, &[1, u32::MAX, 0, 0, 1, 1]);
    scene_command(&mut code, 0x2045, &[1, u32::MAX, parent_x, 0]);
    scene_command(&mut code, 0x2047, &[1, u32::MAX, 10]);
    scene_command(&mut code, 0x2050, &[1, 5]);
    scene_command(&mut code, 0x2044, &[1, 5, 1, 0, 1, 1]);
    scene_command(&mut code, 0x2045, &[1, 5, child_x, 0]);
    scene_command(&mut code, 0x2047, &[1, 5, child_order]);
    code
}
fn frame(code: &[u8]) -> Vec<u8> {
    cmvs_rs::scene_render::render(&build_scene(code), 2, 1)
        .unwrap()
        .pixels()
        .to_vec()
}
const P0: [u8; 4] = [5, 3, 1, 255];
const P1: [u8; 4] = [6, 4, 2, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 255];

#[test]
fn inheriting_children_borrow_the_parent_image_and_position_and_draw_after_it() {
    assert_eq!(frame(&parent_and_child(false, 0, 1, 0)), [P0, P1].concat());
    // The child position is relative to the parent's.
    assert_eq!(
        frame(&parent_and_child(false, 1, 1, 0)),
        [CLEAR, P0].concat()
    );
    // Sublist children follow their parent even with a lower order key.
    assert_eq!(
        frame(&parent_and_child(false, 0, 0, 0)),
        [P1, CLEAR].concat()
    );
}

#[test]
fn inheriting_children_multiply_the_parent_opacity() {
    let mut code = parent_and_child(false, 0, 1, 0);
    scene_command(&mut code, 0x2048, &[1, u32::MAX, 128]);
    let pixels = frame(&code);
    // 255 * 128 / 255 = 128 for both the parent and the child.
    for (actual, source) in pixels.as_chunks::<4>().0.iter().zip([P0, P1]) {
        for c in 0..3 {
            assert_eq!(actual[c], ((u32::from(source[c]) * 128 + 127) / 255) as u8);
        }
    }
    scene_command(&mut code, 0x2048, &[1, 5, u32::MAX]);
    assert!(cmvs_rs::scene_render::render(&build_scene(&code), 2, 1).is_err());
}

#[test]
fn children_without_inheritance_flags_are_independent_queue_entries() {
    // Absolute position: the child lands at x=0 although the parent is at x=1.
    assert_eq!(frame(&parent_and_child(true, 1, 0, 0)), [P1, P0].concat());
    // A higher order sorts the child before (below) the parent; a lower one after it.
    assert_eq!(
        frame(&parent_and_child(true, 1, 1, 20)),
        [CLEAR, P0].concat()
    );
    assert_eq!(
        frame(&parent_and_child(true, 1, 1, 0)),
        [CLEAR, P1].concat()
    );
}

#[test]
fn shown_children_without_draw_state_fail_explicitly() {
    let mut code = loads(1, &[0]);
    scene_command(&mut code, 0x2040, &[1]);
    scene_command(&mut code, 0x2022, &[1, 5]);
    assert!(cmvs_rs::scene_render::render(&build_scene(&code), 2, 1).is_err());
}

/// 0x215d builds window 7 (text object 10), then 0x2170 loads the text offset `name`.
fn window_load(name: u32) -> Vec<u8> {
    let mut code = vec![];
    for value in [720, 1280, 54, 1280, 0, 0, 10, 7] {
        push(&mut code, value);
    }
    op(&mut code, 0x215d);
    push(&mut code, name);
    push(&mut code, 7);
    op(&mut code, 0x2170);
    code
}

#[test]
fn window_image_load_replaces_the_window_object_image_without_a_result() {
    let script = script(&window_load(0), b"win.pb3\0");
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut files = Files::default();
    files.0.insert(b"win.pb3".to_vec(), opaque());
    let mut messages = MessageHost::default();
    run_to_system(&mut vm);
    assert!(messages.try_handle(&mut vm).unwrap());
    run_to_system(&mut vm);
    assert!(!messages.try_handle(&mut vm).unwrap());
    assert!(messages.try_handle_image(&mut vm, &mut files).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert_eq!(
        messages.window(7).unwrap().object.image,
        Some(ObjectImage {
            filter: None,
            name: b"win.pb3".to_vec(),
            pixels: opaque_pixels(),
            alpha: false,
            surface: false,
        })
    );
}

#[test]
fn window_image_failures_keep_the_command_pending() {
    for (contents, missing) in [(Some(b"XYZW".to_vec()), false), (None, true)] {
        let script = script(&window_load(0), b"win.pb3\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut files = Files::default();
        if let Some(contents) = contents {
            files.0.insert(b"win.pb3".to_vec(), contents);
        }
        let mut messages = MessageHost::default();
        run_to_system(&mut vm);
        messages.try_handle(&mut vm).unwrap();
        run_to_system(&mut vm);
        let bytes = vm.stack().len_bytes();
        assert!(
            messages.try_handle_image(&mut vm, &mut files).is_err(),
            "{missing}"
        );
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x170));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert!(messages.window(7).unwrap().object.image.is_none());
    }
}

#[test]
fn image_query_reports_size_and_depth_or_only_absence() {
    // Slot 4 loads the 2x1 opaque image, slot 5 has no image, slot 6 has no object.
    let mut code = loads(4, &[0]);
    push(&mut code, 5);
    op(&mut code, 0x2020);
    for slot in [4, 5, 6] {
        push(&mut code, slot);
        op(&mut code, 0x2033);
    }
    let script = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    let mut scene = SceneHost::default();
    for _ in 0..2 {
        run_to_system(&mut vm);
        if !scene.try_handle(&mut vm).unwrap() {
            assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
        }
    }
    run_to_system(&mut vm);
    assert!(scene.try_handle(&mut vm).unwrap());
    for slot in 1..4 {
        vm.state_mut().set_system_integer(slot, 77).unwrap();
    }
    run_to_system(&mut vm);
    assert!(scene.try_handle(&mut vm).unwrap());
    let results: Vec<_> = (0..4).map(|i| vm.state().system_integer(i)).collect();
    assert_eq!(results, [Some(1), Some(2), Some(1), Some(0)]);
    for slot in 1..4 {
        vm.state_mut().set_system_integer(slot, 77).unwrap();
    }
    run_to_system(&mut vm);
    assert!(scene.try_handle(&mut vm).unwrap());
    let results: Vec<_> = (0..4).map(|i| vm.state().system_integer(i)).collect();
    assert_eq!(results, [Some(0), Some(77), Some(77), Some(77)]);
    run_to_system(&mut vm);
    let bytes = vm.stack().len_bytes();
    assert!(scene.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x33));
    assert_eq!(vm.stack().len_bytes(), bytes);
}

struct CacheRoot(PathBuf);
impl CacheRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "cmvs-raw-image-cache-{name}-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let root = Self(path);
        fs::create_dir(root.0.join("raw")).unwrap();
        root
    }
}
impl Drop for CacheRoot {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("raw/a.pb3"));
        let _ = fs::remove_dir(self.0.join("raw"));
        let _ = fs::remove_dir(&self.0);
    }
}

#[test]
fn next_read_cache_hint_keeps_stack_results_and_active_images_and_only_affects_one_read() {
    let root = CacheRoot::new("hint");
    let first = opaque();
    let second = pb3(1, &[[8, 9], [10, 11], [12, 13]]);
    let path = root.0.join("raw/a.pb3");
    fs::write(&path, &first).unwrap();
    let paths = registered(&[(0, b"raw\\")]);
    let mut archives = ImageArchives::new(&root.0).unwrap();
    let mut code = loads(3, &[0]);
    push(&mut code, 0xdead_beef);
    for _ in 0..4 {
        op(&mut code, 0x203f);
    }
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    assert_eq!(run_to_system(&mut vm), 0x20);
    scene.try_handle(&mut vm).unwrap();
    assert_eq!(run_to_system(&mut vm), 0x30);
    scene
        .try_handle_image(&mut vm, &mut archives.with_paths(&paths))
        .unwrap();
    let before = scene.objects().clone();
    vm.state_mut().set_system_integer(0, 99).unwrap();
    vm.state_mut().set_system_integer(4, 123).unwrap();
    // Arming twice does not accumulate two reads or clear an active decoded image.
    for _ in 0..2 {
        assert_eq!(run_to_system(&mut vm), 0x3f);
        let pc = vm.pc();
        assert!(
            scene
                .try_handle_image(&mut vm, &mut archives.with_paths(&paths))
                .unwrap()
        );
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdead_beef);
    }
    assert_eq!(
        archives.with_paths(&paths).read(b"a.pb3").unwrap(),
        Some(first.clone())
    );
    fs::write(&path, &second).unwrap();
    // Unarmed reads bypass the cache and do not update its stored bytes.
    assert_eq!(
        archives.with_paths(&paths).read(b"a.pb3").unwrap(),
        Some(second)
    );
    assert_eq!(run_to_system(&mut vm), 0x3f);
    scene
        .try_handle_image(&mut vm, &mut archives.with_paths(&paths))
        .unwrap();
    assert_eq!(
        archives.with_paths(&paths).read(b"A.PB3").unwrap(),
        Some(first.clone())
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(archives.with_paths(&paths).read(b"a.pb3").unwrap(), None);
    assert_eq!(run_to_system(&mut vm), 0x3f);
    scene
        .try_handle_image(&mut vm, &mut archives.with_paths(&paths))
        .unwrap();
    assert_eq!(
        archives.with_paths(&paths).read(b"a.pb3").unwrap(),
        Some(first)
    );
    assert_eq!(scene.objects(), &before);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.state().system_integer(0), Some(99));
    assert_eq!(vm.state().system_integer(4), Some(123));
}

#[test]
fn next_read_cache_hint_is_consumed_by_missing_files_and_errors() {
    let root = CacheRoot::new("failure");
    let paths = registered(&[(0, b"raw\\")]);
    let mut archives = ImageArchives::new(&root.0).unwrap();
    for (name, error) in [
        (b"missing.pb3".as_slice(), false),
        (b"..\\outside.pb3", true),
    ] {
        archives.with_paths(&paths).cache_next_read().unwrap();
        let result = archives.with_paths(&paths).read(name);
        if error {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), None);
        }
        let path = root.0.join("raw/a.pb3");
        fs::write(&path, b"uncached").unwrap();
        assert_eq!(
            archives.with_paths(&paths).read(b"a.pb3").unwrap(),
            Some(b"uncached".to_vec())
        );
        fs::remove_file(&path).unwrap();
        archives.with_paths(&paths).cache_next_read().unwrap();
        assert_eq!(archives.with_paths(&paths).read(b"a.pb3").unwrap(), None);
    }
}

#[test]
fn unsupported_cache_source_keeps_command_pending_without_consuming_parameters() {
    struct Unsupported;
    impl ImageSource for Unsupported {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            panic!("0x203f does not read a file");
        }
    }
    let mut code = vec![];
    push(&mut code, 42);
    op(&mut code, 0x203f);
    let program = script(&code, b"");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    assert_eq!(run_to_system(&mut vm), 0x3f);
    assert!(!scene.try_handle(&mut vm).unwrap());
    assert!(scene.try_handle_image(&mut vm, &mut Unsupported).is_err());
    assert_eq!(vm.pending_system(), Some((8, 0x3f)));
    assert_eq!(vm.pc(), 8);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 42);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert!(scene.objects().is_empty());
}

fn change_files() -> Files {
    Files(BTreeMap::from([
        (b"a.pb3".to_vec(), opaque()),
        (b"b.pb3".to_vec(), opaque()),
    ]))
}

#[test]
fn replacement_query_and_cancel_keep_new_image_motion_and_other_results() {
    let mut code = vec![];
    replacement_setup(&mut code, 500, 0);
    for opcode in [0x22c6, 0x22c5, 0x22c6, 0x22c5] {
        scene_command(&mut code, opcode, &[20]);
    }
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = change_files();
    prepare_replacement(&mut vm, &mut scene, &mut files);
    scene.try_change_image(&mut vm, &mut files).unwrap();
    scene.advance_frame(100);
    let mut expected = scene.object(20).unwrap().clone();
    vm.state_mut().set_system_integer(1, 555).unwrap();
    assert_eq!(run_to_system(&mut vm), 0x2c6);
    assert!(scene.try_handle(&mut vm).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(
        scene.object(20),
        Some(&expected),
        "query does not advance time"
    );
    assert_eq!(run_to_system(&mut vm), 0x2c5);
    scene.try_handle(&mut vm).unwrap();
    expected.change = None;
    expected.draw.as_mut().unwrap().words[21] = 256;
    assert_eq!(scene.object(20), Some(&expected));
    assert_eq!(
        vm.state().system_integer(0),
        Some(1),
        "cancel preserves result 0"
    );
    assert_eq!(run_to_system(&mut vm), 0x2c6);
    scene.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(run_to_system(&mut vm), 0x2c5);
    scene.try_handle(&mut vm).unwrap();
    assert_eq!(scene.object(20), Some(&expected));
    assert_eq!(vm.state().system_integer(1), Some(555));
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
}

#[test]
fn replacement_wait_retries_without_advancing_time_then_completes_without_yield() {
    let mut code = vec![];
    replacement_setup(&mut code, 500, 0);
    scene_command(&mut code, 0x22c7, &[20, 0]);
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = change_files();
    prepare_replacement(&mut vm, &mut scene, &mut files);
    scene.try_change_image(&mut vm, &mut files).unwrap();
    assert_eq!(run_to_system(&mut vm), 0x2c7);
    let pc = vm.pc();
    let expected = scene.object(20).unwrap().clone();
    let mut input = ActionInput::default();
    input.set(0, true, true, true).unwrap();
    input.set(7, true, true, false).unwrap();
    let before_input = input.clone();
    let gate = SkipGate {
        effects: false,
        skip: true,
    };
    for repeat in 1..=3 {
        assert_eq!(
            scene.try_wait_change(&mut vm, gate, &mut input).unwrap(),
            Some(Step::Yield)
        );
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.repeat_count()),
            (pc, 8, repeat)
        );
        assert_eq!(scene.object(20), Some(&expected));
        assert_eq!(input, before_input, "non-skippable wait consumes no input");
        assert_eq!(vm.state().system_integer(0), Some(0));
    }
    scene.advance_frame(499);
    assert_eq!(
        scene.try_wait_change(&mut vm, gate, &mut input).unwrap(),
        Some(Step::Yield)
    );
    scene.advance_frame(1);
    assert_eq!(
        scene.try_wait_change(&mut vm, gate, &mut input).unwrap(),
        Some(Step::Continue)
    );
    assert_eq!(
        (vm.pc(), vm.stack().len_bytes(), vm.repeat_count()),
        (pc + 2, 0, 0)
    );
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(input, before_input);
    assert_eq!(
        scene.try_wait_change(&mut vm, gate, &mut input).unwrap(),
        None
    );
}

#[test]
fn replacement_wait_skip_flags_preserve_input_while_released_action_zero_is_consumed() {
    for trigger in 0..5 {
        let mut code = vec![];
        replacement_setup(&mut code, 500, 0);
        scene_command(&mut code, 0x22c7, &[20, 1]);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = change_files();
        prepare_replacement(&mut vm, &mut scene, &mut files);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        run_to_system(&mut vm);
        let mut input = ActionInput::default();
        input.set(0, true, true, true).unwrap();
        input.set(1, false, true, true).unwrap();
        if trigger == 2 || trigger == 3 {
            input.set(7 + (trigger - 2), true, true, true).unwrap();
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
            scene.try_wait_change(&mut vm, gate, &mut input).unwrap(),
            Some(Step::Yield)
        );
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
        assert!(
            vm.pending_system().is_none(),
            "skip yields after completing the command"
        );
        assert_eq!(vm.state().system_integer(0), Some(1));
        assert_eq!(input, expected_input);
        let object = scene.object(20).unwrap();
        assert!(object.change.is_none());
        assert_eq!(object.image.as_ref().unwrap().name, b"b.pb3");
        assert!(object.draw.as_ref().unwrap().world_motion.is_some());
        assert_eq!(object.draw.as_ref().unwrap().words[21], 256);
    }
}

#[test]
fn replacement_control_errors_preserve_scene_parameters_results_and_input() {
    for (opcode, args) in [
        (0x22c5, vec![256]),
        (0x22c6, vec![u32::MAX]),
        (0x22c7, vec![256, 1]),
        (0x22c5, vec![19]),
        (0x22c6, vec![19]),
        (0x22c7, vec![19, 1]),
        (0x22c5, vec![]),
        (0x22c6, vec![]),
        (0x22c7, vec![20]),
    ] {
        let mut code = vec![];
        replacement_setup(&mut code, 500, 0);
        scene_command(&mut code, opcode, &args);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = change_files();
        prepare_replacement(&mut vm, &mut scene, &mut files);
        scene.try_change_image(&mut vm, &mut files).unwrap();
        run_to_system(&mut vm);
        vm.state_mut().set_system_integer(0, 99).unwrap();
        let expected = scene.object(20).unwrap().clone();
        let mut input = ActionInput::default();
        input.set(0, true, true, true).unwrap();
        let expected_input = input.clone();
        for _ in 0..2 {
            if opcode == 0x22c7 {
                assert!(
                    scene
                        .try_wait_change(&mut vm, SkipGate::default(), &mut input)
                        .is_err()
                );
            } else {
                assert!(scene.try_handle(&mut vm).is_err());
            }
            assert_eq!(scene.object(20), Some(&expected));
            assert_eq!(input, expected_input);
            assert_eq!(vm.state().system_integer(0), Some(99));
            assert_eq!(vm.pending_system(), Some((code.len() - 2, opcode - 0x2000)));
            assert_eq!(vm.stack().len_bytes(), args.len() * 4);
            for (i, arg) in args.iter().enumerate() {
                assert_eq!(vm.stack().peek_u32((i + 1) * 4).unwrap(), *arg);
            }
        }
    }
}

#[test]
fn replacement_control_without_draw_allows_query_and_inactive_wait_but_rejects_cancel() {
    let mut code = vec![];
    for (opcode, args) in [
        (0x2020, vec![20]),
        (0x22c6, vec![20]),
        (0x22c7, vec![20, 1]),
        (0x22c5, vec![20]),
    ] {
        scene_command(&mut code, opcode, &args);
    }
    let program = script(&code, b"");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    run_to_system(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    run_to_system(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    run_to_system(&mut vm);
    let mut input = ActionInput::default();
    input.set(0, false, false, true).unwrap();
    assert_eq!(
        scene
            .try_wait_change(
                &mut vm,
                SkipGate {
                    effects: false,
                    skip: true
                },
                &mut input
            )
            .unwrap(),
        Some(Step::Continue)
    );
    assert!(input.released(0));
    assert_eq!(vm.state().system_integer(0), Some(0));
    run_to_system(&mut vm);
    assert!(scene.try_handle(&mut vm).is_err());
    assert!(scene.object(20).unwrap().draw.is_none());
    assert_eq!(vm.stack().len_bytes(), 4);
}

fn sequence_scene(vm: &mut Machine<'_>, scene: &mut SceneHost, files: &mut Files) {
    assert_eq!(run_to_system(vm), 0x20);
    scene.try_handle(vm).unwrap();
    assert_eq!(run_to_system(vm), 0x30);
    scene.try_handle_image(vm, files).unwrap();
    assert_eq!(run_to_system(vm), 0x40);
    scene.try_handle(vm).unwrap();
}

#[test]
fn sequence_image_load_is_deferred_until_wait_expires_and_replacement_advances_in_same_frame() {
    use cmvs_rs::sequence::SequenceHost;
    let mut code = loads(20, &[0]);
    scene_command(&mut code, 0x2040, &[20]);
    scene_command(&mut code, 0x2286, &[30]);
    scene_command(&mut code, 0x2288, &[20, 0x8000_0003, 100]);
    scene_command(&mut code, 0x2282, &[]);
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = change_files();
    sequence_scene(&mut vm, &mut scene, &mut files);
    let previous = scene.object(20).unwrap().clone();
    let mut sequence = SequenceHost::default();
    vm.state_mut().set_system_integer(0, 987).unwrap();
    vm.state_mut().set_string(3, b"b.pb3").unwrap();
    for _ in 0..3 {
        let command = run_to_system(&mut vm);
        sequence.try_handle(&mut vm).unwrap();
        if command == 0x288 {
            vm.state_mut().set_string(3, b"a.pb3").unwrap();
        }
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(987));
    // Removing the queued asset cannot affect append/start or the preceding wait.
    let asset = files.0.remove(b"b.pb3".as_slice()).unwrap();
    sequence
        .advance_frame_with_images(29, &mut scene, &mut files)
        .unwrap();
    assert_eq!(scene.object(20), Some(&previous));
    assert!(
        sequence
            .advance_frame_with_images(1, &mut scene, &mut files)
            .is_err()
    );
    assert_eq!((sequence.header().index, sequence.header().waited), (1, 30));
    assert_eq!(scene.object(20), Some(&previous));
    files.0.insert(b"b.pb3".to_vec(), asset);
    sequence
        .advance_frame_with_images(0, &mut scene, &mut files)
        .unwrap();
    let object = scene.object(20).unwrap();
    assert_eq!(object.image.as_ref().unwrap().name, b"b.pb3");
    assert_eq!(object.change.as_ref().unwrap().duration, 100);
    assert_eq!(object.change.as_ref().unwrap().elapsed, 0);
    assert_eq!(sequence.header().active, 0);
    scene.advance_frame(25);
    assert_eq!(
        scene.object(20).unwrap().change.as_ref().unwrap().elapsed,
        25
    );
}

#[test]
fn sequence_finish_uses_zero_duration_and_failed_finish_retains_pending_command() {
    use cmvs_rs::sequence::{SequenceHeader, SequenceHost};
    for active in [false, true] {
        let mut code = loads(20, &[0]);
        scene_command(&mut code, 0x2040, &[20]);
        scene_command(&mut code, 0x2286, &[u32::MAX]);
        scene_command(&mut code, 0x2288, &[20, 6, 999]);
        if active {
            scene_command(&mut code, 0x2282, &[]);
        }
        scene_command(&mut code, 0x2284, &[]);
        let program = script(&code, b"a.pb3\0b.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        let mut files = change_files();
        sequence_scene(&mut vm, &mut scene, &mut files);
        let previous = scene.object(20).unwrap().clone();
        let mut sequence = SequenceHost::default();
        for _ in 0..(2 + usize::from(active)) {
            run_to_system(&mut vm);
            sequence.try_handle(&mut vm).unwrap();
        }
        let entries = sequence.entries().clone();
        let asset = files.0.remove(b"b.pb3".as_slice()).unwrap();
        assert_eq!(run_to_system(&mut vm), 0x284);
        if active {
            assert!(
                sequence
                    .try_handle_with_images(&mut vm, &mut scene, &mut files)
                    .is_err()
            );
            assert_eq!(sequence.header().index, 1);
            assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x284)));
            assert_eq!(scene.object(20), Some(&previous));
        }
        files.0.insert(b"b.pb3".to_vec(), asset);
        sequence
            .try_handle_with_images(&mut vm, &mut scene, &mut files)
            .unwrap();
        assert_eq!(sequence.header(), SequenceHeader::default());
        assert_eq!(sequence.entries(), &entries);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(
            scene.object(20).unwrap().image.as_ref().unwrap().name,
            if active { b"b.pb3" } else { b"a.pb3" }
        );
        assert!(scene.object(20).unwrap().change.is_none());
    }
}

#[test]
fn sequence_missing_slot_skips_resource_io_but_invalid_slot_is_an_explicit_execution_error() {
    use cmvs_rs::sequence::SequenceHost;
    struct NoReads;
    impl ImageSource for NoReads {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            panic!("missing slot must not load");
        }
    }
    for slot in [255, 256, u32::MAX] {
        let mut code = vec![];
        scene_command(&mut code, 0x2288, &[slot, 0, 10]);
        scene_command(&mut code, 0x2282, &[]);
        let program = script(&code, b"missing.pb3\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut sequence = SequenceHost::default();
        for _ in 0..2 {
            run_to_system(&mut vm);
            sequence.try_handle(&mut vm).unwrap();
        }
        let result =
            sequence.advance_frame_with_images(16, &mut SceneHost::default(), &mut NoReads);
        assert_eq!(result.is_ok(), slot == 255);
        assert_eq!(sequence.header().index, u32::from(slot == 255));
    }
}

#[test]
fn sequence_image_append_checks_arguments_names_and_full_capacity_before_mutating() {
    use cmvs_rs::sequence::{SequenceEntry, SequenceHost};
    let mut code = vec![];
    for i in 0..17 {
        scene_command(&mut code, 0x2288, &[i, 0, i + 100]);
    }
    // Even at capacity the original handler resolves the string reference.
    scene_command(&mut code, 0x2288, &[0, 0x3fff_ffff, 0]);
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut sequence = SequenceHost::default();
    vm.state_mut().set_system_integer(0, 77).unwrap();
    for i in 0..17 {
        run_to_system(&mut vm);
        sequence.try_handle(&mut vm).unwrap();
        assert_eq!(sequence.header().count, (i + 1).min(16));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
    assert!(
        matches!(&sequence.entries()[0], Some(SequenceEntry::ReplaceImage{ slot:0, name, duration:100, .. }) if name == b"a.pb3")
    );
    let entries = sequence.entries().clone();
    run_to_system(&mut vm);
    assert!(sequence.try_handle(&mut vm).is_err());
    assert_eq!(sequence.entries(), &entries);
    assert_eq!(vm.stack().len_bytes(), 12);
    for length in [63, 64] {
        let mut code = vec![];
        scene_command(&mut code, 0x2288, &[0, 0, 0]);
        let mut text = vec![b'x'; length];
        text.push(0);
        let program = script(&code, &text);
        let mut vm = Machine::new(&program).unwrap();
        let mut sequence = SequenceHost::default();
        run_to_system(&mut vm);
        assert_eq!(sequence.try_handle(&mut vm).is_ok(), length == 63);
        assert_eq!(sequence.header().count, u32::from(length == 63));
        assert_eq!(vm.stack().len_bytes(), if length == 63 { 0 } else { 12 });
    }
}

#[test]
fn image_sequence_without_resource_context_reports_error_and_keeps_entry() {
    use cmvs_rs::sequence::SequenceHost;
    let mut code = vec![];
    scene_command(&mut code, 0x2288, &[0, 0, 10]);
    scene_command(&mut code, 0x2282, &[]);
    scene_command(&mut code, 0x2284, &[]);
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut sequence = SequenceHost::default();
    for _ in 0..2 {
        run_to_system(&mut vm);
        sequence.try_handle(&mut vm).unwrap();
    }
    assert!(sequence.advance_frame(16).is_err());
    assert_eq!(sequence.header().index, 0);
    run_to_system(&mut vm);
    assert!(sequence.try_handle(&mut vm).is_err());
    assert_eq!(sequence.header().index, 0);
    assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x284)));
}

#[test]
fn releasing_window_image_keeps_children_draw_text_visibility_and_stack_below_the_slot() {
    let mut code = window_load(0);
    scene_command(&mut code, 0x2180, &[7, u32::MAX]);
    scene_command(&mut code, 0x214e, &[7, 1]);
    scene_command(&mut code, 0x2178, &[7, 3]);
    scene_command(&mut code, 0x2180, &[7, 3]);
    scene_command(&mut code, 0x2183, &[7, 3, 12, 34]);
    // A stack value below the one-word argument must survive both releases.
    push(&mut code, 0x12345678);
    scene_command(&mut code, 0x2171, &[7]);
    scene_command(&mut code, 0x2171, &[7]);
    let program = script(&code, b"win.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut messages = MessageHost::default();
    let mut files = Files(BTreeMap::from([(b"win.pb3".to_vec(), opaque())]));
    run_to_system(&mut vm);
    messages.try_handle(&mut vm).unwrap();
    run_to_system(&mut vm);
    messages.try_handle_image(&mut vm, &mut files).unwrap();
    for _ in 0..5 {
        run_to_system(&mut vm);
        messages.try_handle(&mut vm).unwrap();
    }
    let mut expected = messages.window(7).unwrap().clone();
    assert!(expected.object.image.take().is_some());
    let text = messages.text(10).unwrap().clone();
    vm.state_mut().set_system_integer(0, 901).unwrap();
    for _ in 0..2 {
        run_to_system(&mut vm);
        let pc = vm.pc();
        messages.try_handle(&mut vm).unwrap();
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(messages.window(7), Some(&expected));
        assert_eq!(messages.text(10), Some(&text));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
        assert_eq!(vm.state().system_integer(0), Some(901));
    }
}

#[test]
fn replacement_moves_position_motion_without_cloning_it_into_retained_objects() {
    let mut code = loads(20, &[0]);
    for (op, args) in [
        (0x2040, vec![20]),
        (0x21f4, vec![20, u32::MAX]),
        (0x21f5, vec![20, u32::MAX, 15, u32::MAX, 0, 400, 100, 200]),
        (0x21f6, vec![20, u32::MAX, 15]),
        (0x22c4, vec![20, 6, 100, 0]),
    ] {
        scene_command(&mut code, op, &args);
    }
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files(BTreeMap::from([
        (b"a.pb3".to_vec(), opaque()),
        (b"b.pb3".to_vec(), opaque()),
    ]));
    for _ in 0..6 {
        let op = run_to_system(&mut vm);
        if op == 0x30 {
            scene.try_handle_image(&mut vm, &mut files).unwrap();
        } else {
            scene.try_handle(&mut vm).unwrap();
        }
    }
    let before = scene
        .object(20)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .position_motion
        .clone();
    assert!(before.is_some());
    assert_eq!(run_to_system(&mut vm), 0x2c4);
    scene.try_change_image(&mut vm, &mut files).unwrap();
    let object = scene.object(20).unwrap();
    assert_eq!(object.draw.as_ref().unwrap().position_motion, before);
    let change = object.change.as_ref().unwrap();
    assert!(
        change
            .previous
            .draw
            .as_ref()
            .unwrap()
            .position_motion
            .is_none()
    );
    assert!(
        change
            .common
            .draw
            .as_ref()
            .unwrap()
            .position_motion
            .is_none()
    );
    scene.advance_frame(16);
    assert_eq!(
        scene
            .object(20)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .position_motion
            .as_ref()
            .unwrap()
            .playback
            .as_ref()
            .unwrap()
            .elapsed,
        16
    );
}

#[test]
fn replacement_moves_scale_motion_without_cloning_it_into_retained_objects() {
    let mut code = loads(20, &[0]);
    for (op, args) in [
        (0x2040, vec![20]),
        (0x21e0, vec![20, u32::MAX, 0, 0]),
        (
            0x21e1,
            vec![
                20,
                u32::MAX,
                7,
                u32::MAX,
                0,
                400,
                1.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
        ),
        (0x21e2, vec![20, u32::MAX, 7]),
        (0x22c4, vec![20, 6, 100, 0]),
    ] {
        scene_command(&mut code, op, &args);
    }
    let program = script(&code, b"a.pb3\0b.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files(BTreeMap::from([
        (b"a.pb3".to_vec(), opaque()),
        (b"b.pb3".to_vec(), opaque()),
    ]));
    for _ in 0..6 {
        let op = run_to_system(&mut vm);
        if op == 0x30 {
            scene.try_handle_image(&mut vm, &mut files).unwrap();
        } else {
            scene.try_handle(&mut vm).unwrap();
        }
    }
    let before = scene
        .object(20)
        .unwrap()
        .draw
        .as_ref()
        .unwrap()
        .scale_motion
        .clone();
    assert!(before.is_some());
    assert_eq!(run_to_system(&mut vm), 0x2c4);
    scene.try_change_image(&mut vm, &mut files).unwrap();
    let object = scene.object(20).unwrap();
    assert_eq!(object.draw.as_ref().unwrap().scale_motion, before);
    let change = object.change.as_ref().unwrap();
    assert!(
        change
            .previous
            .draw
            .as_ref()
            .unwrap()
            .scale_motion
            .is_none()
    );
    assert!(change.common.draw.as_ref().unwrap().scale_motion.is_none());
    scene.advance_frame(16);
    assert_eq!(
        scene
            .object(20)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .scale_motion
            .as_ref()
            .unwrap()
            .playback
            .as_ref()
            .unwrap()
            .elapsed,
        16
    );
}

#[test]
fn auxiliary_second_texture_replaces_only_mask_and_survives_content_clear() {
    let mut code = vec![];
    scene_command(&mut code, 0x2380, &[7, 0, 25, 32]);
    for _ in 0..2 {
        scene_command(&mut code, 0x2397, &[7, 1]);
    }
    scene_command(&mut code, 0x2386, &[7]);
    let script = script(&code, b"\0mask.pb3\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut messages = MessageHost::default();
    let mut files = Files::default();
    run_to_system(&mut vm);
    messages.try_handle(&mut vm).unwrap();
    let before = messages.auxiliary_text(7).unwrap().clone();
    // 0x14003b170 selects second-texture mode 4 on the atlas draw state.
    let mut object = before.object.clone();
    object.draw.as_mut().unwrap().texture_mode = 4;
    for i in 0..7 {
        vm.state_mut().set_system_integer(i, 70 + i).unwrap();
    }
    for bytes in [opaque(), pb3(1, &[[0, 0], [0, 0], [0, 0], [128, 255]])] {
        files.0.insert(b"mask.pb3".to_vec(), bytes.clone());
        run_to_system(&mut vm);
        assert!(messages.try_handle_image(&mut vm, &mut files).unwrap());
        let group = messages.auxiliary_text(7).unwrap();
        assert_eq!(group.object, object);
        assert_eq!(group.atlas, before.atlas);
        let image = group.secondary_image.as_ref().unwrap();
        assert_eq!(image.name, b"mask.pb3");
        assert_eq!(image.pixels, decode(&bytes).unwrap().unwrap().pixels);
        assert_eq!(vm.stack().len_bytes(), 0);
        for i in 0..7 {
            assert_eq!(vm.state().system_integer(i), Some(70 + i));
        }
    }
    let mask = messages.auxiliary_text(7).unwrap().secondary_image.clone();
    run_to_system(&mut vm);
    messages.try_handle(&mut vm).unwrap();
    assert_eq!(messages.auxiliary_text(7).unwrap().secondary_image, mask);
}

#[test]
fn auxiliary_second_texture_errors_preserve_pending_command_and_previous_texture() {
    for slot in [0, 7, 8, u32::MAX] {
        for data in [None, Some(b"bad".to_vec())] {
            let mut code = vec![];
            scene_command(&mut code, 0x2380, &[7, 0, 25, 32]);
            scene_command(&mut code, 0x2397, &[7, 1]);
            scene_command(&mut code, 0x2397, &[slot, 1]);
            let script = script(&code, b"\0mask.pb3\0");
            let mut vm = Machine::new(&script).unwrap();
            let mut messages = MessageHost::default();
            let mut files = Files::default();
            files.0.insert(b"mask.pb3".to_vec(), opaque());
            run_to_system(&mut vm);
            messages.try_handle(&mut vm).unwrap();
            run_to_system(&mut vm);
            messages.try_handle_image(&mut vm, &mut files).unwrap();
            let before = messages.auxiliary_text(7).unwrap().clone();
            if let Some(data) = data {
                files.0.insert(b"mask.pb3".to_vec(), data);
            } else {
                files.0.clear();
            }
            run_to_system(&mut vm);
            let pending = vm.pending_system();
            let sp = vm.stack().len_bytes();
            assert!(messages.try_handle_image(&mut vm, &mut files).is_err());
            assert_eq!(vm.pending_system(), pending);
            assert_eq!(vm.stack().len_bytes(), sp);
            let group = messages.auxiliary_text(7).unwrap();
            assert_eq!(group.secondary_image, before.secondary_image);
            assert_eq!(group.object, before.object);
            assert_eq!(group.atlas, before.atlas);
        }
    }
}

#[test]
fn second_texture_load_and_screen_mapping_use_green_as_alpha() {
    let mut code = loads(11, &[0]);
    scene_command(&mut code, 0x2040, &[11]);
    scene_command(&mut code, 0x2044, &[11, u32::MAX, 0, 0, 2, 1]);
    scene_command(&mut code, 0x2035, &[11, 6, 0, 0]);
    scene_command(&mut code, 0x2321, &[11, u32::MAX, 4]);
    scene_command(&mut code, 0x22cb, &[0x806040]);
    scene_command(&mut code, 0x2322, &[11, u32::MAX, 1]);
    let script = script(&code, b"a.pb3\0m.pb3\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(
        b"a.pb3".to_vec(),
        pb3(1, &[[255, 255], [255, 255], [255, 255]]),
    );
    files.0.insert(
        b"m.pb3".to_vec(),
        pb3(1, &[[255, 255], [0, 255], [255, 255]]),
    );
    for _ in 0..6 {
        let cmd = run_to_system(&mut vm);
        if matches!(cmd, 0x30 | 0x35) {
            assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
        } else {
            assert!(scene.try_handle(&mut vm).unwrap());
        }
    }
    assert_eq!(
        scene
            .object(11)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .texture_mode,
        4
    );
    assert_eq!(
        scene
            .object(11)
            .unwrap()
            .secondary_image
            .as_ref()
            .unwrap()
            .name,
        b"m.pb3"
    );
    let frame = cmvs_rs::scene_render::render(&scene, 2, 1).unwrap();
    assert_eq!(frame.pixels(), &[0, 0, 0, 255, 128, 128, 128, 255]);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(1));
    for _ in 0..2 {
        run_to_system(&mut vm);
        assert!(scene.try_handle(&mut vm).unwrap());
    }
    let frame = cmvs_rs::scene_render::render(&scene, 2, 1).unwrap();
    assert_eq!(frame.pixels(), &[128, 96, 64, 255, 64, 48, 32, 255]);
}

#[test]
fn second_texture_bad_flags_and_invalid_slots_preserve_main_image_and_pending_command() {
    for (slot, first, second) in [
        (11, 1, 0),
        (11, 0, 1),
        (12, 0, 0),
        (256, 0, 0),
        (u32::MAX, 0, 0),
    ] {
        let mut code = loads(11, &[0]);
        scene_command(&mut code, 0x2035, &[slot, 0, first, second]);
        let script = script(&code, b"a.pb3\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut scene = SceneHost::default();
        let mut files = Files::default();
        files.0.insert(b"a.pb3".to_vec(), opaque());
        run_to_system(&mut vm);
        scene.try_handle(&mut vm).unwrap();
        run_to_system(&mut vm);
        scene.try_handle_image(&mut vm, &mut files).unwrap();
        let before = scene.object(11).unwrap().clone();
        run_to_system(&mut vm);
        let pc = vm.pc();
        let sp = vm.stack().len_bytes();
        assert!(scene.try_handle_image(&mut vm, &mut files).is_err());
        assert_eq!(scene.object(11), Some(&before));
        assert_eq!(vm.pc(), pc);
        assert_eq!(vm.stack().len_bytes(), sp);
        assert_eq!(vm.pending_system(), Some((pc, 0x35)));
    }
}

#[test]
fn child_inherits_parent_second_texture_and_screen_mode() {
    let mut code = loads(11, &[0]);
    scene_command(&mut code, 0x2040, &[11]);
    scene_command(&mut code, 0x2035, &[11, 6, 0, 0]);
    scene_command(&mut code, 0x2321, &[11, u32::MAX, 4]);
    scene_command(&mut code, 0x2022, &[11, 0]);
    scene_command(&mut code, 0x2050, &[11, 0]);
    scene_command(&mut code, 0x2044, &[11, 0, 0, 0, 2, 1]);
    let script = script(&code, b"a.pb3\0m.pb3\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(
        b"a.pb3".to_vec(),
        pb3(1, &[[255, 255], [255, 255], [255, 255]]),
    );
    files.0.insert(
        b"m.pb3".to_vec(),
        pb3(1, &[[255, 255], [0, 255], [255, 255]]),
    );
    for _ in 0..8 {
        let cmd = run_to_system(&mut vm);
        if matches!(cmd, 0x30 | 0x35) {
            scene.try_handle_image(&mut vm, &mut files).unwrap();
        } else {
            scene.try_handle(&mut vm).unwrap();
        }
    }
    let frame = cmvs_rs::scene_render::render(&scene, 2, 1).unwrap();
    assert_eq!(frame.pixels(), &[0, 0, 0, 255, 128, 128, 128, 255]);
}

#[test]
fn pixel_query_returns_bgra_words_zero_outside_or_without_image_and_reads_children() {
    let mut code = loads(4, &[0]);
    scene_command(&mut code, 0x2022, &[4, 7]);
    scene_command(&mut code, 0x2038, &[4, 7, 0]);
    scene_command(&mut code, 0x2020, &[5]);
    let setup = code.len();
    let queries = [
        ([4, u32::MAX, 0, 0], 0xff05_0301),
        ([4, u32::MAX, 1, 0], 0xff06_0402),
        ([4, u32::MAX, 2, 0], 0),
        ([4, u32::MAX, u32::MAX, 0], 0),
        ([4, u32::MAX, 0, 1], 0),
        ([4, 7, 1, 0], 0xff06_0402),
        ([5, u32::MAX, 0, 0], 0),
    ];
    for (args, _) in queries {
        scene_command(&mut code, 0x2034, &args);
    }
    for args in [[6, u32::MAX, 0, 0], [4, 8, 0, 0], [4, 768, 0, 0]] {
        scene_command(&mut code, 0x2034, &args);
    }
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(b"a.pb3".to_vec(), opaque());
    while vm.pc() < setup {
        run_to_system(&mut vm);
        assert!(
            scene.try_handle(&mut vm).unwrap()
                || scene.try_handle_image(&mut vm, &mut files).unwrap()
        );
    }
    for (_, expected) in queries {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        run_to_system(&mut vm);
        assert!(scene.try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    run_to_system(&mut vm);
    let before = scene.object(4).cloned();
    assert!(scene.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x34));
    assert_eq!(vm.stack().len_bytes(), 16);
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(scene.object(4).cloned(), before);
}

#[test]
fn pixel_query_keeps_alpha_and_rejects_filtered_images() {
    let mut code = loads(4, &[0]);
    scene_command(&mut code, 0x2034, &[4, u32::MAX, 1, 0]);
    scene_command(&mut code, 0x203b, &[4, 0, 2, 0x80_8080, 0]);
    scene_command(&mut code, 0x2034, &[4, u32::MAX, 0, 0]);
    let program = script(&code, b"a.pb3\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    let mut files = Files::default();
    files.0.insert(
        b"a.pb3".to_vec(),
        pb3(1, &[[200, 255], [128, 1], [240, 64], [0, 17]]),
    );
    for _ in 0..3 {
        run_to_system(&mut vm);
        if !scene.try_handle(&mut vm).unwrap() {
            assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
        }
    }
    assert_eq!(vm.state().system_integer(0), Some(0x1140_01ff));
    run_to_system(&mut vm);
    assert!(scene.try_handle_image(&mut vm, &mut files).unwrap());
    assert!(
        scene
            .object(4)
            .unwrap()
            .image
            .as_ref()
            .unwrap()
            .filter
            .is_some()
    );
    run_to_system(&mut vm);
    assert!(scene.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x34));
    assert_eq!(vm.stack().len_bytes(), 16);
}

use cmvs_rs::{
    camera::{Camera, CameraHost},
    image::ImageSource,
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

/// Text offsets: black-then-orange image 0, white image 6, mask 12.
const TEXT: &[u8] = b"a.pb3\0w.pb3\0m.pb3\0";
const ORANGE: [u8; 3] = [200, 100, 40];

struct Files(BTreeMap<Vec<u8>, Vec<u8>>);
impl ImageSource for Files {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(self.0.get(name).cloned())
    }
}
fn files() -> Files {
    let mut files = BTreeMap::new();
    let [r, g, b] = ORANGE;
    files.insert(b"a.pb3".to_vec(), pb3(&[[0, b], [0, g], [0, r]]));
    files.insert(b"w.pb3".to_vec(), pb3(&[[255; 2]; 3]));
    files.insert(b"m.pb3".to_vec(), pb3(&[[9, 9], [0, 255], [9, 9]]));
    Files(files)
}

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
fn f(value: f32) -> u32 {
    value.to_bits()
}

/// Camera 0 of type 3 at the origin (the constructor puts it at y = 3) on an 8x4 screen:
/// +24 = 10, +28 = 8, +32 = 4, so a point at depth 10 maps x to x + 4 and y to 2 − y at
/// unit ratio.
fn camera(code: &mut Vec<u8>) {
    command(code, 0x205f, &[0, 3]);
    command(code, 0x205a, &[0, 0, 0, 0]);
    command(code, 0x2058, &[0, f(8.0), f(4.0), f(10.0)]);
    command(code, 0x2059, &[0, f(8.0), f(4.0)]);
    command(code, 0x205c, &[0, 0]);
}
/// A coordinate mode 2 object in `slot` showing image `name` with origin 0.
fn projected(code: &mut Vec<u8>, slot: u32, name: u32, world: [f32; 3], reference: f32) {
    command(code, 0x2020, &[slot]);
    command(code, 0x2030, &[slot, name]);
    command(code, 0x2040, &[slot]);
    command(code, 0x2044, &[slot, u32::MAX, 0, 0, 2, 1]);
    command(code, 0x2042, &[slot, u32::MAX]);
    command(code, 0x2070, &[slot, u32::MAX, f(reference)]);
    for (op, value) in [(0x2072, world[0]), (0x2073, world[1]), (0x2074, world[2])] {
        command(code, op, &[slot, u32::MAX, f(value)]);
    }
}
/// Runs every command of `code` through the scene and camera hosts.
fn build(code: &[u8]) -> (SceneHost, CameraHost) {
    let script = script(code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let mut cameras = CameraHost::default();
    let mut files = files();
    while vm.pc() < code.len() {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        assert!(
            scene.try_handle(&mut vm).unwrap()
                || cameras.try_handle(&mut vm).unwrap()
                || scene.try_handle_image(&mut vm, &mut files).unwrap()
                || scene.try_handle_mask(&mut vm, &mut files).unwrap()
                || scene.try_change_image(&mut vm, &mut files).unwrap()
        );
    }
    (scene, cameras)
}
fn render(code: &[u8]) -> Vec<[u8; 3]> {
    let (scene, cameras) = build(code);
    let frame = scene_render::render_scene(&scene, Some(&cameras), &[], 8, 4).unwrap();
    frame
        .pixels()
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}
fn rejected(code: &[u8]) -> bool {
    let (scene, cameras) = build(code);
    scene_render::render_scene(&scene, Some(&cameras), &[], 8, 4).is_err()
}

#[test]
fn type_three_projection_matches_the_measured_first_scene() {
    // Camera 0 and the background and standing picture of sn1001 at click 65.
    let mut camera = Camera::default();
    camera.words[0] = 3;
    for (index, value) in [
        (2, 36.0f32),
        (6, 70.0),
        (7, 128.0),
        (8, 72.0),
        (9, 1280.0),
        (10, 720.0),
        (11, 6.0),
        (15, 640.0),
        (16, 360.0),
    ] {
        camera.words[index] = value.to_bits();
    }
    let background = camera
        .project_object([0.0, 36.0, 70.0], 0.0, 70.0)
        .unwrap()
        .unwrap();
    assert_eq!(background.position, [640.0, 360.0]);
    assert_eq!(
        (background.depth, background.scale, background.fade),
        (70.0, 1.0, 255)
    );
    let standing = camera
        .project_object([0.0, 36.0, 27.5], 0.0, 25.0)
        .unwrap()
        .unwrap();
    let ratio = 25.0f32 / 27.5;
    assert_eq!(standing.position, [640.0, 345.0]);
    assert_eq!((standing.scale, standing.fade), (ratio * ratio, 255));
    // Depth 9 is clipped; below 10 the fade is (int)((depth − 9) × 256).
    assert_eq!(camera.project_object([0.0, 0.0, 9.0], 0.0, 70.0), Ok(None));
    let near = camera
        .project_object([0.0, 0.0, 9.5], 0.0, 70.0)
        .unwrap()
        .unwrap();
    assert_eq!(near.fade, 128);
    assert!(
        Camera::default()
            .project_object([0.0; 3], 0.0, 1.0)
            .is_err()
    );
}

#[test]
fn unit_scale_objects_copy_texels_at_the_projected_position() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 0, [1.0, 0.0, 10.0], 10.0);
    let pixels = render(&code);
    for (index, pixel) in pixels.iter().enumerate() {
        let expected = match index {
            21 => [0, 0, 0],
            22 => ORANGE,
            _ => [0, 0, 0],
        };
        assert_eq!(*pixel, expected, "pixel {index}");
    }
    // Moving the camera right by one moves the object left by one.
    command(&mut code, 0x205a, &[0, f(1.0), 0, 0]);
    assert_eq!(render(&code)[21], ORANGE);
}

#[test]
fn magnified_objects_are_sampled_bilinearly_with_clamped_edges() {
    // Reference 20 at depth 10: ratio 2, scale 4, so the 2x1 image covers all 8x4 pixels.
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 0, [-2.0, 1.0, 10.0], 20.0);
    let pixels = render(&code);
    // u = (x + 0.5) / 4 − 0.5: 0 for x < 2, 1 for x >= 6, linear in between.
    let weights = [0.0, 0.0, 0.125, 0.375, 0.625, 0.875, 1.0, 1.0];
    for row in pixels.chunks_exact(8) {
        for (x, pixel) in row.iter().enumerate() {
            let expected = ORANGE.map(|c| (f32::from(c) * weights[x]).round() as u8);
            assert_eq!(*pixel, expected, "column {x}");
        }
    }
}

#[test]
fn deeper_objects_draw_first_and_two_dimensional_objects_cover_projections() {
    let mut code = vec![];
    camera(&mut code);
    // Slot 1 (white) is nearer than slot 2 (orange); both cover pixel 21.
    projected(&mut code, 1, 6, [1.0, 0.0, 10.0], 10.0);
    projected(&mut code, 2, 0, [0.0, 0.0, 11.0], 11.0);
    let pixels = render(&code);
    assert_eq!(pixels[21], [255; 3]);
    // At unit ratio the deeper object maps x to (10 / 11) * x + 4.
    assert_eq!(pixels[20], [0, 0, 0]);
    // A 2D object with a far lower layer still draws after queue 2.
    command(&mut code, 0x2020, &[3]);
    command(&mut code, 0x2030, &[3, 0]);
    command(&mut code, 0x2040, &[3]);
    command(&mut code, 0x2044, &[3, u32::MAX, 0, 0, 2, 1]);
    command(&mut code, 0x2045, &[3, u32::MAX, 4, 2]);
    command(&mut code, 0x2047, &[3, u32::MAX, (-100i32) as u32]);
    let pixels = render(&code);
    assert_eq!((pixels[20], pixels[21]), ([0, 0, 0], ORANGE));
}

#[test]
fn objects_fade_in_between_depth_nine_and_ten_and_vanish_nearer() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 6, [0.0, 0.0, 9.5], 9.5);
    // fade 128 × opacity 255 / 255 = 128 over black.
    let expected = (255 * 128 + 127) / 255;
    assert_eq!(render(&code)[20], [expected as u8; 3]);
    command(&mut code, 0x2074, &[1, u32::MAX, f(9.0)]);
    assert!(render(&code).iter().all(|p| *p == [0, 0, 0]));
}

#[test]
fn unverified_projection_paths_fail_explicitly() {
    let mut base = vec![];
    camera(&mut base);
    projected(&mut base, 1, 0, [0.0, 0.0, 10.0], 10.0);
    let (scene, _) = build(&base);
    assert!(scene_render::render(&scene, 8, 4).is_err());
    let variants: [&[(u16, &[u32])]; 5] = [
        &[(0x205f, &[0, 1])],
        &[(0x205b, &[0, f(10.0)])],
        &[(0x209c, &[0, 1])],
        &[(0x2022, &[1, 0]), (0x2040, &[1])],
        &[(0x222a, &[12, 1, 100, 50, 1])],
    ];
    for extra in variants {
        let mut code = base.clone();
        for (op, args) in extra {
            command(&mut code, *op, args);
        }
        assert!(rejected(&code), "{extra:x?}");
    }
}

#[test]
fn second_camera_has_its_own_queue_and_enable_flag() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 6, [0.0, 0.0, 10.0], 10.0);
    projected(&mut code, 2, 0, [0.0, 0.0, 10.0], 10.0);
    command(&mut code, 0x204c, &[2, u32::MAX, 1]);
    // Disabled camera 1 does not draw, even though its default type is unsupported.
    assert_eq!(render(&code)[20], [255; 3]);
    command(&mut code, 0x205f, &[1, 3]);
    command(&mut code, 0x205a, &[1, 0, 0, 0]);
    command(&mut code, 0x2058, &[1, f(8.0), f(4.0), f(10.0)]);
    command(&mut code, 0x2059, &[1, f(8.0), f(4.0)]);
    command(&mut code, 0x205c, &[1, 0]);
    command(&mut code, 0x205e, &[1, 1]);
    assert_eq!((render(&code)[20], render(&code)[21]), ([0; 3], ORANGE));
    command(&mut code, 0x205a, &[1, f(1.0), 0, 0]);
    assert_eq!(render(&code)[20], ORANGE);
}

#[test]
fn replacement_renders_retained_images_and_finishes_without_a_dark_hole() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 0, [0.0, 0.0, 10.0], 10.0);
    command(&mut code, 0x22c4, &[1, 6, 100, 0]);
    let (mut scene, cameras) = build(&code);
    let sample = |scene: &SceneHost| {
        let frame = scene_render::render_scene(scene, Some(&cameras), &[], 8, 4).unwrap();
        frame.pixels()[80..88].to_vec()
    };
    assert_eq!(sample(&scene), [0, 0, 0, 255, 200, 100, 40, 255]);
    scene.advance_frame(50);
    assert_eq!(sample(&scene), [191, 191, 191, 255, 241, 216, 201, 255]);
    scene.advance_frame(50);
    assert_eq!(sample(&scene), [255; 8]);
    assert!(scene.objects()[&1].change.is_none());
}

#[test]
fn non_finite_projected_extents_are_reported_and_near_nan_depth_is_clipped() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 0, [f32::INFINITY, 0.0, 10.0], 10.0);
    assert!(rejected(&code));
    command(&mut code, 0x2074, &[1, u32::MAX, f(f32::NAN)]);
    assert!(render(&code).iter().all(|p| *p == [0; 3]));
}

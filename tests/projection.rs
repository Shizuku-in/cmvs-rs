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
fn depth_bias_orders_and_gates_the_queue_without_moving_the_projection() {
    let mut code = vec![];
    camera(&mut code);
    // Equal keys keep slot order: orange (black, orange) over white at pixels 20/21.
    projected(&mut code, 1, 6, [0.0, 0.0, 10.0], 10.0);
    projected(&mut code, 2, 0, [0.0, 0.0, 10.0], 10.0);
    assert_eq!(&render(&code)[20..23], &[[0; 3], ORANGE, [0; 3]]);
    // A bias only moves slot 2 behind slot 1; positions and sizes stay at depth 10.
    command(&mut code, 0x2075, &[2, u32::MAX, f(0.125)]);
    assert_eq!(&render(&code)[20..23], &[[255; 3], [255; 3], [0; 3]]);
    // An unverified child would fail, but key (−10 + 10) − 0 is not above 0, so slot 1
    // is not enqueued and nothing of it is checked or drawn.
    command(&mut code, 0x2022, &[1, 0]);
    assert!(rejected(&code));
    command(&mut code, 0x2075, &[1, u32::MAX, f(-10.0)]);
    assert_eq!(&render(&code)[20..22], &[[0; 3], ORANGE]);
    // Image replacement copies the bias into the new and the common draw state.
    command(&mut code, 0x22c4, &[2, 6, 100, 0]);
    let (scene, _) = build(&code);
    let object = &scene.objects()[&2];
    let change = object.change.as_ref().unwrap();
    for draw in [&object.draw, &change.common.draw, &change.previous.draw] {
        assert_eq!(draw.as_ref().unwrap().depth_bias, f(0.125));
    }
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

#[test]
fn solid_rectangles_clip_use_origin_and_blend_opacity_instead_of_color_alpha() {
    for (blend, opacity, expected) in [
        (1, 128, [110, 70, 50]),
        (2, 128, [120, 90, 75]),
        (2, 255, [200, 100, 50]),
    ] {
        let mut code = vec![];
        for slot in [0, 1] {
            command(&mut code, 0x2020, &[slot]);
            command(&mut code, 0x2040, &[slot]);
        }
        command(&mut code, 0x2043, &[0, u32::MAX, 8, 4, 0x142832]);
        command(&mut code, 0x2043, &[1, u32::MAX, 3, 2, 0xc86432]);
        command(&mut code, 0x2046, &[1, u32::MAX, 2, 1]);
        command(&mut code, 0x2045, &[1, u32::MAX, 1, 2]);
        command(&mut code, 0x2048, &[1, u32::MAX, opacity]);
        command(&mut code, 0x204b, &[1, u32::MAX, blend]);
        let frame = render(&code);
        assert_eq!(frame[8], expected);
        assert_eq!(frame[9], expected);
        assert_eq!(frame[10], [20, 40, 50]);
        assert_eq!(frame[0], [20, 40, 50]);
    }
}

#[test]
fn solid_color_blocks_child_image_inheritance_but_keeps_an_objects_own_image() {
    let mut code = vec![];
    command(&mut code, 0x2020, &[0]);
    command(&mut code, 0x2030, &[0, 6]); // white image
    command(&mut code, 0x2040, &[0]);
    command(&mut code, 0x2043, &[0, u32::MAX, 2, 1, 0xff0000]);
    command(&mut code, 0x2022, &[0, 0]);
    command(&mut code, 0x2050, &[0, 0]);
    command(&mut code, 0x2043, &[0, 0, 2, 1, 0x00ff00]);
    command(&mut code, 0x2045, &[0, 0, 2, 0]);
    let frame = render(&code);
    assert_eq!(&frame[..4], &[[255; 3], [255; 3], [0, 255, 0], [0, 255, 0]]);
    // An image-free top-level rectangle uses RGB and effective opacity even when
    // the color alpha is zero (a released image retains its source dimensions).
    let (mut scene, _) = build(&code);
    // Additional objects exercise the exact same renderer path without a loader.
    let mut object = scene.object(0).unwrap().clone();
    object.children.clear();
    object.image = None;
    object.draw.as_mut().unwrap().words[14] = 0x000000ff;
    scene = SceneHost::default();
    let frame = scene_render::render_with_objects(&scene, &[&object], 8, 4).unwrap();
    assert_eq!(&frame.pixels()[..4], &[0, 0, 255, 255]);
}

fn projected_child(code: &mut Vec<u8>) {
    command(code, 0x2022, &[1, 0]);
    command(code, 0x2038, &[1, 0, 6]);
    command(code, 0x2050, &[1, 0]);
    command(code, 0x2042, &[1, 0]);
    command(code, 0x2044, &[1, 0, 0, 0, 2, 1]);
    command(code, 0x2045, &[1, 0, u32::MAX, 0]);
}

#[test]
fn projected_scale_uses_own_origin_then_parent_offset_and_independent_xy_axes() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 6, [0.0, 0.0, 10.0], 10.0);
    command(&mut code, 0x2046, &[1, u32::MAX, 1, 0]);
    command(&mut code, 0x2049, &[1, u32::MAX, f(2.0), f(2.0)]);
    let frame = render(&code);
    for y in 0..4 {
        for x in 0..8 {
            assert_eq!(
                frame[y * 8 + x],
                if y >= 2 && (2..6).contains(&x) {
                    [255; 3]
                } else {
                    [0; 3]
                }
            );
        }
    }
    // Suppress the parent quad; preserve its origin and scale for the child.
    command(&mut code, 0x2044, &[1, u32::MAX, 0, 0, 0, 0]);
    command(&mut code, 0x2049, &[1, u32::MAX, f(2.0), f(1.0)]);
    projected_child(&mut code);
    command(&mut code, 0x2045, &[1, 0, 0, 0]);
    command(&mut code, 0x2046, &[1, 0, 1, 0]);
    command(&mut code, 0x2049, &[1, 0, f(0.5), f(1.0)]);
    let frame = render(&code);
    for y in 0..4 {
        for x in 0..8 {
            assert_eq!(
                frame[y * 8 + x],
                if y == 2 && (1..3).contains(&x) {
                    [255; 3]
                } else {
                    [0; 3]
                }
            );
        }
    }
    command(&mut code, 0x2049, &[1, 0, 0, f(1.0)]);
    assert!(render(&code).iter().all(|p| *p == [0; 3]));
    for scale in [f(-1.0), f(f32::NAN), f(f32::INFINITY)] {
        let mut invalid = code.clone();
        command(&mut invalid, 0x2049, &[1, 0, scale, f(1.0)]);
        assert!(rejected(&invalid));
    }
}
#[test]
fn projected_child_inherits_world_position_origin_and_opacity_without_using_own_world() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 0, [0.0, 0.0, 10.0], 10.0);
    projected_child(&mut code);
    let frame = render(&code);
    assert_eq!(&frame[19..22], &[[255; 3], [255; 3], ORANGE]);
    command(&mut code, 0x2072, &[1, 0, f(1000.0)]);
    command(&mut code, 0x2074, &[1, 0, f(1000.0)]);
    assert_eq!(
        render(&code),
        frame,
        "inherited child uses parent world coordinates"
    );
    command(&mut code, 0x2048, &[1, u32::MAX, 128]);
    command(&mut code, 0x2048, &[1, 0, 128]);
    let alpha = render(&code);
    assert_eq!(alpha[19], [64; 3]);
    assert_eq!(alpha[20], [64; 3]);
    assert_eq!(alpha[21], [100, 50, 20]);
    command(&mut code, 0x2046, &[1, 0, 1, 0]);
    let shifted = render(&code);
    assert_eq!(shifted[18], [64; 3]);
    assert_eq!(shifted[19], [64; 3]);
    assert_eq!(shifted[20], [0; 3]);
}
#[test]
fn projected_child_stays_in_parent_group_and_unsupported_flags_still_fail() {
    let mut code = vec![];
    camera(&mut code);
    projected(&mut code, 1, 6, [0.0, 0.0, 10.0], 10.0);
    projected_child(&mut code);
    projected(&mut code, 2, 0, [0.0, 0.0, 10.0], 10.0);
    let frame = render(&code);
    assert_eq!(frame[20], [0; 3]);
    assert_eq!(frame[21], ORANGE);
    let (scene, cameras) = build(&code);
    let mut object = scene.object(1).unwrap().clone();
    // Attached children sort by parent key + their own biased Z, independently of
    // the inherited projection coordinates. The larger-key orange child draws first.
    let mut farther = object.children[&0].clone();
    farther.image = Some(cmvs_rs::scene::ObjectImage {
        filter: None,
        name: b"orange".to_vec(),
        pixels: cmvs_rs::raster::RgbaImage::new(
            2,
            1,
            [
                ORANGE[0], ORANGE[1], ORANGE[2], 255, ORANGE[0], ORANGE[1], ORANGE[2], 255,
            ]
            .to_vec(),
        )
        .unwrap(),
        alpha: false,
    });
    farther.draw.as_mut().unwrap().words[13] = f(1.0);
    object.children.insert(1, farther);
    let ordered =
        scene_render::render_scene(&SceneHost::default(), Some(&cameras), &[&object], 8, 4)
            .unwrap();
    assert_eq!(&ordered.pixels()[76..84], &[255; 8]);
    object.children.remove(&1);
    for flags in [0, 1, 2] {
        object.children.get_mut(&0).unwrap().flags = flags;
        assert!(
            scene_render::render_scene(&SceneHost::default(), Some(&cameras), &[&object], 8, 4)
                .is_err()
        );
    }
    object.children.get_mut(&0).unwrap().flags = 3;
    object
        .children
        .get_mut(&0)
        .unwrap()
        .draw
        .as_mut()
        .unwrap()
        .words[0] = 0;
    assert!(
        scene_render::render_scene(&SceneHost::default(), Some(&cameras), &[&object], 8, 4).is_ok(),
        "mismatched coordinate modes do not enqueue"
    );
}

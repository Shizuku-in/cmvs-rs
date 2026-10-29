use cmvs_rs::{image::ImageSource, scene::DrawState, snapshot::VisualSnapshot};
use std::error::Error;

fn put(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn chunk(out: &mut Vec<u8>, tag: u16, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
}
fn record(out: &mut Vec<u8>, tag: u16, slot: u16, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    out.extend(slot.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
}
fn base() -> Vec<u8> {
    let mut b = Vec::new();
    for (tag, value) in [(0x200u16, 0x123456u32), (0x203, 1)] {
        b.extend(tag.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(value.to_le_bytes());
    }
    b
}
fn text() -> Vec<u8> {
    let mut b = 3u16.to_le_bytes().to_vec();
    b.extend(196u32.to_le_bytes());
    b.push(0);
    b.extend(1280u32.to_le_bytes());
    b.extend(720u32.to_le_bytes());
    let mut blocks = vec![0; 512];
    for (offset, value) in [
        (16, 13),
        (20, 17),
        (32, 24),
        (112, 1),
        (116, 0xabcdef),
        (148, 1),
        (156, 999),
        (160, 998),
    ] {
        put(&mut blocks, offset - 8, value);
    }
    put(&mut blocks, 156 + 4, 148);
    put(&mut blocks, 304, 1);
    put(&mut blocks, 308, 3);
    blocks[312..315].copy_from_slice(b"ab\0");
    blocks[376..378].copy_from_slice(b"c\0");
    b.extend(blocks);
    b.extend(0u16.to_le_bytes());
    b
}
fn draw(animation: Option<&[u8]>) -> Vec<u8> {
    let mut b = vec![4, 0, 0, 0];
    let mut words = DrawState::default().words;
    words[21] = 12;
    for n in words {
        b.extend(n.to_le_bytes());
    }
    b.extend([0; 24]);
    if let Some(a) = animation {
        chunk(&mut b, 0x793, a);
    }
    b.extend(0x7789u16.to_le_bytes());
    b
}
fn animation() -> Vec<u8> {
    let mut b = 2u16.to_le_bytes().to_vec();
    for n in [1u32, 0, 0, 7] {
        b.extend(n.to_le_bytes());
    }
    for i in 0..64 {
        b.extend((-1i32).to_le_bytes());
        for n in [i as i16, 2, 3, 4, -1, -1] {
            b.extend(n.to_le_bytes());
        }
        b.extend(50u32.to_le_bytes());
    }
    b
}
fn scene(draw: Option<&[u8]>, image: Option<&[u8]>, children: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![0xff, 0xff, 3, 0];
    for n in [u32::from(draw.is_some()), 1, 0xffff_ffff, 0, 0, 0, 0, 0] {
        b.extend(n.to_le_bytes());
    }
    if let Some(image) = image {
        chunk(&mut b, 0x740, image);
    }
    if let Some(draw) = draw {
        chunk(&mut b, 0x780, draw);
    }
    b.extend([0; 44]);
    for (slot, child) in children {
        record(&mut b, 0x700, *slot, child);
    }
    b.extend(0xf0f0u16.to_le_bytes());
    b
}
fn window(scene: &[u8]) -> Vec<u8> {
    let mut b = vec![7, 0, 3, 0];
    b.extend(1280u32.to_le_bytes());
    b.extend(720u32.to_le_bytes());
    let mut raw = vec![0; 2496];
    put(&mut raw, 0, 7);
    put(&mut raw, 4, 1);
    put(&mut raw, 12, (-20i32) as u32);
    put(&mut raw, 40, 1);
    put(&mut raw, 112, 3);
    put(&mut raw, 2492, 1);
    b.extend(raw);
    b.extend((scene.len() as u32).to_le_bytes());
    b.extend(scene);
    b
}
struct Empty;
impl ImageSource for Empty {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(None)
    }
}

#[test]
fn preview_resets_native_transients_and_preserves_window_metadata() {
    let mut b = base();
    let d = draw(Some(&animation()));
    let s = scene(Some(&d), None, &[(767, scene(None, None, &[]))]);
    record(&mut b, 0x400, 0, &window(&s)); // Forward reference is allowed.
    record(&mut b, 0x380, 7, &text());
    record(&mut b, 0x700, 255, &s);
    let visual = VisualSnapshot::decode_records(&b).unwrap();
    let saved = visual.windows()[0].as_ref().unwrap();
    assert_eq!(saved.button_words[0], 3);
    assert_eq!(saved.switch_state, 1);
    let mut p = visual.preview(&mut Empty).unwrap();
    assert_eq!(p.scene.clear().rgba(), [0x12, 0x34, 0x56, 255]);
    assert!(p.scene.clear().depth);
    let text = p.messages.text(7).unwrap();
    assert_eq!(text.cursor, [13, 17]);
    assert_eq!(text.color, 0xabcdef);
    assert_eq!(text.line_break.as_ref().unwrap().hanging, b"ab");
    assert!(p.messages.font_present(7));
    assert!(!p.messages.font_present(0));
    let window = p.messages.window(0).unwrap();
    assert_eq!(window.position, [-20, 0]);
    assert_eq!(window.pressed_button, -1);
    assert!(window.buttons[0].enabled);
    assert_eq!(window.object.draw.as_ref().unwrap().words[21], 256);
    assert_eq!(p.scene.object(255).unwrap().children.len(), 1);
    p.scene.advance_frame(43);
    p.messages.advance_frame(43);
    assert!(
        p.scene
            .object(255)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .animation
            .is_none()
    );
    assert!(
        p.messages
            .window(0)
            .unwrap()
            .object
            .draw
            .as_ref()
            .unwrap()
            .animation
            .is_none()
    );
}

#[test]
fn duplicates_slots_and_missing_text_are_errors() {
    let t = text();
    for (tag, slot, body) in [
        (0x380, 12, t.clone()),
        (0x400, 8, window(&scene(None, None, &[]))),
        (0x700, 256, scene(None, None, &[])),
    ] {
        let mut b = base();
        record(&mut b, tag, slot, &body);
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
    for tag in [0x380, 0x700] {
        let body = if tag == 0x380 {
            t.clone()
        } else {
            scene(None, None, &[])
        };
        let mut b = base();
        record(&mut b, tag, 0, &body);
        record(&mut b, tag, 0, &body);
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
    let mut b = base();
    record(&mut b, 0x400, 0, &window(&scene(None, None, &[])));
    assert!(VisualSnapshot::decode_records(&b).is_err());
    let mut wrong = window(&scene(None, None, &[]));
    put(&mut wrong, 4, 640);
    b.truncate(16);
    record(&mut b, 0x380, 7, &t);
    record(&mut b, 0x400, 0, &wrong);
    assert!(VisualSnapshot::decode_records(&b).is_err());
    let mut b = base();
    b.extend(base());
    assert!(VisualSnapshot::decode_records(&b).is_err());
}

#[test]
fn every_nested_truncation_and_trailing_byte_is_rejected() {
    let full = scene(
        Some(&draw(Some(&animation()))),
        None,
        &[(3, scene(None, None, &[]))],
    );
    for end in 0..full.len() {
        let mut b = base();
        record(&mut b, 0x700, 0, &full[..end]);
        assert!(
            VisualSnapshot::decode_records(&b).is_err(),
            "accepted scene prefix {end}"
        );
    }
    for (tag, body) in [
        (0x700, full),
        (0x380, text()),
        (0x400, window(&scene(None, None, &[]))),
    ] {
        let mut body = body;
        body.push(0);
        let mut b = base();
        record(&mut b, tag, 0, &body);
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
}

#[test]
fn unsupported_effects_fonts_glyphs_and_versions_fail_explicitly() {
    let mut glyphs = text();
    let len = glyphs.len();
    glyphs[len - 2] = 1;
    let mut font = text();
    font[6] = b'A';
    font.insert(7, 0);
    let mut layout = text();
    put(&mut layout, 15 + 104, 2);
    for body in [glyphs, font, layout] {
        let mut b = base();
        record(&mut b, 0x380, 0, &body);
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
    let mut effect = scene(None, None, &[]);
    put(&mut effect, 36, 1);
    let mut kind = scene(None, None, &[]);
    put(&mut kind, 4, 2);
    let mut version = scene(None, None, &[]);
    version[2] = 2;
    let mut motion = draw(None);
    let len = motion.len();
    motion.splice(len - 2..len - 2, [0x90, 7, 0, 0, 0, 0]);
    for body in [effect, kind, version, scene(Some(&motion), None, &[])] {
        let mut b = base();
        record(&mut b, 0x700, 0, &body);
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
}

#[test]
fn animation_bounds_are_checked_before_advancing() {
    for (at, value) in [(6, 64), (10, 64), (18, 64)] {
        let mut a = animation();
        put(&mut a, at, value);
        let mut b = base();
        record(&mut b, 0x700, 0, &scene(Some(&draw(Some(&a))), None, &[]));
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
}

#[test]
fn nested_scene_limits_and_child_identity_are_checked() {
    let leaf = scene(None, None, &[]);
    for children in [
        vec![(768, leaf.clone())],
        vec![(0, leaf.clone()), (0, leaf.clone())],
    ] {
        let mut b = base();
        record(&mut b, 0x700, 0, &scene(None, None, &children));
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
    let mut nested = leaf;
    for _ in 0..34 {
        nested = scene(None, None, &[(0, nested)]);
    }
    let mut b = base();
    record(&mut b, 0x700, 0, &nested);
    assert!(VisualSnapshot::decode_records(&b).is_err());
    let wide = scene(
        None,
        None,
        &(0..767)
            .map(|slot| (slot, scene(None, None, &[])))
            .collect::<Vec<_>>(),
    );
    let mut b = base();
    for slot in 0..3 {
        record(&mut b, 0x700, slot, &wide);
    }
    assert!(VisualSnapshot::decode_records(&b).is_err());
}

fn image() -> Vec<u8> {
    let mut b = 3u32.to_le_bytes().to_vec();
    b.extend([0; 36]);
    b.extend(b"tiny.pb3\0\0");
    for n in [0u32, 2, 1] {
        b.extend(n.to_le_bytes());
    }
    b
}
fn tiny_pb3() -> Vec<u8> {
    let mut b = vec![0; 52];
    b[..4].copy_from_slice(b"PB3B");
    put(&mut b, 8, 1);
    put(&mut b, 24, 1);
    b[28] = 1;
    b[30] = 2;
    b[32] = 1;
    b[34] = 24;
    put(&mut b, 44, 52);
    put(&mut b, 48, 109);
    for _ in 0..3 {
        b.extend(15u32.to_le_bytes());
    }
    for plane in [[1u8, 2], [3, 4], [5, 6]] {
        for n in [1u32, 2, 0] {
            b.extend(n.to_le_bytes());
        }
        b.push(0xc0);
        b.extend(plane);
    }
    b.extend([0; 12]);
    let len = b.len();
    put(&mut b, 4, len as u32);
    let seed = b[len - 47..len - 3].to_vec();
    let key = [b[len - 3], b[len - 2]];
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
    b
}
struct Tiny;
impl ImageSource for Tiny {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(Some(tiny_pb3()))
    }
}

#[test]
fn resources_use_existing_decoder_filter_and_dimension_checks() {
    let mut image = image();
    put(&mut image, 20, 1);
    put(&mut image, 24, 2);
    put(&mut image, 28, 0x804020);
    let mut b = base();
    record(&mut b, 0x700, 0, &scene(None, Some(&image), &[]));
    let visual = VisualSnapshot::decode_records(&b).unwrap();
    assert!(visual.preview(&mut Empty).is_err());
    let p = visual.preview(&mut Tiny).unwrap();
    let i = p.scene.object(0).unwrap().image.as_ref().unwrap();
    assert_eq!(i.pixels.pixels(), &[5, 1, 0, 255, 6, 2, 0, 255]);
    assert_eq!(i.filter, Some([2, 0x804020, 0]));
    let len = image.len();
    put(&mut image, len - 8, 3);
    let mut b = base();
    record(&mut b, 0x700, 0, &scene(None, Some(&image), &[]));
    assert!(
        VisualSnapshot::decode_records(&b)
            .unwrap()
            .preview(&mut Tiny)
            .is_err()
    );
    // Failure returned no host and did not consume/mutate the saved component.
    assert_eq!(visual.objects()[&0].image.as_ref().unwrap().size, [2, 1]);
    assert!(visual.preview(&mut Tiny).is_ok());
    for dimensions in [[0, 1], [u32::MAX, u32::MAX], [4096, 4097]] {
        let mut oversized = image.clone();
        let len = oversized.len();
        put(&mut oversized, len - 8, dimensions[0]);
        put(&mut oversized, len - 4, dimensions[1]);
        let mut b = base();
        record(&mut b, 0x700, 0, &scene(None, Some(&oversized), &[]));
        assert!(VisualSnapshot::decode_records(&b).is_err());
    }
}

#[test]
fn unmodeled_global_records_are_retained_and_not_applied() {
    let mut b = base();
    chunk(&mut b, 0x880, &[1, 2, 3]);
    let visual = VisualSnapshot::decode_records(&b).unwrap();
    assert_eq!(visual.uninterpreted.len(), 1);
    assert_eq!(visual.uninterpreted[0].tag, 0x880);
}

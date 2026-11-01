#[path = "support/shared_images.rs"]
mod shared_images;
use cmvs_rs::{
    image::{self, ImageSource},
    pb3,
    raster::RgbaImage,
};
use shared_images::SharedImages;
use std::{collections::BTreeMap, error::Error};

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn encrypt(b: &mut [u8]) {
    let n = b.len();
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(b[n - 47 + i]) ^ b[n - 3 + i % 2];
    }
}
fn mutate(mut b: Vec<u8>, at: usize, value: u32) -> Vec<u8> {
    let n = b.len();
    for i in 0..44 {
        b[8 + i] = (b[8 + i] ^ b[n - 3 + i % 2]).wrapping_sub(b[n - 47 + i]);
    }
    put(&mut b, at, value);
    encrypt(&mut b);
    b
}
fn header(kind: u8, w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0; 84];
    b[..4].copy_from_slice(b"PB3B");
    b[28] = kind;
    b[30..32].copy_from_slice(&w.to_le_bytes());
    b[32..34].copy_from_slice(&h.to_le_bytes());
    b[34] = 32;
    b
}
fn finish(mut b: Vec<u8>) -> Vec<u8> {
    // Extra key-sample padding is not part of the compressed streams.
    b.extend([0; 47]);
    encrypt(&mut b);
    b
}
fn delta(w: u16, h: u16, bgra: &[[u8; 4]]) -> Vec<u8> {
    assert_eq!(bgra.len(), usize::from(w) * usize::from(h));
    let mut b = header(5, w, h);
    for channel in 0..4 {
        let mut last = 0u8;
        let data: Vec<_> = bgra
            .iter()
            .map(|p| {
                let d = p[channel].wrapping_sub(last);
                last = p[channel];
                d
            })
            .collect();
        let offset = (b.len() - 84) as u32;
        put(&mut b, 52 + 8 * channel, offset);
        b.extend(vec![0; data.len().div_ceil(8)]);
        let offset = (b.len() - 84) as u32;
        put(&mut b, 56 + 8 * channel, offset);
        b.extend(data);
    }
    finish(b)
}
fn patch(w: u16, h: u16, stem: &[u8], mask: &[u8], bgra: &[[u8; 4]]) -> Vec<u8> {
    let mut body = vec![0; 8];
    put(&mut body, 0, mask.len() as u32);
    body.extend(mask);
    for pixel in bgra {
        body.extend(pixel);
    }
    patch_body(w, h, stem, &body)
}
fn patch_body(w: u16, h: u16, stem: &[u8], body: &[u8]) -> Vec<u8> {
    assert!(stem.len() <= 32);
    let mut b = header(6, w, h);
    put(&mut b, 12, 52);
    put(&mut b, 24, body.len() as u32);
    let key = [
        0xa6, 0x75, 0xf3, 0x9c, 0xc5, 0x69, 0x78, 0xa3, 0x3e, 0xa5, 0x4f, 0x79, 0x59, 0xfe, 0x3a,
        0xc7,
    ];
    b[52..52 + stem.len()].copy_from_slice(stem);
    for i in 0..32 {
        b[52 + i] ^= key[i % 16];
    }
    put(&mut b, 44, body.len().div_ceil(8) as u32);
    b.extend(vec![0; body.len().div_ceil(8)]);
    b.extend(body);
    finish(b)
}
fn base(w: u32, h: u32) -> RgbaImage {
    RgbaImage::new(w, h, [20, 30, 40, 50].repeat((w * h) as usize)).unwrap()
}

#[test]
fn type5_predictor_wraps_and_continues_across_rows_in_bgra_order() {
    let raw = delta(
        2,
        2,
        &[
            [250, 1, 7, 255],
            [2, 0, 255, 0],
            [1, 255, 0, 128],
            [90, 91, 92, 93],
        ],
    );
    let got = pb3::decode(&raw).unwrap();
    assert_eq!(
        got.pixels(),
        &[7, 1, 250, 255, 255, 0, 2, 0, 0, 255, 1, 128, 92, 91, 90, 93]
    );
    assert!(image::decode(&raw).unwrap().unwrap().alpha);
    assert!(pb3::decode(&mutate(raw.clone(), 34, 24)).is_err());
    assert!(pb3::decode(&mutate(raw, 52, u32::MAX)).is_err());
}

#[test]
fn type6_tiles_preserve_base_or_replace_all_channels_and_clip_both_edges() {
    // 9x9: replace 8x8 top left and 1x1 bottom right; keep the two edge strips.
    let mut pixels = vec![[1, 2, 3, 0]; 64];
    pixels.push([100, 110, 120, 130]);
    let raw = patch(9, 9, b"base", &[0x60], &pixels);
    let got = pb3::decode_with_base(&raw, base(9, 9)).unwrap();
    for y in 0..9 {
        for x in 0..9 {
            let want = if x < 8 && y < 8 {
                [3, 2, 1, 0]
            } else if x == 8 && y == 8 {
                [120, 110, 100, 130]
            } else {
                [20, 30, 40, 50]
            };
            assert_eq!(&got.pixels()[(y * 9 + x) * 4..(y * 9 + x + 1) * 4], &want);
        }
    }
    assert_eq!(
        pb3::reference_name(&raw).unwrap(),
        Some(b"base.pb3".to_vec())
    );
    assert!(pb3::decode(&raw).unwrap_err().0.contains("reference image"));
    assert!(pb3::decode_with_base(&raw, base(8, 9)).is_err());
}

#[test]
fn type6_mask_bits_continue_across_tile_rows_and_bytes() {
    // Ten tiles across two rows; only the ninth tile is changed.
    let raw = patch(33, 9, b"base", &[0xff, 0x40], &[[8, 9, 10, 11]; 8]);
    let got = pb3::decode_with_base(&raw, base(33, 9)).unwrap();
    for y in 0..9 {
        for x in 0..33 {
            let want = if y == 8 && (24..32).contains(&x) {
                [10, 9, 8, 11]
            } else {
                [20, 30, 40, 50]
            };
            assert_eq!(&got.pixels()[(y * 33 + x) * 4..(y * 33 + x + 1) * 4], &want);
        }
    }
}

#[test]
fn type6_rejects_bad_masks_streams_names_and_unbounded_allocations() {
    for raw in [
        patch(1, 1, b"base", &[], &[]),
        patch(1, 1, b"base", &[0], &[]),
        patch(1, 1, b"base", &[0x80], &[[1, 2, 3, 4]]),
        patch_body(1, 1, b"base", &[0; 4]),
    ] {
        assert!(pb3::decode_with_base(&raw, base(1, 1)).is_err());
    }
    let raw = patch(1, 1, b"base", &[0], &[[1, 2, 3, 4]]);
    for (at, value) in [
        (12, 0),
        (12, u32::MAX),
        (24, u32::MAX),
        (34, 24),
        (44, u32::MAX),
    ] {
        assert!(pb3::decode_with_base(&mutate(raw.clone(), at, value), base(1, 1)).is_err());
    }
    // Without supplying additional bytes, truncate before the last replacement pixel.
    let short = raw[..raw.len() - 48].to_vec();
    assert!(pb3::decode_with_base(&short, base(1, 1)).is_err());
    assert!(pb3::reference_name(&patch(1, 1, b"", &[0x80], &[])).is_err());
    assert!(pb3::reference_name(&patch(1, 1, &[b'x'; 32], &[0x80], &[])).is_err());
    assert_eq!(
        pb3::reference_name(&patch(1, 1, b"\x81A", &[0x80], &[])).unwrap(),
        Some(b"\x81A.pb3".to_vec())
    );
}

#[derive(Default)]
struct Files {
    files: BTreeMap<Vec<u8>, Vec<u8>>,
    reads: Vec<Vec<u8>>,
    hints: usize,
}
impl ImageSource for Files {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.reads.push(name.to_vec());
        Ok(self.files.get(name).cloned())
    }
    fn cache_next_read(&mut self) -> Result<(), Box<dyn Error>> {
        self.hints += 1;
        Ok(())
    }
}
#[test]
fn contextual_loader_resolves_nested_references_through_the_same_source() {
    let mut source = Files::default();
    source
        .files
        .insert(b"base.pb3".to_vec(), delta(1, 1, &[[1, 2, 3, 4]]));
    source.files.insert(
        b"middle.pb3".to_vec(),
        patch(1, 1, b"base", &[0], &[[5, 6, 7, 8]]),
    );
    source
        .files
        .insert(b"top.pb3".to_vec(), patch(1, 1, b"middle", &[0x80], &[]));
    let got = image::load(&mut source, b"top.pb3").unwrap().unwrap();
    assert_eq!(got.pixels.pixels(), &[7, 6, 5, 8]);
    assert_eq!(
        source.reads,
        [
            b"top.pb3".to_vec(),
            b"middle.pb3".to_vec(),
            b"base.pb3".to_vec()
        ]
    );
    assert_eq!(source.hints, 2);
}

#[test]
fn contextual_loader_rejects_cycles_missing_bases_and_deep_chains() {
    let mut source = Files::default();
    source
        .files
        .insert(b"a.pb3".to_vec(), patch(1, 1, b"A", &[0x80], &[]));
    assert!(
        image::load(&mut source, b"a.pb3")
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    source
        .files
        .insert(b"a.pb3".to_vec(), patch(1, 1, b"missing", &[0x80], &[]));
    assert!(
        image::load(&mut source, b"a.pb3")
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
    for i in 0..17 {
        source.files.insert(
            format!("{i}.pb3").into_bytes(),
            patch(1, 1, format!("{}", i + 1).as_bytes(), &[0x80], &[]),
        );
    }
    assert!(
        image::load(&mut source, b"0.pb3")
            .unwrap_err()
            .to_string()
            .contains("depth")
    );
}

#[test]
fn contextual_loader_checks_chain_byte_budget_before_allocating_decoded_planes() {
    let mut source = Files::default();
    // Each header declares a 32 MiB image. Together with raw bytes this exceeds
    // the chain budget, so the tiny, deliberately incomplete base is never decoded.
    let size = 4096 | (2048 << 16);
    source.files.insert(
        b"top.pb3".to_vec(),
        mutate(patch(1, 1, b"base", &[0x80], &[]), 30, size),
    );
    source.files.insert(
        b"base.pb3".to_vec(),
        mutate(delta(1, 1, &[[1, 2, 3, 4]]), 30, size),
    );
    assert!(
        image::load(&mut source, b"top.pb3")
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
}

#[test]
fn registered_loader_caches_reference_bytes_and_defers_outer_insertion() {
    use cmvs_rs::image::ImageArchives;
    struct Root(std::path::PathBuf);
    impl Drop for Root {
        fn drop(&mut self) {
            for name in ["base.pb3", "top.pb3"] {
                let _ = std::fs::remove_file(self.0.join("raw").join(name));
            }
            let _ = std::fs::remove_dir(self.0.join("raw"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }
    let root = Root(std::env::temp_dir().join(format!("cmvs-type6-cache-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    std::fs::create_dir(root.0.join("raw")).unwrap();
    let original = delta(1, 1, &[[1, 2, 3, 4]]);
    std::fs::write(root.0.join("raw/base.pb3"), &original).unwrap();
    std::fs::write(
        root.0.join("raw/top.pb3"),
        patch(1, 1, b"base", &[0x80], &[]),
    )
    .unwrap();
    let paths = BTreeMap::from([((0x2011, 0), b"raw\\".to_vec())]);
    let mut archives = ImageArchives::new(&root.0).unwrap();
    archives.with_paths(&paths).cache_next_read().unwrap();
    assert_eq!(
        image::load(&mut archives.with_paths(&paths), b"top.pb3")
            .unwrap()
            .unwrap()
            .pixels
            .pixels(),
        &[3, 2, 1, 4]
    );
    // Every type-6 decode implicitly arms the base read. It must survive deletion.
    std::fs::remove_file(root.0.join("raw/base.pb3")).unwrap();
    assert_eq!(
        image::load(&mut archives.with_paths(&paths), b"top.pb3")
            .unwrap()
            .unwrap()
            .pixels
            .pixels(),
        &[3, 2, 1, 4]
    );
    // The top file was explicitly cached too; even its removal and changed roots keep it.
    std::fs::remove_file(root.0.join("raw/top.pb3")).unwrap();
    let empty = BTreeMap::new();
    archives.with_paths(&empty).cache_next_read().unwrap();
    assert_eq!(
        image::load(&mut archives.with_paths(&empty), b"TOP.PB3")
            .unwrap()
            .unwrap()
            .pixels
            .pixels(),
        &[3, 2, 1, 4]
    );
}

#[test]
fn reference_failure_preserves_pending_vm_stack_results_and_previous_image() {
    use cmvs_rs::{
        machine::{Machine, Step},
        ps3::Script,
        scene::SceneHost,
    };
    fn push(code: &mut Vec<u8>, v: u32) {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(v.to_le_bytes());
    }
    fn system(vm: &mut Machine<'_>) {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    }
    let mut code = vec![];
    push(&mut code, 3);
    code.extend(0x2020u16.to_le_bytes());
    for offset in [0, 9] {
        push(&mut code, offset);
        push(&mut code, 3);
        code.extend(0x2030u16.to_le_bytes());
    }
    let strings = b"base.pb3\0top.pb3\0";
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
    payload.extend(strings);
    let mut raw = b"PS2A".to_vec();
    for value in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        strings.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(value.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    let mut source = Files::default();
    source
        .files
        .insert(b"base.pb3".to_vec(), delta(1, 1, &[[1, 2, 3, 4]]));
    source
        .files
        .insert(b"top.pb3".to_vec(), patch(1, 1, b"missing", &[0x80], &[]));
    system(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    system(&mut vm);
    scene.shared_image(&mut vm, &mut source).unwrap();
    system(&mut vm);
    let before = scene.objects().clone();
    let pc = vm.pc();
    let sp = vm.stack().len_bytes();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    assert!(scene.shared_image(&mut vm, &mut source).is_err());
    assert_eq!(vm.pc(), pc);
    assert_eq!(vm.pending_system(), Some((pc, 0x30)));
    assert_eq!(vm.stack().len_bytes(), sp);
    assert_eq!(vm.state().system_integer(0), Some(123));
    assert_eq!(scene.objects(), &before);
}

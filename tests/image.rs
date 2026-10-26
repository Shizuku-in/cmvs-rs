use cmvs_rs::{
    cpz::{Cpz6Index, data_checksum},
    image::{DecodedImage, ImageArchives, ImageSource, decode},
    machine::{Machine, Step},
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
        name: b"a.pb3".to_vec(),
        pixels: opaque_pixels(),
        alpha: false,
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
        .chunks_exact(2)
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

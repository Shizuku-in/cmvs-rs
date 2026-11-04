//! Engine snapshot record stream: canonical order, typed codecs and raw payloads.
use cmvs_rs::{
    save::Csv2Save,
    snapshot::{EngineSnapshot, parse_records},
};

/// Decodes and re-encodes section 0 of external original saves; the record stream must be
/// byte-identical. Inputs are read-only.
#[test]
#[ignore = "requires explicit CMVS_SAVE_TEST_INPUTS pointing to external read-only CSV2 saves"]
fn external_snapshots_reencode_byte_identically() {
    let inputs = std::env::var_os("CMVS_SAVE_TEST_INPUTS").expect("CMVS_SAVE_TEST_INPUTS");
    let mut checked = 0;
    for path in std::env::split_paths(&inputs) {
        let save = Csv2Save::decode(&std::fs::read(&path).unwrap()).unwrap();
        let snapshot = EngineSnapshot::decode(&save.snapshot).unwrap();
        let encoded = snapshot.encode().unwrap();
        if encoded != save.snapshot {
            let at = encoded
                .iter()
                .zip(&save.snapshot)
                .position(|(a, b)| a != b)
                .unwrap_or(encoded.len().min(save.snapshot.len()));
            let record = parse_records(&save.snapshot)
                .unwrap()
                .into_iter()
                .rfind(|r| r.offset <= at)
                .unwrap();
            panic!(
                "{}: differs at {at} in record {:#x} index {:?} (record offset {}, body {:?}); lengths {} vs {}",
                path.display(),
                record.tag,
                record.index,
                record.offset,
                record.body,
                encoded.len(),
                save.snapshot.len()
            );
        }
        println!(
            "{}: {} snapshot bytes identical",
            path.display(),
            encoded.len()
        );
        checked += 1;
    }
    assert!(checked > 0);
}

fn short(out: &mut Vec<u8>, tag: u16, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    out.extend((body.len() as u16).to_le_bytes());
    out.extend(body);
}
fn long(out: &mut Vec<u8>, tag: u16, index: Option<u16>, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    if let Some(index) = index {
        out.extend(index.to_le_bytes());
    }
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
}
fn word(n: u32) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

/// A synthetic stream in writer order; `objects` go between 0x283 and 0x6e0.
fn stream(objects: &[(u16, Option<u16>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, text) in [
        (0x100, &b"- save -\0"[..]),
        (0x101, b"- scene -\0"),
        (0x102, b"main.ps3\0"),
        (0x103, b"intproc.ps3\0"),
        (0x104, b"\0"),
        (0x105, b"\0"),
    ] {
        short(&mut out, tag, text);
    }
    for (tag, n) in [(0x112, 100), (0x113, 50), (0x114, 0), (0x115, 0)] {
        short(&mut out, tag, &word(n));
    }
    for (tag, n) in [(0x106, 0), (0x107, 12), (0x108, 0)] {
        short(&mut out, tag, &word(n));
    }
    short(&mut out, 0x109, &[7; 40]);
    short(&mut out, 0x10a, &word(1));
    short(&mut out, 0x10b, &[0xff; 1796]);
    for (tag, n) in [(0x200, 0x123456), (0x201, 1), (0x202, 1), (0x203, 1)] {
        short(&mut out, tag, &word(n));
    }
    short(&mut out, 0x240, &word(8));
    short(&mut out, 0x241, &word(0));
    long(&mut out, 0x242, None, &[0; 256]);
    long(&mut out, 0x243, None, &[0; 0x10000]);
    short(&mut out, 0x248, &[0; 44]);
    long(&mut out, 0x280, None, &[0; 8192]);
    long(&mut out, 0x281, None, &[0; 4096]);
    long(&mut out, 0x282, None, &[1; 256]);
    long(&mut out, 0x283, None, &[0; 64]);
    for (tag, index, body) in objects {
        long(&mut out, *tag, *index, body);
    }
    long(&mut out, 0x6e0, None, b"six-e0");
    for tag in 0x880..=0x883 {
        long(&mut out, tag, None, &[tag as u8; 10]);
    }
    long(&mut out, 0x900, None, b"");
    out.extend(0x304u16.to_le_bytes());
    for name in [&b"bgm01\0"[..], b"\0"] {
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(name);
    }
    long(&mut out, 0x310, None, &[0; 312]);
    long(&mut out, 0x318, None, &[1, 0, 0, 0, 0, 0]);
    out
}

#[test]
fn synthetic_stream_round_trips_with_raw_objects_and_effects() {
    let objects = [
        (0x6a0, Some(1), b"fade".to_vec()),
        (0x8c8, Some(3), b"eight-c8".to_vec()),
        (0x540, None, b"history".to_vec()),
        (0x620, None, b"mask".to_vec()),
    ];
    let bytes = stream(&objects);
    let snapshot = EngineSnapshot::decode(&bytes).unwrap();
    assert_eq!(snapshot.encode().unwrap(), bytes);
    assert_eq!(snapshot.script_names[1], b"intproc.ps3");
    assert_eq!(snapshot.timers, [0x0707_0707; 10]);
    assert_eq!(snapshot.bgm, [b"bgm01".to_vec(), Vec::new()]);
    assert_eq!(snapshot.raw_objects.len(), 3);
    assert_eq!(snapshot.effects[2].as_deref(), Some(&b"mask"[..]));
    assert!(snapshot.effects[0].is_none());
}

#[test]
fn realive_engine_round_trips_scene_v4_and_rejects_other_layouts() {
    use cmvs_rs::snapshot::SnapshotLayout;
    // Native scene v4: marker/version, kind/shown/flags/width/height,
    // 44-byte inactive effect block and terminator.
    let scene = |flags: u32| {
        let mut out = vec![255, 255, 4, 0];
        for n in [0u32, 1, flags, 1280, 720] {
            out.extend(n.to_le_bytes());
        }
        out.extend([0; 44]);
        out
    };
    let mut child = scene(0);
    child.extend(0xf0f0u16.to_le_bytes());
    let mut parent = scene(0x8000_0000);
    long(&mut parent, 0x700, Some(1023), &child);
    parent.extend(0xf0f0u16.to_le_bytes());
    let bytes = stream(&[(0x700, Some(255), parent)]);
    let layout = SnapshotLayout::TextV3SceneV4;
    let snapshot = EngineSnapshot::decode_with_layout(&bytes, layout).unwrap();
    assert_eq!(
        snapshot.objects[&255].children[&1023].logical_size,
        [1280, 720]
    );
    assert_eq!(snapshot.encode_with_layout(layout).unwrap(), bytes);
    assert!(EngineSnapshot::decode(&bytes).is_err());
    assert!(snapshot.encode().is_err());
}

#[test]
fn records_out_of_writer_order_or_beyond_tables_are_rejected() {
    let swapped = [
        (0x8c8, Some(1), b"b".to_vec()),
        (0x6a0, Some(0), b"a".to_vec()),
    ];
    let duplicate = [
        (0x6a0, Some(0), b"a".to_vec()),
        (0x6a0, Some(0), b"b".to_vec()),
    ];
    let beyond = [(0x6a0, Some(2), b"a".to_vec())];
    for objects in [&swapped[..], &duplicate, &beyond] {
        assert!(EngineSnapshot::decode(&stream(objects)).is_err());
    }
    // A missing head record or trailing record after 0x318 fails too.
    let mut bytes = stream(&[]);
    bytes.extend(&stream(&[])[..13]);
    assert!(EngineSnapshot::decode(&bytes).is_err());
    let bytes = stream(&[]);
    let without_title = &bytes[13..];
    assert!(EngineSnapshot::decode(without_title).is_err());
}

fn v1_stream(objects: &[(u16, Option<u16>, Vec<u8>)]) -> Vec<u8> {
    let bytes = stream(objects);
    let mut out = Vec::new();
    for record in parse_records(&bytes).unwrap() {
        match record.tag {
            0x6e0 => {}
            0x304 => short(&mut out, 0x300, b"bgm01\0"),
            _ => out.extend(&bytes[record.offset..record.body.end]),
        }
    }
    out
}

#[test]
fn text_v1_scene_v1_wire_records_roundtrip_and_reject_unsupported_or_out_of_range_state() {
    use cmvs_rs::snapshot::SnapshotLayout;
    let layout = SnapshotLayout::TextV1SceneV1;
    let mut text = 1u16.to_le_bytes().to_vec();
    text.extend(196u32.to_le_bytes());
    text.push(0);
    let mut fields = [0u8; 304];
    fields[104..108].copy_from_slice(&1u32.to_le_bytes()); // font mode
    fields[160..164].copy_from_slice(&148u32.to_le_bytes()); // ruby block size
    text.extend(fields);
    text.extend(0u16.to_le_bytes());
    let scene = |kind: u32, flags: u32| {
        let mut out = vec![255, 255, 1, 0];
        for word in [kind, 1, flags, 0, 800, 600] {
            out.extend(word.to_le_bytes());
        }
        out
    };
    let mut child = scene(0, 0);
    child.extend(0xf0f0u16.to_le_bytes());
    let mut draw = vec![3, 0, 0, 0];
    for word in cmvs_rs::scene::DrawState::default().words {
        draw.extend(word.to_le_bytes());
    }
    draw.extend([0; 20]);
    let mut frames = 1u16.to_le_bytes().to_vec();
    frames.extend([0; 656]); // four header words and 32 twenty-byte entries
    long(&mut draw, 0x793, None, &frames);
    let mut world = 1u16.to_le_bytes().to_vec();
    world.extend([0; 28]);
    for _ in 0..8 {
        for word in [u32::MAX, 0, 0, 0, 0x21, 1350] {
            world.extend(word.to_le_bytes());
        }
    }
    long(&mut draw, 0x795, None, &world);
    draw.extend(0x7789u16.to_le_bytes());
    let mut image = Vec::new();
    for word in [2u32, 0, 0, 0, 0, 1, 1, 0xffe0e0, 0] {
        image.extend(word.to_le_bytes());
    }
    image.extend(b"synthetic.pb3\0\0");
    for word in [0u32, 2, 1] {
        image.extend(word.to_le_bytes());
    }
    let mut parent = scene(1, 0x80000000);
    long(&mut parent, 0x740, None, &image);
    long(&mut parent, 0x780, None, &draw);
    long(&mut parent, 0x700, Some(255), &child);
    parent.extend(0xf0f0u16.to_le_bytes());
    let mut window = vec![0, 0, 1, 0];
    window.extend([0; 1280]);
    window.extend((child.len() as u32).to_le_bytes());
    window.extend(&child);
    let bytes = v1_stream(&[
        (0x380, Some(0), text),
        (0x400, Some(0), window),
        (0x700, Some(20), parent),
    ]);
    let saved = EngineSnapshot::decode_with_layout(&bytes, layout).unwrap();
    assert_eq!(saved.encode_with_layout(layout).unwrap(), bytes);
    assert!(EngineSnapshot::decode(&bytes).is_err());
    assert_eq!(
        saved.objects[&20].image.as_ref().unwrap().filter,
        Some([1, 0xffe0e0, 0])
    );
    assert_eq!(
        saved.objects[&20]
            .children
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [255]
    );
    assert!(saved.record_6e0.is_empty());
    assert_eq!(saved.bgm, [b"bgm01".to_vec(), vec![]]);
    for mutate in [0, 1, 2, 3] {
        let mut bad = saved.clone();
        match mutate {
            0 => {
                let c = bad
                    .objects
                    .get_mut(&20)
                    .unwrap()
                    .children
                    .remove(&255)
                    .unwrap();
                bad.objects.get_mut(&20).unwrap().children.insert(256, c);
            }
            1 => bad.texts[8] = bad.texts[0].clone(),
            2 => bad.bgm[1] = b"second".to_vec(),
            _ => {
                bad.objects
                    .get_mut(&20)
                    .unwrap()
                    .draw
                    .as_mut()
                    .unwrap()
                    .animation
                    .as_mut()
                    .unwrap()
                    .frames[32]
                    .duration = 1
            }
        }
        assert!(bad.encode_with_layout(layout).is_err());
    }
    for end in [1, 3, bytes.len() - 1] {
        assert!(EngineSnapshot::decode_with_layout(&bytes[..end], layout).is_err());
    }
}

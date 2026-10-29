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

use cmvs_rs::{lzss, ps3::Script};

fn fixture(compressed: bool, key: u32) -> Vec<u8> {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend_from_slice(&[0, 0x20]);
    payload.extend_from_slice(b"hello\0");
    let plain = if compressed {
        payload
            .chunks(8)
            .flat_map(|c| std::iter::once(255).chain(c.iter().copied()))
            .collect()
    } else {
        payload.clone()
    };
    let xor = ((key >> 3).wrapping_add(key >> 24)) as u8;
    let shift = (key >> 20) % 5 + 1;
    let encoded: Vec<u8> = plain
        .iter()
        .map(|b| (b.rotate_left(shift) ^ xor).wrapping_add(124))
        .collect();
    let mut raw = b"PS2A".to_vec();
    for field in [
        48,
        0,
        key,
        1,
        2,
        0,
        6,
        0,
        if compressed { encoded.len() as u32 } else { 0 },
        12,
        0,
    ] {
        raw.extend_from_slice(&field.to_le_bytes());
    }
    raw.extend(encoded);
    raw
}

#[test]
fn decode_both_storage_modes_and_key_rotations() {
    for key in [
        0,
        0x00100000,
        0x00200000,
        0x00300000,
        0x00400000,
        0x5d3f3020,
        u32::MAX,
    ] {
        for compressed in [true, false] {
            let script = Script::decode(&fixture(compressed, key)).unwrap();
            assert_eq!(script.labels(), vec![0]);
            assert_eq!(script.code(), &[0, 0x20]);
            assert!(script.unknown().is_empty());
            let strings = script.strings().unwrap();
            assert_eq!(strings[0].bytes, b"hello");
            assert_eq!(script.payload().len(), 12);
        }
    }
}

#[test]
fn overlapping_backreference() {
    assert_eq!(lzss::decode(b"\x01A\xdf\xe1", 4).unwrap(), b"AAAA");
}

#[test]
fn ring_wrap_and_literal_flag_reload() {
    let expected: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
    let input: Vec<u8> = expected
        .chunks(8)
        .flat_map(|c| std::iter::once(255).chain(c.iter().copied()))
        .collect();
    assert_eq!(lzss::decode(&input, expected.len()).unwrap(), expected);
}

#[test]
fn malformed_lzss_and_size_limits() {
    for (input, expected) in [
        (&b"\x00\xdf"[..], 4),
        (&b"\xff"[..], 1),
        (&b"\x01A\xdf\xe1"[..], 3),
        (&b""[..], 1),
    ] {
        assert!(lzss::decode(input, expected).is_err());
    }
    assert!(lzss::decode(&[], usize::MAX).is_err());
    assert_eq!(lzss::decode(&[], 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn every_truncated_prefix_is_rejected() {
    let raw = fixture(true, 0x5d3f3020);
    for end in 0..raw.len() {
        assert!(Script::decode(&raw[..end]).is_err(), "prefix {end}");
    }
}

#[test]
fn bad_headers_rejected_without_panics() {
    for (at, value) in [
        (4, 0),
        (16, u32::MAX),
        (20, 0),
        (28, 7),
        (32, 2),
        (36, u32::MAX),
        (40, u32::MAX),
    ] {
        let mut raw = fixture(true, 0);
        raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Script::decode(&raw).is_err(), "field {at}");
    }
    let mut raw = fixture(true, 0);
    raw[0] = 0;
    assert!(Script::decode(&raw).is_err());
}

#[test]
fn label_and_text_validation() {
    let mut raw = fixture(false, 0);
    // Decode a deliberately out-of-range label through the byte transform.
    raw[48] = (200u8.rotate_left(1)).wrapping_add(124);
    assert!(Script::decode(&raw).is_err());
    let mut raw = fixture(false, 0);
    *raw.last_mut().unwrap() = (b'x'.rotate_left(1)).wrapping_add(124);
    assert!(Script::decode(&raw).unwrap().strings().is_err());
}

#[test]
fn reports_trailing_bytes_outside_compressed_payload() {
    let mut raw = fixture(true, 0);
    raw.extend_from_slice(b"tail");
    assert_eq!(Script::decode(&raw).unwrap().trailing_file_bytes, 4);
}

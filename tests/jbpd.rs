use cmvs_rs::jbpd;

fn pack(bits: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; bits.len().div_ceil(32) * 4];
    for (i, &b) in bits.iter().enumerate() {
        bytes[i / 8] |= b << (i % 8);
    }
    bytes
}
fn symbol(bits: &mut Vec<u8>, n: u8) {
    for i in (0..4).rev() {
        bits.push((n >> i) & 1);
    }
}
fn coefficient(bits: &mut Vec<u8>, v: i16) {
    let n = 16 - v.unsigned_abs().leading_zeros() as u8;
    symbol(bits, n);
    if n != 0 {
        let raw = if v < 0 {
            (i32::from(v) + (1 << n) - 1) as u16
        } else {
            v as u16
        };
        for i in (0..n).rev() {
            bits.push(((raw >> i) & 1) as u8);
        }
    }
}
/// Equal Huffman frequencies give symbols 0..15 their four-bit binary codes.
fn frame(dc_values: [i16; 6], ac_blocks: [Vec<u8>; 6], mask: bool) -> Vec<u8> {
    let mut dc = vec![];
    let mut old = 0;
    for v in dc_values {
        coefficient(&mut dc, v - old);
        old = v;
    }
    let dc = pack(&dc);
    let ac = pack(&ac_blocks.concat());
    let mut data = vec![0; 48];
    data[..4].copy_from_slice(b"JBPD");
    for (at, v) in [
        (4, 44),
        (8, 0x58c00002 | if mask { 0x1000 } else { 0 }),
        (16, 16 | (16 << 16)),
        (20, 24),
        (28, dc.len() as u32),
        (32, ac.len() as u32),
    ] {
        data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    for _ in 0..32 {
        data.extend(1u32.to_le_bytes());
    }
    data.extend([0u8; 16]);
    data.extend([1u8; 128]);
    data.extend([1, 1, 1, 0, 1, 1, 0, 0]);
    if mask {
        data.extend([255, 0, 0, 0]);
    }
    data.extend(dc);
    data.extend(ac);
    data
}
fn end() -> Vec<u8> {
    vec![1, 1, 1, 1]
}
fn flat(values: [i16; 6], mask: bool) -> Vec<u8> {
    frame(values, std::array::from_fn(|_| end()), mask)
}

#[test]
fn neutral_luma_quadrants_and_subsampled_chroma() {
    let image = jbpd::decode(&flat([0, 64, 128, 192, 0, 0], false)).unwrap();
    for (x, y, v) in [(0, 0, 128), (15, 0, 136), (0, 15, 144), (15, 15, 152)] {
        let at = (y * 16 + x) * 4;
        assert_eq!(&image.pixels()[at..at + 4], &[v, v, v, 255]);
    }
    let image = jbpd::decode(&flat([0, 0, 0, 0, 128, -128], false)).unwrap();
    assert!(
        image
            .pixels()
            .chunks_exact(4)
            .all(|p| p == [105, 135, 156, 255])
    );
}
#[test]
fn reuse_mask_still_allows_independent_frame_decoding() {
    assert_eq!(
        jbpd::decode(&flat([32; 6], true)).unwrap(),
        jbpd::decode(&flat([32; 6], false)).unwrap()
    );
}
#[test]
fn ac_coefficients_and_zero_runs_change_the_correct_block() {
    let mut ac = vec![];
    coefficient(&mut ac, 64);
    ac.extend(end());
    let mut run = vec![];
    symbol(&mut run, 0);
    run.push(0);
    coefficient(&mut run, 64);
    run.extend(end());
    let a = jbpd::decode(&frame(
        [0; 6],
        [ac, end(), end(), end(), end(), end()],
        false,
    ))
    .unwrap();
    let b = jbpd::decode(&frame(
        [0; 6],
        [run, end(), end(), end(), end(), end()],
        false,
    ))
    .unwrap();
    assert_ne!(a, b);
    assert_ne!(a.pixels()[0], a.pixels()[7 * 4]);
    assert_eq!(&a.pixels()[8 * 4..8 * 4 + 4], &[128, 128, 128, 255]);
}
#[test]
fn truncated_streams_headers_and_tables_return_errors() {
    let data = flat([0; 6], false);
    for n in 0..data.len() {
        assert!(jbpd::decode(&data[..n]).is_err(), "prefix {n}");
    }
    for (at, value) in [
        (4, u32::MAX),
        (8, 0),
        (16, 0),
        (16, 17 | (16 << 16)),
        (20, 32),
        (28, u32::MAX),
        (32, u32::MAX),
    ] {
        let mut bad = data.clone();
        bad[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(jbpd::decode(&bad).is_err());
    }
    for at in [48 + 272, 48 + 272 + 2, 48 + 272 + 4, 48 + 272 + 5] {
        let mut bad = data.clone();
        bad[at] = 255;
        assert!(jbpd::decode(&bad).is_err());
    }
}
#[test]
fn malformed_huffman_and_unbounded_zero_runs_are_rejected() {
    let mut bad = flat([0; 6], false);
    bad[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(jbpd::decode(&bad).is_err());
    let mut run = vec![];
    symbol(&mut run, 0);
    run.extend([1; 16]);
    run.push(0);
    let bad = frame([0; 6], [run, end(), end(), end(), end(), end()], false);
    assert!(jbpd::decode(&bad).is_err());
}

#[test]
fn cmv_frame_checks_container_dimensions_lengths_and_index() {
    use cmvs_rs::cmv::{CmvEntry, CmvHeader, CmvIndex};
    use std::io::Cursor;
    let data = flat([0; 6], false);
    let mut index = CmvIndex {
        header: CmvHeader {
            resource_offset: 0,
            file_size: data.len() as u32,
            unused: 0,
            max_sequence: 1,
            rate: 24,
            rate_time: 1,
            width: 16,
            height: 16,
            bit_count: 24,
            audio_flag: 0,
        },
        entries: vec![CmvEntry {
            sequence: 0,
            stored_size: data.len() as u32,
            decoded_size: 16 * 16 * 3,
            kind: 2,
            offset: 0,
        }],
    };
    assert_eq!(
        index
            .decode_frame(&mut Cursor::new(&data), 0)
            .unwrap()
            .pixels()
            .len(),
        16 * 16 * 4
    );
    assert!(index.decode_frame(&mut Cursor::new(&data), 1).is_err());
    assert!(
        index
            .decode_frame(&mut Cursor::new(&data[..data.len() - 1]), 0)
            .is_err()
    );
    index.entries[0].decoded_size = 0;
    assert!(index.decode_frame(&mut Cursor::new(&data), 0).is_err());
    index.entries[0].decoded_size = 16 * 16 * 3;
    index.header.width = 32;
    index.entries[0].decoded_size = 32 * 16 * 3;
    assert!(index.decode_frame(&mut Cursor::new(&data), 0).is_err());
    index.header.width = u32::MAX;
    index.header.height = u32::MAX;
    assert!(index.decode_frame(&mut Cursor::new(&data), 0).is_err());
}

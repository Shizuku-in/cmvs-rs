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
fn partial_mcus_are_cropped_and_the_reuse_mask_covers_the_rounded_grid() {
    let full = jbpd::decode(&flat([0, 64, 128, 192, 32, -32], false)).unwrap();
    for mask in [false, true] {
        let mut data = flat([0, 64, 128, 192, 32, -32], mask);
        data[16..20].copy_from_slice(&(12u32 | (10 << 16)).to_le_bytes());
        let image = jbpd::decode(&data).unwrap();
        assert_eq!((image.width(), image.height()), (12, 10));
        for y in 0..10 {
            assert_eq!(
                &image.pixels()[y * 48..y * 48 + 48],
                &full.pixels()[y * 64..y * 64 + 48]
            );
        }
    }
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
        alpha: false,
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

/// Synthetic CMV8 JBPE: `width`x16 with one tile, all AC blocks ended immediately, a third
/// (alpha) quantization table whose DC factor is `alpha_q`, and an explicit DC count.
fn jbpe(dc_values: &[i16], width: u16, mask: bool, alpha_q: u8, dc_count: usize) -> Vec<u8> {
    let mut dc = vec![];
    let mut old = 0;
    for &v in dc_values {
        coefficient(&mut dc, v - old);
        old = v;
    }
    let dc = pack(&dc);
    let ac = pack(&end().repeat(dc_values.len()));
    let mcus = usize::from(width).div_ceil(16);
    let mut data = vec![0; 48];
    data[..4].copy_from_slice(b"JBPE");
    for (at, v) in [
        (4, 44),
        (8, 0x58c00002 | if mask { 0x1000 } else { 0 }),
        (16, u32::from(width) | (16 << 16)),
        (20, 32),
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
    let mut alpha_table = [1u8; 64];
    alpha_table[0] = alpha_q;
    data.extend(alpha_table);
    data.extend((dc_count as u32).to_le_bytes());
    data.extend([1, 1, 1, 0, 1, mcus as u8, 0, 0]);
    if mask {
        // The mask covers the MCU grid rounded up to 16 pixels, padded to four bytes.
        data.extend([255, 0, 0, 0]);
    }
    data.extend(dc);
    data.extend(ac);
    data
}
fn alpha_mcu(alpha: [i16; 4]) -> [i16; 10] {
    [0, 0, 0, 0, 0, 0, alpha[0], alpha[1], alpha[2], alpha[3]]
}

#[test]
fn jbpe_alpha_blocks_follow_color_and_use_the_third_table() {
    // A flat block's sample is DC / 8; alpha has no +128 level shift.
    let data = jbpe(&alpha_mcu([800, -800, 2040, 2400]), 16, false, 1, 10);
    let image = jbpd::decode_alpha(&data).unwrap();
    for (x, y, a) in [(0, 0, 100), (15, 0, 0), (0, 15, 255), (15, 15, 255)] {
        let at = (y * 16 + x) * 4;
        assert_eq!(&image.pixels()[at..at + 4], &[128, 128, 128, a]);
    }
    let doubled = jbpd::decode_alpha(&jbpe(&alpha_mcu([400; 4]), 16, false, 2, 10)).unwrap();
    assert!(doubled.pixels().chunks_exact(4).all(|p| p[3] == 100));
    assert_eq!(
        jbpd::decode_alpha(&jbpe(&alpha_mcu([800; 4]), 16, true, 1, 10)).unwrap(),
        jbpd::decode_alpha(&jbpe(&alpha_mcu([800; 4]), 16, false, 1, 10)).unwrap()
    );
}

#[test]
fn jbpe_clips_partial_mcus_and_rejects_unverified_alpha_or_layout() {
    let mut values = alpha_mcu([800; 4]).to_vec();
    values.extend(alpha_mcu([1600; 4]));
    let image = jbpd::decode_alpha(&jbpe(&values, 17, true, 1, 20)).unwrap();
    assert_eq!((image.width(), image.height()), (17, 16));
    assert_eq!(image.pixels()[15 * 4 + 3], 100);
    assert_eq!(image.pixels()[16 * 4 + 3], 200);
    // Alpha samples above 383 or below -128 leave the verified clamp table.
    for alpha in [3200, -1200] {
        assert!(jbpd::decode_alpha(&jbpe(&alpha_mcu([alpha; 4]), 16, false, 1, 10)).is_err());
    }
    let good = jbpe(&alpha_mcu([0; 4]), 16, false, 1, 10);
    assert!(jbpd::decode_alpha(&jbpe(&alpha_mcu([0; 4]), 16, false, 1, 6)).is_err());
    assert!(jbpd::decode(&good).is_err());
    let mut jbpd_signature = good.clone();
    jbpd_signature[..4].copy_from_slice(b"JBPD");
    assert!(jbpd::decode_alpha(&jbpd_signature).is_err());
    let mut depth = good.clone();
    depth[20..24].copy_from_slice(&24u32.to_le_bytes());
    assert!(jbpd::decode_alpha(&depth).is_err());
    for n in 0..good.len() {
        assert!(jbpd::decode_alpha(&good[..n]).is_err(), "prefix {n}");
    }
}

#[test]
fn cmv8_scene_index_reads_alpha_frames_but_the_op_reader_does_not() {
    use cmvs_rs::cmv::CmvIndex;
    use std::io::Cursor;
    let frame = jbpe(&alpha_mcu([800; 4]), 16, false, 1, 10);
    let resource = 44 + 2 * 20;
    let mut data = vec![0; resource];
    data[..4].copy_from_slice(b"CMV8");
    let total = (resource + frame.len()) as u32;
    for (at, v) in [
        (4, resource as u32),
        (8, total),
        (16, 1),
        (20, 15),
        (24, 1),
        (28, 16),
        (32, 16),
        (36, 32),
        (40, 2),
        // Frame 0, then the empty final audio entry.
        (48, frame.len() as u32),
        (52, 16 * 16 * 4 + 54),
        (56, 2),
        (64, 1),
        (80, frame.len() as u32),
    ] {
        data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    data.extend(&frame);
    assert!(CmvIndex::read(&mut Cursor::new(&data)).is_err());
    let index = CmvIndex::read_scene(&mut Cursor::new(&data)).unwrap();
    assert!(index.alpha);
    let image = index.decode_frame(&mut Cursor::new(&data), 0).unwrap();
    assert!(
        image
            .pixels()
            .chunks_exact(4)
            .all(|p| p == [128, 128, 128, 100])
    );
    let mut opaque_depth = data.clone();
    opaque_depth[36..40].copy_from_slice(&24u32.to_le_bytes());
    let index = CmvIndex::read_scene(&mut Cursor::new(&opaque_depth)).unwrap();
    assert!(
        index
            .decode_frame(&mut Cursor::new(&opaque_depth), 0)
            .is_err()
    );
}

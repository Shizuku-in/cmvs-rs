use cmvs_rs::{pb3, raster::RgbaImage};
use std::{fs, process::Command};
fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
struct Plane {
    tile_flags: Vec<u8>,
    solids: Vec<u8>,
    literals: usize,
    lz_flags: Vec<u8>,
    data: Vec<u8>,
}
fn plane(tiles: &[bool], solids: &[u8], literals: &[u8]) -> Plane {
    let mut tile_flags = vec![0; tiles.len().div_ceil(8)];
    for (i, solid) in tiles.iter().enumerate() {
        if *solid {
            tile_flags[i / 8] |= 128 >> (i % 8);
        }
    }
    Plane {
        tile_flags,
        solids: solids.to_vec(),
        literals: literals.len(),
        lz_flags: vec![0; literals.len().div_ceil(8)],
        data: literals.to_vec(),
    }
}
fn encrypt(b: &mut [u8]) {
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
}
fn mutate(mut b: Vec<u8>, at: usize, value: u32) -> Vec<u8> {
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = (b[8 + i] ^ key[i % 2]).wrapping_sub(seed[i]);
    }
    put(&mut b, at, value);
    encrypt(&mut b);
    b
}
fn fixture(w: u16, h: u16, block: u32, planes: Vec<Plane>) -> Vec<u8> {
    let n = planes.len();
    let mut flags = vec![];
    for p in &planes {
        let mut b = vec![0; 12];
        put(&mut b, 0, p.tile_flags.len() as u32);
        put(&mut b, 4, p.solids.len() as u32);
        put(&mut b, 8, p.literals as u32);
        b.extend(&p.tile_flags);
        b.extend(&p.solids);
        b.extend(&p.lz_flags);
        flags.push(b);
    }
    let data_at = 52 + 4 * n + flags.iter().map(Vec::len).sum::<usize>();
    let mut b = vec![0; 52];
    b[..4].copy_from_slice(b"PB3B");
    put(&mut b, 8, 1);
    put(&mut b, 24, block);
    b[28] = 1;
    b[30..32].copy_from_slice(&w.to_le_bytes());
    b[32..34].copy_from_slice(&h.to_le_bytes());
    b[34..36].copy_from_slice(&((n * 8) as u16).to_le_bytes());
    put(&mut b, 44, 52);
    put(&mut b, 48, data_at as u32);
    for f in &flags {
        b.extend((f.len() as u32).to_le_bytes());
    }
    for f in flags {
        b.extend(f);
    }
    for p in &planes {
        b.extend((p.data.len() as u32).to_le_bytes());
    }
    for p in planes {
        b.extend(p.data);
    }
    let len = b.len() as u32;
    put(&mut b, 4, len);
    assert!(b.len() >= 99);
    encrypt(&mut b);
    b
}
fn simple() -> Vec<u8> {
    fixture(
        2,
        2,
        2,
        vec![
            plane(&[false], &[], &[1, 2, 3, 4]),
            plane(&[false], &[], &[11, 12, 13, 14]),
            plane(&[false], &[], &[21, 22, 23, 24]),
        ],
    )
}

#[test]
fn literal_planes_decode_bgr_to_top_down_rgba() {
    let raw = simple();
    let h = pb3::Header::parse(&raw).unwrap();
    assert_eq!(
        (h.kind, h.width, h.height, h.bpp, h.block),
        (1, 2, 2, 24, 2)
    );
    let image = pb3::decode(&raw).unwrap();
    assert_eq!(
        image.pixels(),
        [
            21, 11, 1, 255, 22, 12, 2, 255, 23, 13, 3, 255, 24, 14, 4, 255
        ]
    );
}

#[test]
fn partial_edge_tiles_and_alpha_channel_are_not_flipped_or_rounded() {
    let tiles = [true, false, false, true];
    let raw = fixture(
        3,
        3,
        2,
        vec![
            plane(&tiles, &[1, 2], &[3, 4, 5, 6]),
            plane(&tiles, &[11, 12], &[13, 14, 15, 16]),
            plane(&tiles, &[21, 22], &[23, 24, 25, 26]),
            plane(&tiles, &[64, 128], &[192, 255, 1, 0]),
        ],
    );
    let image = pb3::decode(&raw).unwrap();
    let expected_b = [1, 1, 3, 1, 1, 4, 5, 6, 2];
    let expected_a = [64, 64, 192, 64, 64, 255, 1, 0, 128];
    for (i, p) in image.pixels().chunks_exact(4).enumerate() {
        assert_eq!(
            p,
            [
                expected_b[i] + 20,
                expected_b[i] + 10,
                expected_b[i],
                expected_a[i]
            ]
        );
    }
}

#[test]
fn all_solid_planes_need_no_lzss_tokens() {
    let raw = fixture(
        1,
        1,
        16,
        vec![
            plane(&[true], &[7], &[]),
            plane(&[true], &[8], &[]),
            plane(&[true], &[9], &[]),
        ],
    );
    assert_eq!(pb3::decode(&raw).unwrap().pixels(), [9, 8, 7, 255]);
}

#[test]
fn lzss_msb_flags_maximum_reference_overlap_and_ring_wrap() {
    let mut p = plane(&[false; 70], &[], &[7; 70]);
    p.lz_flags = vec![0x60];
    p.data = vec![7];
    let pair = (2014u16 << 5) | 31;
    p.data.extend(pair.to_le_bytes());
    p.data.extend(pair.to_le_bytes());
    p.data.push(7);
    let raw = fixture(
        70,
        1,
        1,
        vec![
            p,
            plane(&[false; 70], &[], &[8; 70]),
            plane(&[false; 70], &[], &[9; 70]),
        ],
    );
    let image = pb3::decode(&raw).unwrap();
    assert!(image.pixels().chunks_exact(4).all(|p| p == [9, 8, 7, 255]));
}

#[test]
fn truncation_bad_dimensions_offsets_and_unsupported_codecs_fail() {
    let raw = simple();
    for n in 0..raw.len() {
        assert!(pb3::decode(&raw[..n]).is_err(), "prefix={n}");
    }
    for (at, v) in [
        (24, 0),
        (30, u32::MAX),
        (44, 0),
        (44, u32::MAX),
        (48, u32::MAX),
    ] {
        assert!(pb3::decode(&mutate(raw.clone(), at, v)).is_err());
    }
    // Type 3 is decoded as JBP1, which this type-1 payload is not.
    let type3 = mutate(raw.clone(), 28, 3 | (2 << 16));
    assert_eq!(pb3::Header::parse(&type3).unwrap().kind, 3);
    assert!(pb3::decode(&type3).unwrap_err().0.contains("JBP1"));
    let type4 = mutate(raw.clone(), 28, 4 | (2 << 16));
    assert!(
        pb3::decode(&type4)
            .unwrap_err()
            .0
            .contains("not implemented")
    );
    assert!(pb3::decode(&mutate(raw, 34, 8)).is_err());
}

#[test]
fn exhausted_solid_literal_and_flag_streams_are_rejected() {
    for p in [
        plane(&[true], &[], &[]),
        plane(&[false], &[], &[1]),
        plane(&[], &[], &[1, 2, 3, 4]),
    ] {
        let raw = fixture(
            2,
            2,
            2,
            vec![p, plane(&[true], &[0], &[]), plane(&[true], &[0], &[])],
        );
        assert!(pb3::decode(&raw).is_err());
    }
    let mut p = plane(&[false], &[], &[1, 2, 3, 4]);
    p.lz_flags.clear();
    let raw = fixture(
        2,
        2,
        2,
        vec![p, plane(&[true], &[0], &[]), plane(&[true], &[0], &[])],
    );
    assert!(pb3::decode(&raw).is_err());
}

#[test]
fn lzss_overshoot_truncated_reference_and_unused_stream_bytes_fail() {
    let cases = [
        (vec![128], vec![0, 31]),
        (vec![128], vec![0]),
        (vec![0], vec![1, 2]),
        (vec![0, 0], vec![1]),
    ];
    for (flags, data) in cases {
        let mut p = plane(&[false], &[], &[1]);
        p.lz_flags = flags;
        p.data = data;
        let raw = fixture(
            1,
            1,
            1,
            vec![p, plane(&[true], &[0], &[]), plane(&[true], &[0], &[])],
        );
        assert!(pb3::decode(&raw).is_err());
    }
}

#[test]
fn allocation_limits_and_unused_tile_values_are_checked() {
    let raw = mutate(simple(), 30, u32::MAX);
    assert!(pb3::Header::parse(&raw).is_err());
    for p in [plane(&[true], &[1, 2], &[]), plane(&[true], &[1], &[1])] {
        let raw = fixture(
            1,
            1,
            1,
            vec![p, plane(&[true], &[0], &[]), plane(&[true], &[0], &[])],
        );
        assert!(pb3::decode(&raw).is_err());
    }
}

#[test]
fn aspect_fit_centers_letterbox_and_nearest_pixels() {
    let image = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    let mut target = vec![0x101010; 16];
    image.draw_fit(&mut target, 4, 4).unwrap();
    assert_eq!(&target[..4], &[0x101010; 4]);
    assert_eq!(
        &target[4..12],
        &[
            0xff0000, 0xff0000, 0x0000ff, 0x0000ff, 0xff0000, 0xff0000, 0x0000ff, 0x0000ff
        ]
    );
    assert_eq!(&target[12..], &[0x101010; 4]);
}

#[test]
fn preview_alpha_is_straight_and_invalid_surface_is_not_modified() {
    let image = RgbaImage::new(1, 1, vec![200, 50, 0, 128]).unwrap();
    let mut target = [0x6464c8];
    image.draw_fit(&mut target, 1, 1).unwrap();
    assert_eq!(target, [0x964b64]);
    assert!(image.draw_fit(&mut target, 0, 1).is_err());
    assert_eq!(target, [0x964b64]);
    assert!(RgbaImage::new(0, 1, vec![]).is_err());
    assert!(RgbaImage::new(u32::MAX, u32::MAX, vec![]).is_err());
}

#[test]
fn cli_export_is_create_new_and_unsupported_codec_creates_no_file() {
    let root = std::env::temp_dir().join(format!("cmvs-pb3-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("synthetic.pb3");
    let output = root.join("pixels.bin");
    let raw = simple();
    fs::write(&path, &raw).unwrap();
    let exe = env!("CARGO_BIN_EXE_cmvs-pb3");
    let run = |p: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--rgba")
            .arg(p)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(
        fs::read(&output).unwrap(),
        pb3::decode(&raw).unwrap().pixels()
    );
    assert!(!run(&output).status.success());
    fs::remove_file(&output).unwrap();
    fs::write(&path, mutate(raw, 28, 3 | (2 << 16))).unwrap();
    assert!(!run(&output).status.success());
    assert!(!output.exists());
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

use cmvs_rs::raster::{RgbaImage, adler32, crc32};

#[test]
fn checksums_match_published_check_values() {
    // CRC-32/ISO-HDLC and Adler-32 reference vectors.
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    assert_eq!(crc32(b""), 0);
    assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    assert_eq!(adler32(b""), 1);
}

/// Splits a PNG into (type, data) chunks, checking lengths and CRCs on the way.
fn chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut out = vec![];
    let mut at = 8;
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
        let data = png[at + 8..at + 8 + len].to_vec();
        let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
        assert_eq!(crc, crc32(&png[at + 4..at + 8 + len]));
        out.push((kind, data));
        at += 12 + len;
    }
    assert_eq!(at, png.len());
    out
}

/// Reassembles stored deflate blocks; returns the raw bytes and each block's BFINAL bit.
fn inflate_stored(zlib: &[u8]) -> (Vec<u8>, Vec<bool>) {
    assert_eq!(&zlib[..2], &[0x78, 0x01]);
    assert_eq!((u16::from(zlib[0]) << 8 | u16::from(zlib[1])) % 31, 0);
    let (mut raw, mut finals, mut at) = (vec![], vec![], 2);
    loop {
        let header = zlib[at];
        assert_eq!(header & 0b110, 0, "stored block type");
        let len = u16::from_le_bytes([zlib[at + 1], zlib[at + 2]]);
        let nlen = u16::from_le_bytes([zlib[at + 3], zlib[at + 4]]);
        assert_eq!(nlen, !len);
        raw.extend_from_slice(&zlib[at + 5..at + 5 + usize::from(len)]);
        finals.push(header & 1 == 1);
        at += 5 + usize::from(len);
        if header & 1 == 1 {
            break;
        }
    }
    let adler = u32::from_be_bytes(zlib[at..at + 4].try_into().unwrap());
    assert_eq!(adler, adler32(&raw));
    assert_eq!(at + 4, zlib.len());
    (raw, finals)
}

#[test]
fn png_round_trips_rgba_rows_with_filter_zero() {
    let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let image = RgbaImage::new(1, 3, pixels.clone()).unwrap();
    let parts = chunks(&image.to_png());
    let kinds: Vec<_> = parts.iter().map(|(kind, _)| kind).collect();
    assert_eq!(kinds, [b"IHDR", b"IDAT", b"IEND"]);
    // Width 1, height 3, 8-bit RGBA, deflate, adaptive filtering, no interlace.
    assert_eq!(parts[0].1, [0, 0, 0, 1, 0, 0, 0, 3, 8, 6, 0, 0, 0]);
    assert!(parts[2].1.is_empty());
    let (raw, finals) = inflate_stored(&parts[1].1);
    assert_eq!(finals, [true]);
    let expected: Vec<u8> = pixels
        .chunks(4)
        .flat_map(|row| [&[0][..], row].concat())
        .collect();
    assert_eq!(raw, expected);
}

#[test]
fn large_frames_span_several_stored_blocks() {
    // 128 rows of 1 + 512 bytes (65664 in total) exceed one 65535-byte stored block.
    let image = RgbaImage::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let parts = chunks(&image.to_png());
    let (raw, finals) = inflate_stored(&parts[1].1);
    assert_eq!(raw.len(), 128 * (1 + 128 * 4));
    assert_eq!(finals, [false, true]);
    assert!(
        raw.chunks(1 + 128 * 4)
            .all(|row| row[0] == 0 && row[1..].iter().all(|&b| b == 7))
    );
}

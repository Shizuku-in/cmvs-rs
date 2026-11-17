#![no_main]

use cmvs_core::{
    MAX_PAYLOAD,
    format::{cursor, jbp1, jbpd, lzss, msk, pb2, pb3},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pb2::Header::parse(data);
    let _ = pb2::part_names(data);
    if let Ok(img) = pb2::decode(data) {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }

    let _ = pb3::Header::parse(data);
    let _ = pb3::reference_name(data);
    if let Ok(img) = pb3::decode(data) {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }

    if let Ok(jbp) = jbp1::decode(data) {
        assert!(jbp.rgba.len() <= MAX_PAYLOAD);
    }
    if let Ok(img) = jbpd::decode(data) {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }
    if let Ok(img) = jbpd::decode_alpha(data) {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }

    for expected in [0, data.len(), 1024, MAX_PAYLOAD + 1] {
        if let Ok(out) = lzss::decode(data, expected) {
            assert!(out.len() <= MAX_PAYLOAD);
        }
    }

    let _ = msk::Header::parse(data);
    if let Ok(img) = msk::decode(data) {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }

    if let Ok(cur) = cursor::CursorBitmap::decode(data)
        && let Ok(rgba) = cur.rgba()
    {
        assert!(rgba.len() <= MAX_PAYLOAD);
    }
});

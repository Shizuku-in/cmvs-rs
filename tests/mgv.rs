use cmvs_rs::mgv::MgvIndex;
use std::io::Cursor;
fn synthetic() -> Vec<u8> {
    let mut b = b"MGV1".to_vec();
    for word in [32u32, 4, 3, 4] {
        b.extend(word.to_le_bytes());
    }
    b.extend(32u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(1000u32.to_le_bytes());
    b.extend(24000u32.to_le_bytes());
    for word in [1u32, 1 << 16, 1 << 16, (2 << 16) | 1] {
        b.extend(word.to_le_bytes());
    }
    b.extend(b"oggdata");
    b
}
#[test]
fn container_preserves_nominal_frame_mapping_and_stream_ranges() {
    let b = synthetic();
    let i = MgvIndex::read(&mut Cursor::new(&b)).unwrap();
    assert_eq!(
        (i.width, i.height, i.rate, i.rate_time),
        (32, 16, 24000, 1000)
    );
    assert_eq!(
        (i.audio_offset, i.audio_size, i.video_offset, i.video_size),
        (48, 3, 51, 4)
    );
    assert_eq!(
        i.frames.iter().map(|f| f.packet).collect::<Vec<_>>(),
        [0, 1, 1, 2]
    );
    assert_eq!(i.packet_count(), 3);
}
#[test]
fn malformed_ranges_geometry_rates_and_frame_maps_are_rejected() {
    let b = synthetic();
    for length in 0..b.len() {
        assert!(MgvIndex::read(&mut Cursor::new(&b[..length])).is_err());
    }
    for (at, value) in [
        (4, 31),
        (8, 5),
        (12, u32::MAX),
        (16, 0),
        (16, u32::MAX),
        (20, 0),
        (20, u32::MAX),
        (24, 0),
        (28, 0),
        (32, 0),
        (36, 2),
        (36, 3 << 16),
        (40, 0),
        (44, 4 << 16),
    ] {
        let mut bad = b.clone();
        bad[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            MgvIndex::read(&mut Cursor::new(bad)).is_err(),
            "offset {at} value {value}"
        );
    }
}

use cmvs_rs::cmv9::Cmv9Index;
#[path = "support/cmv9.rs"]
mod fixture;

#[test]
fn static_and_dynamic_blocks_compose_and_crop_the_padded_edge() {
    for reuse in [false, true] {
        let bytes = fixture::movie(reuse);
        let index = Cmv9Index::parse(&bytes).unwrap();
        let image = index.decode_frame(&bytes, 0).unwrap();
        assert_eq!((image.width(), image.height()), (17, 16));
        for row in image.pixels().as_chunks::<{ 17 * 4 }>().0 {
            assert!(
                row[..16 * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| *p == [136, 136, 136, 255])
            );
            assert_eq!(&row[16 * 4..], &[144, 144, 144, 255]);
        }
        assert!(index.decode_frame(&bytes, 1).is_err());
        assert!(index.decode_frame(&bytes, u32::MAX).is_err());
    }
}
#[test]
fn malformed_container_and_mask_fields_are_rejected() {
    let good = fixture::movie(false);
    for at in [4, 8, 16, 28, 32, 36, 40, 48, 60, 84] {
        let mut bad = good.clone();
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            Cmv9Index::parse(&bad)
                .and_then(|i| i.decode_frame(&bad, 0))
                .is_err(),
            "offset {at}"
        );
    }
    for at in [20, 24] {
        let mut bad = good.clone();
        bad[at..at + 4].fill(0);
        assert!(Cmv9Index::parse(&bad).is_err());
    }
    // One bitmap bit changes which blocks have encoded coefficients.
    let mut bad = good.clone();
    bad[92] = 0;
    assert!(
        Cmv9Index::parse(&bad)
            .unwrap()
            .decode_frame(&bad, 0)
            .is_err()
    );
    for n in 0..good.len() {
        assert!(Cmv9Index::parse(&good[..n]).is_err(), "prefix {n}");
    }
}

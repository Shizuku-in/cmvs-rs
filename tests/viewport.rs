use cmvs_rs::{raster::RgbaImage, viewport::Viewport};

#[test]
fn all_four_modes_share_rendered_pixel_and_pointer_mapping() {
    let mut pixels = vec![];
    for n in 1..=8 {
        pixels.extend([n, 0, 0, 255]);
    }
    let image = RgbaImage::new(4, 2, pixels).unwrap();
    for (mode, rect) in [
        (0, [1, 2, 4, 2]),
        (1, [0, 1, 6, 3]),
        (2, [-3, 0, 12, 6]),
        (3, [0, 0, 6, 6]),
    ] {
        let view = Viewport::new([4, 2], [6, 6], mode).unwrap();
        assert_eq!(
            [
                view.left,
                view.top,
                i64::from(view.width),
                i64::from(view.height)
            ],
            rect
        );
        let mut target = vec![0xabcdef; 36];
        image.draw_aspect(&mut target, 6, 6, mode).unwrap();
        for y in 0..6 {
            for x in 0..6 {
                let expected = view
                    .to_source([f64::from(x), f64::from(y)])
                    .map_or(0xabcdef, |[sx, sy]| (sy as u32 * 4 + sx as u32 + 1) << 16);
                assert_eq!(
                    target[y as usize * 6 + x as usize],
                    expected,
                    "mode={mode} x={x} y={y}"
                );
            }
        }
        assert!(view.to_source([-1.0, 0.0]).is_none());
        assert!(view.to_source([6.0, 0.0]).is_none());
        assert!(view.to_source([f64::NAN, 0.0]).is_none());
    }
}

#[test]
fn normal_falls_back_to_contain_and_fixed_point_rounding_matches_engine() {
    assert_eq!(
        Viewport::new([1280, 720], [1000, 1000], 0),
        Viewport::new([1280, 720], [1000, 1000], 1)
    );
    let view = Viewport::new([1280, 720], [1001, 1000], 1).unwrap();
    assert_eq!(view.height, (((1001u32 << 13) / 1280) * 720) >> 13);
    for mode in 0..=3 {
        let view = Viewport::new([1280, 720], [1920, 1080], mode).unwrap();
        for point in [[640, 360], [500, 300], [800, 400]] {
            assert_eq!(view.to_source(view.to_target(point)), Some(point));
        }
    }
    assert!(Viewport::new([0, 2], [5, 5], 1).is_err());
    assert!(Viewport::new([2, 2], [5, 5], 4).is_err());
    assert!(Viewport::new([2, 2], [u32::MAX, u32::MAX], 1).is_err());
}

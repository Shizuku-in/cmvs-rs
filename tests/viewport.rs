use cmvs_rs::{raster::RgbaImage, viewport::Viewport};

#[test]
fn scaling_disable_centers_native_pixels_and_input_without_losing_selected_mode() {
    use cmvs_rs::viewport::presentation_mode;
    let source = [4, 2];
    let target = [12, 8];
    let scaled = Viewport::new(source, target, presentation_mode(true, true, 3)).unwrap();
    assert_eq!(
        (scaled.left, scaled.top, scaled.width, scaled.height),
        (0, 0, 12, 8)
    );
    let native = Viewport::new(source, target, presentation_mode(true, false, 3)).unwrap();
    assert_eq!(
        (native.left, native.top, native.width, native.height),
        (4, 3, 4, 2)
    );
    assert_eq!(native.to_source([3.0, 3.0]), None);
    assert_eq!(native.to_source([4.0, 3.0]), Some([0, 0]));
    assert_eq!(native.to_source(native.to_target([3, 1])), Some([3, 1]));
    let image = RgbaImage::new(4, 2, [40, 20, 10, 255].repeat(8)).unwrap();
    let mut pixels = vec![0; 12 * 8];
    image
        .draw_scaled(&mut pixels, 12, 8, presentation_mode(true, false, 3), false)
        .unwrap();
    assert_eq!(pixels.iter().filter(|p| **p == 0x28140a).count(), 8);
    assert_eq!(pixels[3 * 12 + 4], 0x28140a);
    assert_eq!(presentation_mode(false, false, 3), 1);
    assert_eq!(presentation_mode(false, true, 0), 1);
}

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

#[test]
fn linear_presentation_uses_texel_centers_clamped_edges_and_premultiplied_alpha() {
    let image = RgbaImage::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
    let mut target = vec![0x123456; 4];
    image.draw_scaled(&mut target, 4, 1, 3, true).unwrap();
    assert_eq!(target, [0, 0x404040, 0xbfbfbf, 0xffffff]);
    let mut nearest = vec![0x123456; 4];
    image.draw_scaled(&mut nearest, 4, 1, 3, false).unwrap();
    assert_eq!(nearest, [0, 0, 0xffffff, 0xffffff]);

    // An unscaled view samples exact texels, matching the point path.
    let mut pixels = vec![];
    for n in 0..12u8 {
        pixels.extend([n * 20, 255 - n, n, if n % 3 == 0 { 128 } else { 255 }]);
    }
    let image = RgbaImage::new(4, 3, pixels).unwrap();
    let mut linear = vec![0x203040; 8 * 5];
    let mut point = linear.clone();
    image.draw_scaled(&mut linear, 8, 5, 0, true).unwrap();
    image.draw_scaled(&mut point, 8, 5, 0, false).unwrap();
    assert_eq!(linear, point);

    // Transparent texels contribute no color; half coverage keeps half the backdrop.
    let image = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
    let mut target = vec![0x0000ff; 3];
    image.draw_scaled(&mut target, 3, 1, 3, true).unwrap();
    assert_eq!(target[1], 0x800080);
    assert!(image.draw_scaled(&mut [0; 2], 3, 1, 3, true).is_err());
}

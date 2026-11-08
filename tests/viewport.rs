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
    assert!(Viewport::new([2, 2], [5, 5], 5).is_err());
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

#[test]
fn legacy_full_move_keeps_unscaled_drawing_and_pointer_input_at_the_origin() {
    use cmvs_rs::viewport::{UNSCALED_ORIGIN, presentation_mode_with_full_move};
    let mode = presentation_mode_with_full_move(true, false, 3, true);
    assert_eq!(mode, UNSCALED_ORIGIN);
    let view = Viewport::new([4, 2], [8, 6], mode).unwrap();
    assert_eq!((view.left, view.top, view.width, view.height), (0, 0, 4, 2));
    assert_eq!(view.to_source([0.0, 0.0]), Some([0, 0]));
    assert_eq!(view.to_source([4.0, 0.0]), None);
    assert_eq!(view.to_source(view.to_target([3, 1])), Some([3, 1]));
    let image = RgbaImage::new(4, 2, vec![255; 4 * 2 * 4]).unwrap();
    let mut pixels = vec![0; 8 * 6];
    image.draw_scaled(&mut pixels, 8, 6, mode, false).unwrap();
    for y in 0..6 {
        for x in 0..8 {
            assert_eq!(pixels[y * 8 + x], if x < 4 && y < 2 { 0xffffff } else { 0 });
        }
    }
    assert_eq!(presentation_mode_with_full_move(true, false, 3, false), 0);
    assert_eq!(presentation_mode_with_full_move(true, true, 3, true), 3);
    assert_eq!(presentation_mode_with_full_move(false, false, 3, true), 1);
    assert_eq!(
        Viewport::new([8, 6], [4, 4], mode),
        Viewport::new([8, 6], [4, 4], 1)
    );
}

/// Scalar presentation oracle: source texels are weighted in premultiplied space,
/// then composited over the existing target with one final rounding operation.
fn reference_present(
    image: &RgbaImage,
    target: &mut [u32],
    size: [u32; 2],
    mode: u32,
    linear: bool,
) {
    let view = Viewport::new([image.width(), image.height()], size, mode).unwrap();
    let tap = |position: i64, origin: i64, span: u32, source: u32| {
        let coordinate =
            ((position - origin) * 2 + 1) * i64::from(source) * 256 / (i64::from(span) * 2) - 128;
        let coordinate = coordinate.clamp(0, (i64::from(source) - 1) * 256);
        let low = coordinate / 256;
        [
            (low, 256 - coordinate % 256),
            ((low + 1).min(i64::from(source) - 1), coordinate % 256),
        ]
    };
    for y in 0..i64::from(size[1]) {
        for x in 0..i64::from(size[0]) {
            if x < view.left
                || y < view.top
                || x >= view.left + i64::from(view.width)
                || y >= view.top + i64::from(view.height)
            {
                continue;
            }
            let sample = |position, origin, span, source| {
                if linear {
                    tap(position, origin, span, source)
                } else {
                    let at = (position - origin) * i64::from(source) / i64::from(span);
                    [(at, 256), (at, 0)]
                }
            };
            let mut sums = [0u64; 4];
            for (sy, wy) in sample(y, view.top, view.height, image.height()) {
                for (sx, wx) in sample(x, view.left, view.width, image.width()) {
                    let at = (sy as usize * image.width() as usize + sx as usize) * 4;
                    let pixel = &image.pixels()[at..at + 4];
                    let weight = (wx * wy) as u64;
                    let alpha = u64::from(pixel[3]);
                    for channel in 0..3 {
                        sums[channel] += weight * alpha * u64::from(pixel[channel]);
                    }
                    sums[3] += weight * alpha;
                }
            }
            let target = &mut target[y as usize * size[0] as usize + x as usize];
            let scale = 255 * 65536;
            let mut rgb = 0;
            for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                let backdrop = u64::from((*target >> shift) & 255);
                rgb |= (((sums[channel] + backdrop * (scale - sums[3]) + scale / 2) / scale)
                    as u32)
                    << shift;
            }
            *target = rgb;
        }
    }
}

#[test]
fn presentation_matches_scalar_alpha_and_rounding_across_sizes_and_modes() {
    for [width, height] in [[1, 1], [1, 7], [7, 1], [3, 5], [8, 6]] {
        for alpha_mode in 0..3 {
            let mut pixels = Vec::new();
            for i in 0..width * height {
                let alpha = match alpha_mode {
                    0 => 255,
                    1 => [0, 1, 127, 128, 254, 255][i as usize % 6],
                    _ if i + 1 == width * height => 254,
                    _ => 255,
                };
                pixels.extend([
                    (i * 73) as u8,
                    (i * 37 + 19) as u8,
                    (i * 17 + 231) as u8,
                    alpha,
                ]);
            }
            let image = RgbaImage::new(width, height, pixels).unwrap();
            for size in [
                [width, height],
                [width, height.saturating_sub(2).max(1)],
                [width.saturating_sub(2).max(1), height],
                [11, 9],
                [2, 3],
                [17, 11],
            ] {
                for mode in 0..=4 {
                    for linear in [false, true] {
                        // Include nonzero high bytes: untouched borders retain them,
                        // while covered pixels are the compositor's 0x00RRGGBB.
                        let mut expected: Vec<_> = (0..size[0] * size[1])
                            .map(|i| 0xa500_0000 | (i * 193 + 0x123456) & 0xffffff)
                            .collect();
                        let mut actual = expected.clone();
                        if let Err(error) = Viewport::new([width, height], size, mode) {
                            assert_eq!(
                                image.draw_scaled(&mut actual, size[0], size[1], mode, linear),
                                Err(error)
                            );
                            assert_eq!(actual, expected);
                            continue;
                        }
                        reference_present(&image, &mut expected, size, mode, linear);
                        image
                            .draw_scaled(&mut actual, size[0], size[1], mode, linear)
                            .unwrap();
                        assert_eq!(
                            actual, expected,
                            "source={width}x{height}, alpha={alpha_mode}, target={size:?}, mode={mode}, linear={linear}"
                        );
                    }
                }
            }
        }
    }
}

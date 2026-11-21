use super::RgbaImage;
use crate::{FormatError, viewport::Viewport};

/// Sampling filter for the completed logical frame. Values match the VM's draw modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ScaleFilter {
    Nearest = 0,
    Bilinear = 1,
    /// Keys cubic convolution with the original mode 2's parameter a = -1.
    Keys = 2,
    /// Windowed sinc with support radius four texels.
    Lanczos4 = 3,
}

impl From<bool> for ScaleFilter {
    fn from(linear: bool) -> Self {
        if linear {
            Self::Bilinear
        } else {
            Self::Nearest
        }
    }
}

impl TryFrom<u32> for ScaleFilter {
    type Error = FormatError;

    fn try_from(mode: u32) -> Result<Self, Self::Error> {
        match mode {
            0 => Ok(Self::Nearest),
            1 => Ok(Self::Bilinear),
            2 => Ok(Self::Keys),
            3 => Ok(Self::Lanczos4),
            _ => Err(FormatError("invalid presentation filter")),
        }
    }
}

pub(super) fn keys(distance: f32) -> f32 {
    // The original mode 2 uses Keys cubic convolution with a = -1.
    let x = distance.abs();
    if x < 1.0 {
        1.0 + x * x * (x - 2.0)
    } else {
        // -(x - 1) * (x - 2)^2 on the outer lobe.
        (1.0 - x) * (x - 2.0) * (x - 2.0)
    }
}

pub(super) fn lanczos4(distance: f32) -> f32 {
    if distance == 0.0 {
        return 1.0;
    }
    let angle = std::f32::consts::PI * distance;
    let window = angle * 0.25;
    (angle.sin() / angle) * (window.sin() / window)
}

struct Axis {
    origin: i64,
    span: i64,
    size: i64,
}

impl Axis {
    fn taps<const N: usize>(&self, pixel: i64, weight: fn(f32) -> f32) -> [(usize, f32); N] {
        // Convert the destination pixel center to source texel-center coordinates.
        let numerator = (2 * (pixel - self.origin) + 1) * self.size - self.span;
        let denominator = 2 * self.span;
        let center = numerator.div_euclid(denominator);
        let remainder = numerator.rem_euclid(denominator);
        if remainder == 0 {
            let mut taps = [(center.clamp(0, self.size - 1) as usize, 0.0); N];
            taps[0].1 = 1.0;
            return taps;
        }
        let fraction = (remainder as f64 / denominator as f64) as f32;
        let mut taps = std::array::from_fn(|i| {
            let offset = i as i64 + 1 - (N / 2) as i64;
            let index = (center + offset).clamp(0, self.size - 1) as usize;
            (index, weight(fraction - offset as f32))
        });
        let sum = taps.iter().map(|(_, weight)| weight).sum::<f32>();
        for (_, weight) in &mut taps {
            *weight /= sum;
        }
        taps
    }
}

pub(super) fn draw<const N: usize>(
    image: &RgbaImage,
    target: &mut [u32],
    width: usize,
    view: Viewport,
    weight: fn(f32) -> f32,
) {
    let [xs, ys] = view.visible_ranges();
    let horizontal = Axis {
        origin: view.left,
        span: i64::from(view.width),
        size: i64::from(image.width()),
    };
    let vertical = Axis {
        origin: view.top,
        span: i64::from(view.height),
        size: i64::from(image.height()),
    };
    let stride = image.width() as usize;
    let (pixels, _) = image.pixels().as_chunks::<4>();
    let opaque = image.is_opaque();
    // Filter in strips to bound scratch space even for very wide surfaces. Each
    // strip keeps only the horizontal rows the vertical filter can still use.
    const STRIP_WIDTH: usize = 4096;
    for start in (xs.start..xs.end).step_by(STRIP_WIDTH) {
        let columns: Vec<_> = (start..(start + STRIP_WIDTH as i64).min(xs.end))
            .map(|x| horizontal.taps::<N>(x, weight))
            .collect();
        let mut cache: [(Option<usize>, Vec<[f32; 4]>); N] =
            std::array::from_fn(|_| (None, vec![[0.0; 4]; columns.len()]));
        let mut sums = vec![[0.0; 4]; columns.len()];
        for y in ys.clone() {
            sums.fill([0.0; 4]);
            for (row, row_weight) in vertical.taps::<N>(y, weight) {
                if row_weight == 0.0 {
                    continue;
                }
                let (cached, filtered) = &mut cache[row % N];
                if *cached != Some(row) {
                    let source = &pixels[row * stride..][..stride];
                    for (out, taps) in filtered.iter_mut().zip(&columns) {
                        let mut sum = [0.0; 4];
                        for &(x, weight) in taps {
                            let [r, g, b, a] = source[x].map(f32::from);
                            let alpha = if opaque { 1.0 } else { a / 255.0 };
                            let sample = [r * alpha, g * alpha, b * alpha, a];
                            for c in 0..4 {
                                sum[c] += sample[c] * weight;
                            }
                        }
                        *out = sum;
                    }
                    *cached = Some(row);
                }
                for (sum, sample) in sums
                    .as_flattened_mut()
                    .iter_mut()
                    .zip(filtered.as_flattened())
                {
                    *sum += sample * row_weight;
                }
            }
            let at = y as usize * width + start as usize;
            let target = &mut target[at..][..columns.len()];
            for (pixel, sum) in target.iter_mut().zip(&sums) {
                let alpha = if opaque {
                    255.0
                } else {
                    sum[3].clamp(0.0, 255.0)
                };
                let channel = |c: usize, shift: u32| {
                    let under = if opaque {
                        0.0
                    } else {
                        ((*pixel >> shift) & 255) as f32
                    };
                    (sum[c].clamp(0.0, alpha) + under * (1.0 - alpha / 255.0) + 0.5) as u32
                };
                *pixel = channel(0, 16) << 16 | channel(1, 8) << 8 | channel(2, 0);
            }
        }
    }
}

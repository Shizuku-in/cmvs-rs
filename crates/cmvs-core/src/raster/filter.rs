use super::RgbaImage;
use crate::{FormatError, MAX_PAYLOAD, viewport::Viewport};

/// Internal GPU scene resolution scaling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderScale {
    /// Render at the game's logical canvas size (1x, original parity).
    #[default]
    Original,
    /// Match the presentation viewport up to 3x logical size (never below 1x).
    Auto,
    /// Fixed 1.5x logical resolution (e.g. 1280x720 -> 1920x1080).
    X1_5,
    /// Fixed 2x logical resolution (e.g. 1280x720 -> 2560x1440).
    X2,
    /// Fixed 3x logical resolution (e.g. 1280x720 -> 3840x2160).
    X3,
}

impl RenderScale {
    /// Maximum automatic/internal scale relative to logical dimensions.
    const MAX_FACTOR: f64 = 3.0;

    /// Computes the clamped internal render target size for a logical canvas and optional
    /// physical presentation viewport, bounded by `max_dim` and `MAX_PAYLOAD / 4` pixels.
    pub fn resolve(self, logical: [u32; 2], viewport: Option<[u32; 2]>, max_dim: u32) -> [u32; 2] {
        let [lw, lh] = logical;
        let max_pixels = (MAX_PAYLOAD / 4) as u64;
        if lw == 0
            || lh == 0
            || max_dim == 0
            || lw > max_dim
            || lh > max_dim
            || u64::from(lw) * u64::from(lh) > max_pixels
        {
            return logical;
        }
        let [lw64, lh64] = [u64::from(lw), u64::from(lh)];
        let requested = match self {
            Self::Original => return logical,
            Self::Auto => match viewport {
                Some([vw, vh]) if vw > lw || vh > lh => {
                    [u64::from(vw.max(lw)), u64::from(vh.max(lh))]
                }
                _ => return logical,
            },
            Self::X1_5 => [(lw64 * 3).div_ceil(2), (lh64 * 3).div_ceil(2)],
            Self::X2 => [lw64 * 2, lh64 * 2],
            Self::X3 => [lw64 * 3, lh64 * 3],
        };
        let [rw, rh] = requested.map(|v| v as f64);
        let factor_cap = Self::MAX_FACTOR / (rw / lw64 as f64).max(rh / lh64 as f64);
        let dim_cap = f64::from(max_dim) / rw.max(rh);
        let pixel_cap = (max_pixels as f64 / (rw * rh)).sqrt();
        let clamp = factor_cap.min(dim_cap).min(pixel_cap).min(1.0);
        let max_w = (lw64 * 3).min(u64::from(max_dim));
        let max_h = (lh64 * 3).min(u64::from(max_dim));
        let mut w = if clamp >= 1.0 {
            requested[0]
        } else {
            (rw * clamp).floor() as u64
        }
        .clamp(lw64, max_w);
        let mut h = if clamp >= 1.0 {
            requested[1]
        } else {
            (rh * clamp).floor() as u64
        }
        .clamp(lh64, max_h);
        // Raising one axis back to its logical floor (or float rounding) can push w * h
        // above max_pixels; enforce the budget against the final dimensions.
        if w * h > max_pixels {
            if w == lw64 {
                h = (max_pixels / w).clamp(lh64, max_h);
            } else if h == lh64 {
                w = (max_pixels / h).clamp(lw64, max_w);
            } else {
                h = (max_pixels / w).clamp(lh64, max_h);
                if w * h > max_pixels {
                    w = (max_pixels / h).clamp(lw64, max_w);
                }
            }
        }
        [w as u32, h as u32]
    }

    /// Vector glyph rasterization scale corresponding to an internal render target.
    pub fn font_scale(logical: [u32; 2], render: [u32; 2]) -> f32 {
        if logical.contains(&0) || render == logical {
            return 1.0;
        }
        ((render[0] as f32 / logical[0] as f32).max(render[1] as f32 / logical[1] as f32))
            .clamp(1.0, 4.0)
    }
}

/// Sampling algorithm for image resizing and presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleFilter {
    Nearest,
    Bilinear,
    /// Keys cubic convolution with the original mode 2's parameter a = -1.
    Keys,
    /// Mitchell–Netravali cubic with B = C = 1/3.
    Mitchell,
    /// Lanczos with radius three, optionally clamped to the local source range.
    Lanczos3 {
        anti_ringing: bool,
    },
    /// Windowed sinc with support radius four texels.
    Lanczos4,
}

impl ScaleFilter {
    /// The linear flag for texture samplers that support nearest and bilinear filtering.
    pub fn linear_sampler(self) -> Option<bool> {
        match self {
            Self::Nearest => Some(false),
            Self::Bilinear => Some(true),
            _ => None,
        }
    }
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

fn keys(distance: f32) -> f32 {
    // The original mode 2 uses Keys cubic convolution with a = -1.
    let x = distance.abs();
    if x < 1.0 {
        1.0 + x * x * (x - 2.0)
    } else {
        // -(x - 1) * (x - 2)^2 on the outer lobe.
        (1.0 - x) * (x - 2.0) * (x - 2.0)
    }
}

fn mitchell(distance: f32) -> f32 {
    let x = distance.abs();
    if x < 1.0 {
        (16.0 + x * x * (21.0 * x - 36.0)) / 18.0
    } else {
        (8.0 - 7.0 * x) * (2.0 - x) * (2.0 - x) / 18.0
    }
}

fn lanczos<const RADIUS: u32>(distance: f32) -> f32 {
    if distance == 0.0 {
        return 1.0;
    }
    let angle = std::f32::consts::PI * distance;
    let window = angle / RADIUS as f32;
    (angle.sin() / angle) * (window.sin() / window)
}

#[derive(Clone, Copy)]
struct Kernel {
    weight: fn(f32) -> f32,
    interpolating: bool,
    anti_ringing: bool,
}

impl Kernel {
    fn interpolating(weight: fn(f32) -> f32) -> Self {
        Self {
            weight,
            interpolating: true,
            anti_ringing: false,
        }
    }
}

struct Axis {
    origin: i64,
    span: i64,
    size: i64,
}

impl Axis {
    fn taps<const N: usize>(&self, pixel: i64, kernel: Kernel) -> [(usize, f32); N] {
        if N == 1 {
            // Match the point sampler's integer mapping.
            let index = ((pixel - self.origin) * self.size / self.span) as usize;
            return [(index, 1.0); N];
        }
        // Convert the destination pixel center to source texel-center coordinates.
        let numerator = (2 * (pixel - self.origin) + 1) * self.size - self.span;
        let denominator = 2 * self.span;
        let center = numerator.div_euclid(denominator);
        let remainder = numerator.rem_euclid(denominator);
        if self.size == self.span || (remainder == 0 && kernel.interpolating) {
            let mut taps = [(center.clamp(0, self.size - 1) as usize, 0.0); N];
            taps[0].1 = 1.0;
            return taps;
        }
        let fraction = (remainder as f64 / denominator as f64) as f32;
        let mut taps = std::array::from_fn(|i| {
            let offset = i as i64 + 1 - (N / 2) as i64;
            let index = (center + offset).clamp(0, self.size - 1) as usize;
            (index, (kernel.weight)(fraction - offset as f32))
        });
        let sum = taps.iter().map(|(_, weight)| weight).sum::<f32>();
        for (_, weight) in &mut taps {
            *weight /= sum;
        }
        taps
    }
}

pub(super) fn draw(
    image: &RgbaImage,
    target: &mut [u32],
    width: usize,
    view: Viewport,
    filter: ScaleFilter,
) {
    let opaque = image.is_opaque();
    resample_image(image, view, filter, |x, y, sums| {
        let at = y * width + x;
        for (pixel, sum) in target[at..].iter_mut().zip(sums) {
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
    });
}

impl RgbaImage {
    /// Resize to exact dimensions, filtering premultiplied colors and returning straight RGBA.
    pub fn resize(&self, size: [u32; 2], filter: ScaleFilter) -> Result<Self, FormatError> {
        let source = [self.width(), self.height()];
        if source == size {
            return Ok(self.clone());
        }
        let view = Viewport::new(source, size, 3)?;
        let [width, height] = size;
        let mut output = vec![0u8; width as usize * height as usize * 4];
        resample_image(self, view, filter, |x, y, sums| {
            let start = (y * width as usize + x) * 4;
            for (pixel, &[r, g, b, alpha]) in
                output[start..].as_chunks_mut::<4>().0.iter_mut().zip(sums)
            {
                let alpha = alpha.clamp(0.0, 255.0);
                let a = (alpha + 0.5) as u8;
                if a != 0 {
                    let [r, g, b] =
                        [r, g, b].map(|c| (c.clamp(0.0, alpha) * (255.0 / alpha) + 0.5) as u8);
                    *pixel = [r, g, b, a];
                }
            }
        });
        Self::new(width, height, output)
    }
}

fn resample_image(
    image: &RgbaImage,
    view: Viewport,
    filter: ScaleFilter,
    emit: impl FnMut(usize, usize, &[[f32; 4]]),
) {
    match filter {
        ScaleFilter::Nearest => resample::<1>(image, view, Kernel::interpolating(|_| 1.0), emit),
        ScaleFilter::Bilinear => {
            resample::<2>(image, view, Kernel::interpolating(|d| 1.0 - d.abs()), emit)
        }
        ScaleFilter::Keys => resample::<4>(image, view, Kernel::interpolating(keys), emit),
        ScaleFilter::Mitchell => resample::<4>(
            image,
            view,
            Kernel {
                weight: mitchell,
                interpolating: false,
                anti_ringing: false,
            },
            emit,
        ),
        ScaleFilter::Lanczos3 { anti_ringing } => resample::<6>(
            image,
            view,
            Kernel {
                anti_ringing,
                ..Kernel::interpolating(lanczos::<3>)
            },
            emit,
        ),
        ScaleFilter::Lanczos4 => {
            resample::<8>(image, view, Kernel::interpolating(lanczos::<4>), emit)
        }
    }
}

fn resample<const N: usize>(
    image: &RgbaImage,
    view: Viewport,
    kernel: Kernel,
    mut emit: impl FnMut(usize, usize, &[[f32; 4]]),
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
    let sample = |pixel: [u8; 4]| {
        let [r, g, b, a] = pixel.map(f32::from);
        let alpha = if opaque { 1.0 } else { a / 255.0 };
        [r * alpha, g * alpha, b * alpha, a]
    };
    // Filter in strips to bound scratch space even for very wide surfaces. Each
    // strip keeps only the horizontal rows the vertical filter can still use.
    const STRIP_WIDTH: usize = 4096;
    for start in (xs.start..xs.end).step_by(STRIP_WIDTH) {
        let columns: Vec<_> = (start..(start + STRIP_WIDTH as i64).min(xs.end))
            .map(|x| horizontal.taps::<N>(x, kernel))
            .collect();
        let mut cache: [(Option<usize>, Vec<[f32; 4]>); N] =
            std::array::from_fn(|_| (None, vec![[0.0; 4]; columns.len()]));
        let mut sums = vec![[0.0; 4]; columns.len()];
        for y in ys.clone() {
            sums.fill([0.0; 4]);
            let rows = vertical.taps::<N>(y, kernel);
            for (row, row_weight) in rows {
                if row_weight == 0.0 {
                    continue;
                }
                let (cached, filtered) = &mut cache[row % N];
                if *cached != Some(row) {
                    let source = &pixels[row * stride..][..stride];
                    for (out, taps) in filtered.iter_mut().zip(&columns) {
                        let mut sum = [0.0; 4];
                        for &(x, weight) in taps {
                            let sample = sample(source[x]);
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
            if kernel.anti_ringing {
                // Clamp once after both passes, using the adjacent 2x2 source texels.
                let [low, high] = [(N - 1) / 2, N / 2];
                let top = rows[low].0 * stride;
                let bottom = rows[high].0 * stride;
                for (sum, taps) in sums.iter_mut().zip(&columns) {
                    let left = taps[low].0;
                    let right = taps[high].0;
                    let [a, b, c, d] = [top + left, top + right, bottom + left, bottom + right]
                        .map(|at| sample(pixels[at]));
                    for ch in 0..4 {
                        sum[ch] = sum[ch].clamp(
                            a[ch].min(b[ch]).min(c[ch]).min(d[ch]),
                            a[ch].max(b[ch]).max(c[ch]).max(d[ch]),
                        );
                    }
                }
            }
            emit(start as usize, y as usize, &sums);
        }
    }
}

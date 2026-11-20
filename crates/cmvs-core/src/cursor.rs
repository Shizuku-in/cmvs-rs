//! Single-image, 4-bit, 8-bit, 24-bit and 32-bit BI_RGB CUR files. No OS cursor handles.
//! Preserve AND/XOR semantics; not every legacy cursor can be represented by RGBA.
use crate::FormatError;

/// 0x140019430 / 0x140056530: inactivity uses logical pointer changes and a u32
/// millisecond clock. Mode zero suspends automatic changes, retaining visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorVisibility {
    position: [i32; 2],
    moved_at: u32,
    visible: bool,
}
impl Default for CursorVisibility {
    fn default() -> Self {
        Self {
            position: [0, 0],
            moved_at: 0,
            visible: true,
        }
    }
}
impl CursorVisibility {
    pub fn update(&mut self, now: u32, mode: u32, active: bool, position: [i32; 2]) -> bool {
        if active {
            if position != self.position {
                self.position = position;
                self.moved_at = now;
            }
            if mode != 0 {
                // Preserve the original raw unsigned deadline comparison, including
                // its behavior around clock wrap; do not substitute elapsed time.
                self.visible = now < self.moved_at.wrapping_add(mode.wrapping_mul(5000));
            }
        }
        self.visible
    }
}

fn word(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn dword(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("checked range"))
}

/// Single BI_RGB DIB with original BGRA palette and bottom-up, DWORD-padded planes.
pub struct CursorDib<'a> {
    pub width: usize,
    pub height: usize,
    pub hotspot: (u16, u16),
    pub bits: u16,
    pub palette: &'a [u8],
    pub xor: &'a [u8],
    pub and: &'a [u8],
}
fn stride(width: usize, bits: u16) -> usize {
    (width * usize::from(bits)).div_ceil(32) * 4
}
impl<'a> CursorDib<'a> {
    pub fn parse(raw: &'a [u8]) -> Result<Self, FormatError> {
        if raw.len() < 22 || word(raw, 0) != 0 || word(raw, 2) != 2 || word(raw, 4) != 1 {
            return Err(FormatError("expected single-image CUR directory"));
        }
        let width = if raw[6] == 0 {
            256
        } else {
            usize::from(raw[6])
        };
        let height = if raw[7] == 0 {
            256
        } else {
            usize::from(raw[7])
        };
        let hotspot = (word(raw, 10), word(raw, 12));
        if raw[9] != 0 || usize::from(hotspot.0) >= width || usize::from(hotspot.1) >= height {
            return Err(FormatError("invalid CUR reserved field or hotspot"));
        }
        let start = u64::from(dword(raw, 18));
        let size = u64::from(dword(raw, 14));
        if start < 22 || size < 40 || start + size != raw.len() as u64 {
            return Err(FormatError("CUR image range does not match file"));
        }
        let b = &raw[start as usize..];
        if dword(b, 0) != 40
            || dword(b, 4) != width as u32
            || dword(b, 8) != 2 * height as u32
            || word(b, 12) != 1
            || !matches!(word(b, 14), 4 | 8 | 24 | 32)
            || dword(b, 16) != 0
        {
            return Err(FormatError(
                "unsupported CUR DIB layout; expected bottom-up 4-bit, 8-bit, 24-bit or 32-bit BI_RGB",
            ));
        }
        let bits = word(b, 14);
        let colors = match bits {
            24 | 32 => {
                if dword(b, 32) != 0 {
                    return Err(FormatError("24-bit or 32-bit CUR palette is unsupported"));
                }
                0
            }
            4 | 8 => {
                let max_colors = 1usize << bits;
                let used = if dword(b, 32) == 0 {
                    max_colors
                } else {
                    dword(b, 32) as usize
                };
                if used > max_colors {
                    return Err(FormatError("CUR palette exceeds bit-depth limit"));
                }
                used
            }
            _ => unreachable!("validated bit depth"),
        };
        let xor_stride = stride(width, bits);
        let and_stride = stride(width, 1);
        let pixels = 40 + colors * 4;
        let mask = pixels + xor_stride * height;
        if mask + and_stride * height != b.len() {
            return Err(FormatError("CUR palette or pixel planes are truncated"));
        }
        Ok(Self {
            width,
            height,
            hotspot,
            bits,
            palette: &b[40..pixels],
            xor: &b[pixels..mask],
            and: &b[mask..],
        })
    }

    #[cfg(feature = "encode")]
    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let (w, h) = (self.width, self.height);
        if !(1..=256).contains(&w)
            || !(1..=256).contains(&h)
            || !matches!(self.bits, 4 | 8 | 24 | 32)
            || !self.palette.len().is_multiple_of(4)
            || self.palette.len() > 1024
            || self.xor.len() != stride(w, self.bits) * h
            || self.and.len() != stride(w, 1) * h
        {
            return Err(FormatError("invalid CUR dimensions, palette or planes"));
        }
        let colors = self.palette.len() / 4;
        if (self.bits <= 8 && !(1..=1 << self.bits).contains(&colors))
            || (self.bits > 8 && colors != 0)
        {
            return Err(FormatError("invalid CUR palette for bit depth"));
        }
        let mut out = vec![0; 62];
        for (at, v) in [
            (2, 2),
            (4, 1),
            (10, self.hotspot.0),
            (12, self.hotspot.1),
            (34, 1),
            (36, self.bits),
        ] {
            out[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }
        out[6] = w as u8;
        out[7] = h as u8;
        out[8] = colors as u8;
        let payload = self.xor.len() + self.and.len();
        for (at, v) in [
            (14, 40 + self.palette.len() + payload),
            (18, 22),
            (22, 40),
            (26, w),
            (30, h * 2),
            (42, payload),
            (54, colors),
        ] {
            out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
        }
        out.extend(self.palette);
        out.extend(self.xor);
        out.extend(self.and);
        CursorBitmap::decode(&out)?;
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorBitmap {
    width: usize,
    height: usize,
    hotspot: (u16, u16),
    xor_rgb: Vec<[u8; 3]>,
    and_mask: Vec<bool>,
    alpha: Option<Vec<u8>>,
}
impl CursorBitmap {
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn hotspot(&self) -> (u16, u16) {
        self.hotspot
    }
    /// Top-down RGB values: XOR colors for legacy cursors, straight colors with alpha.
    pub fn xor_rgb(&self) -> &[[u8; 3]] {
        &self.xor_rgb
    }
    pub fn and_mask(&self) -> &[bool] {
        &self.and_mask
    }
    /// A nonzero 32-bit alpha plane selects alpha blending instead of AND/XOR.
    pub fn alpha(&self) -> Option<&[u8]> {
        self.alpha.as_deref()
    }
    /// Straight RGBA for native cursors; legacy inversion cannot be represented.
    pub fn rgba(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        for (i, rgb) in self.xor_rgb.iter().enumerate() {
            let alpha = if let Some(alpha) = &self.alpha {
                alpha[i]
            } else {
                if self.and_mask[i] && *rgb != [0, 0, 0] {
                    return Err(FormatError(
                        "native RGBA cursor cannot represent AND/XOR inversion",
                    ));
                }
                if self.and_mask[i] { 0 } else { 255 }
            };
            out.extend_from_slice(rgb);
            out.push(alpha);
        }
        Ok(out)
    }
    pub fn decode(raw: &[u8]) -> Result<Self, FormatError> {
        let dib = CursorDib::parse(raw)?;
        let CursorDib {
            width,
            height,
            hotspot,
            bits,
            palette,
            xor,
            and,
        } = dib;
        let xor_stride = stride(width, bits);
        let and_stride = stride(width, 1);
        let mut xor_rgb = Vec::with_capacity(width * height);
        let mut and_mask = Vec::with_capacity(width * height);
        let mut alpha = Vec::with_capacity(if bits == 32 { width * height } else { 0 });
        for y in (0..height).rev() {
            for x in 0..width {
                let color = match bits {
                    32 => {
                        let at = y * xor_stride + x * 4;
                        alpha.push(xor[at + 3]);
                        &xor[at..]
                    }
                    24 => &xor[y * xor_stride + x * 3..],
                    _ => {
                        let index = if bits == 4 {
                            let byte = xor[y * xor_stride + x / 2];
                            usize::from(if x.is_multiple_of(2) {
                                byte >> 4
                            } else {
                                byte & 0x0f
                            })
                        } else {
                            usize::from(xor[y * xor_stride + x])
                        };
                        if index >= palette.len() / 4 {
                            return Err(FormatError("CUR pixel index outside palette"));
                        }
                        &palette[index * 4..]
                    }
                };
                xor_rgb.push([color[2], color[1], color[0]]);
                and_mask.push(and[y * and_stride + x / 8] & (128 >> (x % 8)) != 0);
            }
        }
        Ok(Self {
            width,
            height,
            hotspot,
            xor_rgb,
            and_mask,
            alpha: alpha.iter().any(|&a| a != 0).then_some(alpha),
        })
    }
    /// Composites over exactly one cursor-sized top-down RGB background.
    pub fn composite(&self, background: &[[u8; 3]]) -> Result<Vec<[u8; 3]>, FormatError> {
        if background.len() != self.xor_rgb.len() {
            return Err(FormatError("cursor background dimensions differ"));
        }
        if let Some(alpha) = &self.alpha {
            return Ok(background
                .iter()
                .zip(&self.xor_rgb)
                .zip(alpha)
                .map(|((old, rgb), a)| {
                    std::array::from_fn(|i| {
                        ((u32::from(rgb[i]) * u32::from(*a)
                            + u32::from(old[i]) * (255 - u32::from(*a))
                            + 127)
                            / 255) as u8
                    })
                })
                .collect());
        }
        Ok(background
            .iter()
            .zip(&self.xor_rgb)
            .zip(&self.and_mask)
            .map(|((old, xor), and)| {
                std::array::from_fn(|i| (if *and { old[i] } else { 0 }) ^ xor[i])
            })
            .collect())
    }
}

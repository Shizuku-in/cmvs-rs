//! Single-image, 8-bit and 32-bit BI_RGB CUR files. No OS cursor handles.
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
            || !matches!(word(b, 14), 8 | 32)
            || dword(b, 16) != 0
        {
            return Err(FormatError(
                "unsupported CUR DIB layout; expected bottom-up 8-bit or 32-bit BI_RGB",
            ));
        }
        let bits = word(b, 14);
        let colors = if bits == 32 {
            if dword(b, 32) != 0 {
                return Err(FormatError("32-bit CUR palette is unsupported"));
            }
            0
        } else if dword(b, 32) == 0 {
            256
        } else {
            dword(b, 32) as usize
        };
        if colors > 256 {
            return Err(FormatError("CUR palette exceeds 256 entries"));
        }
        let xor_stride = if bits == 32 {
            width * 4
        } else {
            width.div_ceil(4) * 4
        };
        let and_stride = width.div_ceil(32) * 4;
        let pixels = 40 + colors * 4;
        let mask = pixels + xor_stride * height;
        if mask + and_stride * height != b.len() {
            return Err(FormatError("CUR palette or pixel planes are truncated"));
        }
        let mut xor_rgb = Vec::with_capacity(width * height);
        let mut and_mask = Vec::with_capacity(width * height);
        let mut alpha = Vec::with_capacity(if bits == 32 { width * height } else { 0 });
        for y in (0..height).rev() {
            for x in 0..width {
                let color = if bits == 32 {
                    let color = pixels + y * xor_stride + x * 4;
                    alpha.push(b[color + 3]);
                    color
                } else {
                    let index = usize::from(b[pixels + y * xor_stride + x]);
                    if index >= colors {
                        return Err(FormatError("CUR pixel index outside palette"));
                    }
                    40 + index * 4
                };
                xor_rgb.push([b[color + 2], b[color + 1], b[color]]);
                and_mask.push(b[mask + y * and_stride + x / 8] & (128 >> (x % 8)) != 0);
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

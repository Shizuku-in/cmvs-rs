//! Current game's single-image, 8-bit BI_RGB CUR files. No OS cursor handles.
//! Preserve AND/XOR semantics; not every legacy cursor can be represented by RGBA.
use crate::FormatError;

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
    /// Top-down RGB XOR values, with one corresponding AND bit per pixel.
    pub fn xor_rgb(&self) -> &[[u8; 3]] {
        &self.xor_rgb
    }
    pub fn and_mask(&self) -> &[bool] {
        &self.and_mask
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
            || word(b, 14) != 8
            || dword(b, 16) != 0
        {
            return Err(FormatError(
                "unsupported CUR DIB layout; expected bottom-up 8-bit BI_RGB",
            ));
        }
        let colors = if dword(b, 32) == 0 {
            256
        } else {
            dword(b, 32) as usize
        };
        if colors > 256 {
            return Err(FormatError("CUR palette exceeds 256 entries"));
        }
        let xor_stride = width.div_ceil(4) * 4;
        let and_stride = width.div_ceil(32) * 4;
        let pixels = 40 + colors * 4;
        let mask = pixels + xor_stride * height;
        if mask + and_stride * height != b.len() {
            return Err(FormatError("CUR palette or pixel planes are truncated"));
        }
        let mut xor_rgb = Vec::with_capacity(width * height);
        let mut and_mask = Vec::with_capacity(width * height);
        for y in (0..height).rev() {
            for x in 0..width {
                let index = usize::from(b[pixels + y * xor_stride + x]);
                if index >= colors {
                    return Err(FormatError("CUR pixel index outside palette"));
                }
                let color = 40 + index * 4;
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
        })
    }
    /// Composites over exactly one cursor-sized top-down RGB background.
    pub fn composite(&self, background: &[[u8; 3]]) -> Result<Vec<[u8; 3]>, FormatError> {
        if background.len() != self.xor_rgb.len() {
            return Err(FormatError("cursor background dimensions differ"));
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

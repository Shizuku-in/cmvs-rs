//! Bounded top-down straight-alpha image and explicit resource-preview compositor.
use crate::{FormatError, MAX_PAYLOAD};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl RgbaImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FormatError> {
        let size = (u64::from(width) * u64::from(height))
            .checked_mul(4)
            .ok_or(FormatError("RGBA image byte size overflow"))?;
        if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 || size != pixels.len() as u64 {
            return Err(FormatError("invalid or oversized RGBA image"));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    /// Nearest-neighbor, centered aspect fit over 0x00RRGGBB; not CMVS scene blending.
    pub fn draw_fit(&self, target: &mut [u32], width: u32, height: u32) -> Result<(), FormatError> {
        let size = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 / 4 || size != target.len() as u64
        {
            return Err(FormatError("invalid preview surface dimensions"));
        }
        let mut w = u64::from(width);
        let mut h = (u64::from(self.height) * w / u64::from(self.width)).max(1);
        if h > u64::from(height) {
            h = u64::from(height);
            w = (u64::from(self.width) * h / u64::from(self.height)).max(1);
        }
        let left = (u64::from(width) - w) / 2;
        let top = (u64::from(height) - h) / 2;
        for y in 0..h {
            for x in 0..w {
                let sx = x * u64::from(self.width) / w;
                let sy = y * u64::from(self.height) / h;
                let at = ((sy * u64::from(self.width) + sx) * 4) as usize;
                let p = &self.pixels[at..at + 4];
                let dst = &mut target[((top + y) * u64::from(width) + left + x) as usize];
                let alpha = u32::from(p[3]);
                let channel = |i: usize, shift: u32| {
                    ((u32::from(p[i]) * alpha + ((*dst >> shift) & 255) * (255 - alpha) + 127)
                        / 255)
                        << shift
                };
                *dst = channel(0, 16) | channel(1, 8) | channel(2, 0);
            }
        }
        Ok(())
    }
}

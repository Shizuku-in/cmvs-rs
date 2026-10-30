//! Bounded top-down straight-alpha image, explicit resource-preview compositor and an
//! uncompressed PNG writer for research frame exports.
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
    /// Software fullscreen presentation using the same mapping as pointer input.
    pub fn draw_aspect(
        &self,
        target: &mut [u32],
        width: u32,
        height: u32,
        mode: u32,
    ) -> Result<(), FormatError> {
        let view =
            crate::viewport::Viewport::new([self.width, self.height], [width, height], mode)?;
        if target.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(FormatError("invalid preview surface dimensions"));
        }
        for y in view.top.max(0)..(view.top + i64::from(view.height)).min(i64::from(height)) {
            for x in view.left.max(0)..(view.left + i64::from(view.width)).min(i64::from(width)) {
                let sx = (x - view.left) as u64 * u64::from(self.width) / u64::from(view.width);
                let sy = (y - view.top) as u64 * u64::from(self.height) / u64::from(view.height);
                let at = ((sy * u64::from(self.width) + sx) * 4) as usize;
                let p = &self.pixels[at..at + 4];
                let dst = &mut target[y as usize * width as usize + x as usize];
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
    /// RGBA8 PNG with filter 0 rows and stored (uncompressed) deflate blocks, so no
    /// compression library is needed. Intended for exporting rendered research frames.
    pub fn to_png(&self) -> Vec<u8> {
        let row = self.width as usize * 4;
        let mut raw = Vec::with_capacity((row + 1) * self.height as usize);
        for line in self.pixels.chunks_exact(row) {
            raw.push(0);
            raw.extend_from_slice(line);
        }
        let mut zlib = vec![0x78, 0x01];
        let blocks = raw.chunks(65535);
        let count = blocks.len();
        for (i, block) in blocks.enumerate() {
            let len = block.len() as u16;
            zlib.push(u8::from(i + 1 == count));
            zlib.extend(len.to_le_bytes());
            zlib.extend((!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend(adler32(&raw).to_be_bytes());
        let mut header = Vec::with_capacity(13);
        header.extend(self.width.to_be_bytes());
        header.extend(self.height.to_be_bytes());
        header.extend([8, 6, 0, 0, 0]);
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        for (kind, data) in [
            (b"IHDR", &header[..]),
            (b"IDAT", &zlib[..]),
            (b"IEND", &[][..]),
        ] {
            png.extend((data.len() as u32).to_be_bytes());
            let start = png.len();
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            let crc = crc32(&png[start..]);
            png.extend(crc.to_be_bytes());
        }
        png
    }
}

/// CRC-32 (ISO-HDLC, reflected 0xEDB88320) as PNG chunks require.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, entry) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    !bytes.iter().fold(!0u32, |c, &b| {
        table[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8)
    })
}

/// Adler-32 checksum that ends a zlib stream.
pub fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

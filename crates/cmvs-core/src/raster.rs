//! Bounded top-down straight-alpha image, explicit resource-preview compositor and an
//! uncompressed PNG writer for research frame exports.
use crate::{FormatError, MAX_PAYLOAD};
use std::sync::{Arc, OnceLock};

#[derive(Debug)]
struct ImageData {
    pixels: Vec<u8>,
    content_digest: OnceLock<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    // Clones share both immutable pixels and the digest, including when the first
    // cache lookup happens on a temporary clone (e.g. a prepared glyph object).
    data: Arc<ImageData>,
    /// Set for decoded movie frames, which replace each other and are never shared.
    frame: Option<u64>,
}

/// How a texture cache recognizes an image without comparing pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheKey {
    /// Equal pixel content and dimensions share one texture.
    Content([u8; 32], u32, u32),
    /// One decoded movie frame and its clones. Hashing it would cost more than the
    /// decode, and no other image shares its content.
    Frame(u64),
}
impl PartialEq for RgbaImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels() == other.pixels()
    }
}
impl Eq for RgbaImage {}
impl RgbaImage {
    /// Lazily hashes immutable pixels once. Cache state is not part of image equality.
    pub fn content_digest(&self) -> &[u8; 32] {
        self.data
            .content_digest
            .get_or_init(|| crate::digest::sha256(self.pixels()))
    }

    /// Marks a decoded movie frame with an identity unique within the process.
    pub fn into_frame(mut self) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.frame = Some(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        self
    }

    /// The decoded movie frame identity, without hashing other images.
    pub fn frame_id(&self) -> Option<u64> {
        self.frame
    }

    /// Movie frames are keyed by identity; every other image by its content digest.
    pub fn cache_key(&self) -> CacheKey {
        match self.frame {
            Some(frame) => CacheKey::Frame(frame),
            None => CacheKey::Content(*self.content_digest(), self.width, self.height),
        }
    }

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
            data: Arc::new(ImageData {
                pixels,
                content_digest: OnceLock::new(),
            }),
            frame: None,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.data.pixels
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
                let p = &self.pixels()[at..at + 4];
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
        self.draw_scaled(target, width, height, mode, false)
    }
    /// `linear` selects the render mode 1 counterpart of StretchRect D3DTEXF_LINEAR
    /// (0x140029320): texel centers, clamped edges and 8-bit weights over
    /// premultiplied texels. It is a software filter, not a GPU-exact match.
    pub fn draw_scaled(
        &self,
        target: &mut [u32],
        width: u32,
        height: u32,
        mode: u32,
        linear: bool,
    ) -> Result<(), FormatError> {
        let view =
            crate::viewport::Viewport::new([self.width, self.height], [width, height], mode)?;
        if target.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(FormatError("invalid preview surface dimensions"));
        }
        let xs = view.left.max(0)..(view.left + i64::from(view.width)).min(i64::from(width));
        let ys = view.top.max(0)..(view.top + i64::from(view.height)).min(i64::from(height));
        if linear {
            // Per-axis source pair and weight for each target column/row in view.
            let taps = |range: std::ops::Range<i64>, origin: i64, span: u32, source: u32| {
                range
                    .map(|d| {
                        let center = (2 * (d - origin) + 1) * i64::from(source) * 256
                            / (2 * i64::from(span))
                            - 128;
                        let at = center.clamp(0, (i64::from(source) - 1) * 256);
                        let low = (at >> 8) as usize;
                        (low, (low + 1).min(source as usize - 1), (at & 255) as u64)
                    })
                    .collect::<Vec<_>>()
            };
            let columns = taps(xs.clone(), view.left, view.width, self.width);
            let rows = taps(ys.clone(), view.top, view.height, self.height);
            let stride = self.width as usize;
            for (y, &(y0, y1, fy)) in ys.zip(&rows) {
                for (x, &(x0, x1, fx)) in xs.clone().zip(&columns) {
                    let weights = [
                        (y0 * stride + x0, (256 - fx) * (256 - fy)),
                        (y0 * stride + x1, fx * (256 - fy)),
                        (y1 * stride + x0, (256 - fx) * fy),
                        (y1 * stride + x1, fx * fy),
                    ];
                    let mut sums = [0u64; 4];
                    for (at, weight) in weights {
                        let p = &self.pixels()[at * 4..at * 4 + 4];
                        let alpha = u64::from(p[3]);
                        for (channel, sum) in sums[..3].iter_mut().enumerate() {
                            *sum += weight * u64::from(p[channel]) * alpha;
                        }
                        sums[3] += weight * alpha;
                    }
                    let dst = &mut target[y as usize * width as usize + x as usize];
                    // Exact rational composite: (premul + dst * (1 - alpha)) / 255.
                    let scale = 255 * 65536;
                    let channel = |i: usize, shift: u32| {
                        let under = u64::from((*dst >> shift) & 255);
                        (((sums[i] + under * (scale - sums[3]) + scale / 2) / scale) as u32)
                            << shift
                    };
                    *dst = channel(0, 16) | channel(1, 8) | channel(2, 0);
                }
            }
            return Ok(());
        }
        for y in ys {
            for x in xs.clone() {
                let sx = (x - view.left) as u64 * u64::from(self.width) / u64::from(view.width);
                let sy = (y - view.top) as u64 * u64::from(self.height) / u64::from(view.height);
                let at = ((sy * u64::from(self.width) + sx) * 4) as usize;
                let p = &self.pixels()[at..at + 4];
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
        for line in self.pixels().chunks_exact(row) {
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

#[cfg(test)]
mod content_digest_tests {
    use super::*;
    #[test]
    fn temporary_clones_share_pixels_and_initialize_the_cached_digest() {
        let cached = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let prepared = cached.clone();
        assert!(std::ptr::eq(cached.pixels(), prepared.pixels()));
        assert!(cached.data.content_digest.get().is_none());
        let key = prepared.cache_key();
        drop(prepared);
        assert_eq!(
            cached.data.content_digest.get(),
            Some(&crate::digest::sha256(cached.pixels()))
        );
        assert_eq!(cached.clone().cache_key(), key);
        assert_eq!(cached, RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap());
    }

    #[test]
    fn digest_cache_does_not_change_equality_and_tracks_pixel_content() {
        let first = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let second = first.clone();
        assert_eq!(
            first.content_digest(),
            &crate::digest::sha256(first.pixels())
        );
        assert_eq!(first, second);
        assert_eq!(first, first.clone());
        let changed = RgbaImage::new(1, 1, vec![1, 2, 3, 5]).unwrap();
        assert_ne!(first.content_digest(), changed.content_digest());
    }
    #[test]
    fn frames_have_their_own_identity_and_keep_it_when_cloned() {
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let content = image.cache_key();
        assert_eq!(
            content,
            CacheKey::Content(crate::digest::sha256(image.pixels()), 1, 1)
        );
        let first = image.clone().into_frame();
        let second = image.into_frame();
        assert!(matches!(first.cache_key(), CacheKey::Frame(_)));
        assert_eq!(
            first.cache_key(),
            CacheKey::Frame(first.frame_id().unwrap())
        );
        assert_eq!(RgbaImage::new(1, 1, vec![0; 4]).unwrap().frame_id(), None);
        assert_ne!(first.cache_key(), second.cache_key());
        assert_eq!(first.cache_key(), first.clone().cache_key());
        assert_eq!(first, second, "identity is not part of equality");
    }
}

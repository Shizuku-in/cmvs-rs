//! Bounded top-down straight-alpha image, explicit resource-preview compositor and an
//! uncompressed PNG writer for research frame exports.
use crate::{FormatError, MAX_PAYLOAD};
use std::sync::{Arc, OnceLock};

struct ImageData {
    pixels: Vec<u8>,
    content_digest: OnceLock<u128>,
    opaque: OnceLock<bool>,
}

/// Immutable top-down RGBA pixels with straight (unpremultiplied) alpha.
///
/// Rows have no padding and each pixel has four bytes in red, green, blue, alpha
/// order. Clones share pixel storage. Dimensions and storage are validated by
/// [`Self::new`].
#[derive(Clone)]
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
    /// Equal pixel content and dimensions share one texture. The hash is 128 bits of
    /// [`content_hash`], not a cryptographic digest: images are not adversarial here.
    /// Decoded image files carry a hash of what they were decoded from instead.
    Content(u128, u32, u32),
    /// One decoded movie frame and its clones. Hashing it would cost more than the
    /// decode, and no other image shares its content.
    Frame(u64),
}
/// Omits the lazily cached digest and the process-unique frame identity, so equal state
/// formats identically across runs in one process.
impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgbaImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &self.data.pixels)
            .field("movie_frame", &self.frame.is_some())
            .finish()
    }
}
impl PartialEq for RgbaImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels() == other.pixels()
    }
}
impl Eq for RgbaImage {}
impl RgbaImage {
    /// Lazily hashes immutable pixels once, unless the image was identified by its
    /// source. Cache state is not part of image equality.
    pub fn content_digest(&self) -> u128 {
        *self
            .data
            .content_digest
            .get_or_init(|| content_hash(self.pixels()))
    }

    /// Stands in for the content hash with one derived from `identity`, which must
    /// determine the pixels: the identity of the bytes an image was decoded from. Saves
    /// hashing the decoded pixels, which are many times larger. Images with equal pixels
    /// but different identities then no longer share a texture. Has no effect once the
    /// digest exists.
    pub(crate) fn identify_content(&self, identity: u128) {
        let mut tagged = *b"decoded image\0\0\0################";
        tagged[16..].copy_from_slice(&identity.to_le_bytes());
        let _ = self.data.content_digest.set(content_hash(&tagged));
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
            None => CacheKey::Content(self.content_digest(), self.width, self.height),
        }
    }

    /// Create an image from exactly `width * height * 4` bytes.
    ///
    /// Rejects zero dimensions, incorrect byte lengths, and images exceeding
    /// [`MAX_PAYLOAD`].
    ///
    /// ```
    /// use cmvs_core::raster::RgbaImage;
    ///
    /// let image = RgbaImage::new(1, 1, vec![255, 0, 0, 255])?;
    /// assert_eq!(image.pixels(), &[255, 0, 0, 255]);
    /// # Ok::<(), cmvs_core::FormatError>(())
    /// ```
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
                opaque: OnceLock::new(),
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
    /// Shared immutable pixels need at most one alpha scan, including across clones.
    pub(crate) fn is_opaque(&self) -> bool {
        *self
            .data
            .opaque
            .get_or_init(|| self.pixels().as_chunks::<4>().0.iter().all(|p| p[3] == 255))
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
        let opaque = self.is_opaque();
        if view.width == self.width && view.height == self.height {
            // Both filters sample exact texels at native size, including centered
            // views and cropped cover modes. Avoid coordinate division and four taps.
            let columns = (xs.end - xs.start) as usize;
            let source_x = (xs.start - view.left) as usize;
            for y in ys {
                let source = ((y - view.top) as usize * self.width as usize + source_x) * 4;
                let target_at = y as usize * width as usize + xs.start as usize;
                let source = &self.pixels()[source..source + columns * 4];
                let target = &mut target[target_at..target_at + columns];
                if opaque {
                    for (target, pixel) in target.iter_mut().zip(source.as_chunks::<4>().0) {
                        *target = rgb(*pixel);
                    }
                } else {
                    for (target, pixel) in target.iter_mut().zip(source.as_chunks::<4>().0) {
                        composite(target, *pixel);
                    }
                }
            }
            return Ok(());
        }
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
            if opaque {
                // Adjacent output rows often share source rows. Keep horizontal
                // RGB sums at 8-bit fractional precision and round only at output.
                let mut first = vec![[0u32; 3]; columns.len()];
                let mut second = first.clone();
                let (mut first_y, mut second_y) = (None, None);
                let sample_row = |y: usize, row: &mut [[u32; 3]]| {
                    let source = self.pixels()[y * stride * 4..][..stride * 4]
                        .as_chunks::<4>()
                        .0;
                    for (out, &(x0, x1, fx)) in row.iter_mut().zip(&columns) {
                        let (a, b, fx) = (source[x0], source[x1], fx as u32);
                        *out = std::array::from_fn(|c| {
                            u32::from(a[c]) * (256 - fx) + u32::from(b[c]) * fx
                        });
                    }
                };
                for (y, &(y0, y1, fy)) in ys.zip(&rows) {
                    if second_y == Some(y0) {
                        std::mem::swap(&mut first, &mut second);
                        std::mem::swap(&mut first_y, &mut second_y);
                    }
                    if first_y != Some(y0) {
                        sample_row(y0, &mut first);
                        first_y = Some(y0);
                    }
                    if y1 != y0 && second_y != Some(y1) {
                        sample_row(y1, &mut second);
                        second_y = Some(y1);
                    }
                    let below = if y0 == y1 { &first } else { &second };
                    let fy = fy as u32;
                    let target_at = y as usize * width as usize + xs.start as usize;
                    let target = &mut target[target_at..target_at + columns.len()];
                    for ((target, a), b) in target.iter_mut().zip(&first).zip(below) {
                        let channel = |c| (a[c] * (256 - fy) + b[c] * fy + 32768) >> 16;
                        *target = channel(0) << 16 | channel(1) << 8 | channel(2);
                    }
                }
                return Ok(());
            }
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
        if opaque {
            let columns: Vec<_> = xs
                .clone()
                .map(|x| {
                    ((x - view.left) as u64 * u64::from(self.width) / u64::from(view.width))
                        as usize
                })
                .collect();
            for y in ys {
                let source_y =
                    (y - view.top) as u64 * u64::from(self.height) / u64::from(view.height);
                let source_at = source_y as usize * self.width as usize * 4;
                let source = self.pixels()[source_at..][..self.width as usize * 4]
                    .as_chunks::<4>()
                    .0;
                let target_at = y as usize * width as usize + xs.start as usize;
                let target = &mut target[target_at..target_at + columns.len()];
                for (target, &source_x) in target.iter_mut().zip(&columns) {
                    *target = rgb(source[source_x]);
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

fn rgb([r, g, b, _]: [u8; 4]) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

fn composite(target: &mut u32, pixel: [u8; 4]) {
    let alpha = u32::from(pixel[3]);
    let channel = |i: usize, shift: u32| {
        ((u32::from(pixel[i]) * alpha + ((*target >> shift) & 255) * (255 - alpha) + 127) / 255)
            << shift
    };
    *target = channel(0, 16) | channel(1, 8) | channel(2, 0);
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

/// Texture-cache identity of pixel content: four independent lanes of folded 64x64-bit
/// multiplies over 64-byte stripes (the wyhash construction), finished with the length.
/// A software SHA-256 cost more than decoding the image on console CPUs.
pub fn content_hash(bytes: &[u8]) -> u128 {
    const K: [u64; 6] = [
        0xa076_1d64_78bd_642f,
        0xe703_7ed1_a0b4_28db,
        0x8ebc_6af0_9c88_c6e3,
        0x5899_65cc_7537_4cc3,
        0x1d8e_4e27_c47d_124f,
        0x9e37_79b9_7f4a_7c15,
    ];
    #[inline(always)]
    fn fold(a: u64, b: u64) -> u64 {
        let m = u128::from(a) * u128::from(b);
        (m as u64) ^ (m >> 64) as u64
    }
    #[inline(always)]
    fn stripe(lanes: &mut [u64; 4], stripe: &[u8; 64]) {
        for (i, lane) in lanes.iter_mut().enumerate() {
            let a = u64::from_le_bytes(stripe[i * 16..i * 16 + 8].try_into().unwrap());
            let b = u64::from_le_bytes(stripe[i * 16 + 8..i * 16 + 16].try_into().unwrap());
            *lane = fold(a ^ *lane, b ^ K[i]);
        }
    }
    let length = bytes.len() as u64;
    let mut lanes = [K[0] ^ length, K[1], K[2], K[3]];
    let (stripes, tail) = bytes.as_chunks::<64>();
    for chunk in stripes {
        stripe(&mut lanes, chunk);
    }
    let mut last = [0u8; 64];
    last[..tail.len()].copy_from_slice(tail);
    stripe(&mut lanes, &last);
    let low = fold(lanes[0] ^ K[4], lanes[1] ^ length);
    let high = fold(lanes[2] ^ K[5], lanes[3] ^ length.rotate_left(32));
    let low = fold(low ^ K[0], high ^ K[1]) ^ low;
    let high = fold(high ^ K[2], low ^ K[3]) ^ high;
    u128::from(low) | u128::from(high) << 64
}

#[cfg(test)]
mod content_digest_tests {
    use super::*;
    #[test]
    fn clones_share_alpha_classification_without_changing_image_identity() {
        for alpha in [0, 254, 255] {
            let image = RgbaImage::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, alpha]).unwrap();
            let clone = image.clone();
            let key = image.cache_key();
            assert!(image.data.opaque.get().is_none());
            assert_eq!(clone.is_opaque(), alpha == 255);
            assert_eq!(image.data.opaque.get(), Some(&(alpha == 255)));
            assert_eq!(image.cache_key(), key);
            assert_eq!(image, clone);
        }
    }

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
            Some(&content_hash(cached.pixels()))
        );
        assert_eq!(cached.clone().cache_key(), key);
        assert_eq!(cached, RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap());
    }

    #[test]
    fn a_source_identity_stands_in_for_the_pixel_hash_until_one_exists() {
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let other = image.clone();
        image.identify_content(7);
        let key = image.content_digest();
        assert_ne!(key, content_hash(image.pixels()));
        // Clones share it, a later identity does not replace it, and it is stable.
        other.identify_content(8);
        assert_eq!(other.content_digest(), key);
        let same = RgbaImage::new(1, 1, vec![9; 4]).unwrap();
        same.identify_content(7);
        assert_eq!(same.content_digest(), key);
        let different = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        different.identify_content(8);
        assert_ne!(different.content_digest(), key);
        // An image already hashed keeps its pixel hash.
        let hashed = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let pixels = hashed.content_digest();
        hashed.identify_content(7);
        assert_eq!(hashed.content_digest(), pixels);
        assert_eq!(image, hashed);
    }

    #[test]
    fn digest_cache_does_not_change_equality_and_tracks_pixel_content() {
        let first = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let second = first.clone();
        assert_eq!(first.content_digest(), content_hash(first.pixels()));
        assert_eq!(first, second);
        assert_eq!(first, first.clone());
        let changed = RgbaImage::new(1, 1, vec![1, 2, 3, 5]).unwrap();
        assert_ne!(first.content_digest(), changed.content_digest());
    }
    #[test]
    fn content_hash_depends_on_every_byte_and_on_the_length() {
        let mut seen = std::collections::HashMap::new();
        let mut check = |bytes: Vec<u8>| {
            let hash = content_hash(&bytes);
            let first = seen.entry(hash).or_insert_with(|| bytes.clone());
            assert_eq!(*first, bytes, "different inputs share {hash:#x}");
        };
        for length in 0..200usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 7 + 3) as u8).collect();
            // Trailing zeros are padding inside the last stripe; the length tells them apart.
            check([bytes.as_slice(), &[0]].concat());
            for at in 0..length {
                let mut flipped = bytes.clone();
                flipped[at] ^= 1;
                check(flipped);
            }
            check(bytes);
        }
    }

    #[test]
    fn frames_have_their_own_identity_and_keep_it_when_cloned() {
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let content = image.cache_key();
        assert_eq!(
            content,
            CacheKey::Content(content_hash(image.pixels()), 1, 1)
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

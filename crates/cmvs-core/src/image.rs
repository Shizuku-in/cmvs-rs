//! Image-specific lookup and codec dispatch; shared I/O lives in `resources`.
use crate::{
    FormatError, pb3,
    raster::RgbaImage,
    resources::{RegisteredResources, ResourceArchives, ResourceKind},
};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    path::Path,
};
mod cache;
use cache::Cache;
/// 0x2011 / 0x1400717c0: image manager has ten registration slots.
pub const IMAGE_ENTRIES: u32 = ResourceKind::Image.slot_count();
pub trait ImageSource {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
    /// 0x203f: arm the next image-file read's raw-byte cache lookup/insertion.
    fn cache_next_read(&mut self) -> Result<(), Box<dyn Error>> {
        Err("image source does not implement the next-read cache".into())
    }
    /// Begin a decode read. Sources without caches can use the default implementation.
    fn read_image(&mut self, name: &[u8]) -> Result<Option<ImageRead>, Box<dyn Error>> {
        Ok(self.read(name)?.map(|bytes| ImageRead {
            bytes,
            cache_key: None,
        }))
    }
    /// Complete even a failed decode: the original inserts raw bytes after decoding.
    fn finish_image(&mut self, _read: ImageRead) {}
    /// Host-side decoded-image cache, not part of the original: a decode of exactly
    /// these bytes (and reference chain) seen before. `identity` comes from [`load`].
    fn caches_decodes(&self) -> bool {
        false
    }
    fn cached_decode(&mut self, _identity: u128) -> Option<DecodedImage> {
        None
    }
    fn store_decode(&mut self, _identity: u128, _image: &DecodedImage) {}
}

/// Decoded images by the identity of their bytes, least recently used first out.
/// Hits share pixels (and the texture cache's content hash) with the first decode.
#[derive(Default)]
struct DecodedCache {
    entries: VecDeque<(u128, DecodedImage)>,
    bytes: usize,
    limit: usize,
}
impl DecodedCache {
    fn get(&mut self, identity: u128) -> Option<DecodedImage> {
        let at = self.entries.iter().position(|(id, _)| *id == identity)?;
        let entry = self.entries.remove(at).expect("found above");
        let image = entry.1.clone();
        self.entries.push_front(entry);
        Some(image)
    }
    fn evict_to(&mut self, bytes: usize) {
        while self.bytes > bytes {
            let (_, old) = self.entries.pop_back().expect("bytes imply entries");
            self.bytes -= old.pixels.pixels().len();
        }
    }
    fn insert(&mut self, identity: u128, image: &DecodedImage) {
        let size = image.pixels.pixels().len();
        if size > self.limit || self.entries.iter().any(|(id, _)| *id == identity) {
            return;
        }
        self.evict_to(self.limit - size);
        self.bytes += size;
        self.entries.push_front((identity, image.clone()));
    }
}
/// Owned read with an opaque deferred-cache token. This keeps outer insertions after
/// recursive reference reads, without retaining pointers into an evictable cache.
pub struct ImageRead {
    bytes: Vec<u8>,
    cache_key: Option<Vec<u8>>,
}
/// Compatibility image view over the common read-only resource store.
pub struct ImageArchives {
    archives: ResourceArchives,
    cache: Cache,
    decoded: DecodedCache,
}
impl ImageArchives {
    pub fn configure_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), crate::cpz::Cpz7Parameters>,
    ) {
        self.archives.configure_parameters(parameters);
    }
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            archives: ResourceArchives::new(root)?,
            cache: Cache::default(),
            decoded: DecodedCache::default(),
        })
    }
    /// Keeps up to `bytes` of decoded RGBA for images loaded again (0, the default,
    /// disables it). Results are unchanged; repeated loads skip decoding.
    pub fn set_decoded_cache_limit(&mut self, bytes: usize) {
        self.decoded.limit = bytes;
        self.decoded.evict_to(bytes);
    }
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Registered<'a> {
        Registered {
            inner: self.archives.with_paths(paths, ResourceKind::Image),
            cache: &mut self.cache,
            decoded: &mut self.decoded,
        }
    }
}
pub struct Registered<'a> {
    inner: RegisteredResources<'a>,
    cache: &'a mut Cache,
    decoded: &'a mut DecodedCache,
}
impl ImageSource for Registered<'_> {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.cache.read(name, &mut self.inner)
    }
    fn cache_next_read(&mut self) -> Result<(), Box<dyn Error>> {
        self.cache.arm();
        Ok(())
    }
    fn read_image(&mut self, name: &[u8]) -> Result<Option<ImageRead>, Box<dyn Error>> {
        self.cache.begin(name, &mut self.inner)
    }
    fn finish_image(&mut self, read: ImageRead) {
        self.cache.finish(read);
    }
    fn caches_decodes(&self) -> bool {
        self.decoded.limit > 0
    }
    fn cached_decode(&mut self, identity: u128) -> Option<DecodedImage> {
        self.decoded.get(identity)
    }
    fn store_decode(&mut self, identity: u128, image: &DecodedImage) {
        self.decoded.insert(identity, image);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub pixels: RgbaImage,
    /// Image info +24 (object image +1152): the source has 32 bits per pixel.
    pub alpha: bool,
}

/// 0x140036700 picks the codec by magic; PB3B types 1, 2, 3 and 5 decode here. Types 2/3
/// share 0x1400a0700 (Aoitori 0x1400a76f0); other known codecs
/// are explicit errors. Ok(None) is the original quiet failure for an unknown magic or a
/// PB3 type outside 1..8: the load returns 0 without a MessageBox.
pub fn decode(bytes: &[u8]) -> Result<Option<DecodedImage>, FormatError> {
    let unsupported = FormatError(
        "image codec not implemented; standalone PB3 types 1, 2, 3, 5 and 7 are supported",
    );
    let magic = bytes
        .get(..4)
        .ok_or(FormatError("image file shorter than its 4-byte magic"))?;
    match magic {
        b"PB3B" => {
            let header = pb3::Header::parse(bytes)?;
            match header.kind {
                1 | 2 | 3 | 5 | 7 => Ok(Some(DecodedImage {
                    pixels: pb3::decode(bytes)?,
                    alpha: header.bpp == 32,
                })),
                6 | 8 => Err(FormatError(
                    "PB3 types 6/8 require a reference image source",
                )),
                4 => Err(unsupported),
                _ => Ok(None),
            }
        }
        [0x89, b'P', b'N', b'G'] | b"MSK0" | b"PB2A" | [b'B', b'M', ..] => Err(unsupported),
        _ => Ok(None),
    }
}

/// Contextual loading through the same registered source for all reference images.
/// Bounds are research limits: 16 active files and 128 MiB total raw/decoded bytes.
/// Each individual file and RGBA plane still has the 64 MiB payload limit.
pub fn load(
    source: &mut dyn ImageSource,
    name: &[u8],
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
    Ok(load_inner(source, name, &mut Vec::new(), &mut 0, false)?.map(|(image, _)| image))
}

/// A decoded image and its decoded-cache identity, when the source keeps that cache.
type Identified = (DecodedImage, Option<u128>);

/// A decode through the source's decoded-image cache. `identity` is `None` when the
/// source keeps no cache; failures and quiet `None` results are never stored.
fn decode_cached(
    source: &mut dyn ImageSource,
    identity: Option<u128>,
    decode: impl FnOnce() -> Result<Option<DecodedImage>, Box<dyn Error>>,
) -> Result<Option<Identified>, Box<dyn Error>> {
    if let Some(id) = identity
        && let Some(hit) = source.cached_decode(id)
    {
        return Ok(Some((hit, identity)));
    }
    let image = decode()?;
    if let (Some(id), Some(image)) = (identity, &image) {
        source.store_decode(id, image);
    }
    Ok(image.map(|image| (image, identity)))
}

fn charge(bytes: usize, budget: &mut usize) -> Result<(), Box<dyn Error>> {
    *budget = budget
        .checked_add(bytes)
        .ok_or("PB3 reference budget overflow")?;
    if *budget > crate::MAX_PAYLOAD * 2 {
        return Err("PB3 reference chain exceeds the byte budget".into());
    }
    Ok(())
}

fn load_inner(
    source: &mut dyn ImageSource,
    name: &[u8],
    chain: &mut Vec<Vec<u8>>,
    budget: &mut usize,
    reference_read: bool,
) -> Result<Option<Identified>, Box<dyn Error>> {
    let key = crate::cpz::lookup_key(name)?;
    if chain.len() >= 16 || chain.contains(&key) {
        return Err("PB3 reference cycle or depth limit exceeded".into());
    }
    if reference_read {
        source.cache_next_read()?;
    }
    chain.push(key);
    let read = source.read_image(name)?.ok_or_else(|| {
        format!(
            "image {} not found in any 0x2011 entry",
            name.escape_ascii()
        )
    })?;
    let result = (|| {
        charge(read.bytes.len(), budget)?;
        let identity = source
            .caches_decodes()
            .then(|| crate::raster::content_hash(&read.bytes));
        let decoded = if read.bytes.starts_with(b"PB3B") {
            let h = pb3::Header::parse(&read.bytes)?;
            charge(usize::from(h.width) * usize::from(h.height) * 4, budget)?;
            if let Some(reference) = pb3::reference_name(&read.bytes)? {
                let (base, base_identity) = load_inner(source, &reference, chain, budget, true)?
                    .ok_or("PB3 reference codec returned no image")?;
                // The overlay depends on its own bytes and on the whole base chain.
                let identity = identity.zip(base_identity).map(|(own, base)| {
                    let mut both = [0; 32];
                    both[..16].copy_from_slice(&own.to_le_bytes());
                    both[16..].copy_from_slice(&base.to_le_bytes());
                    crate::raster::content_hash(&both)
                });
                decode_cached(source, identity, || {
                    Ok(Some(DecodedImage {
                        pixels: pb3::decode_with_base(&read.bytes, base.pixels)?,
                        alpha: h.bpp == 32,
                    }))
                })?
            } else {
                decode_cached(source, identity, || Ok(decode(&read.bytes)?))?
            }
        } else {
            decode_cached(source, identity, || Ok(decode(&read.bytes)?))?
        };
        Ok(decoded)
    })();
    source.finish_image(read);
    chain.pop();
    result.map_err(|e: Box<dyn Error>| format!("image {}: {e}", name.escape_ascii()).into())
}

#[cfg(test)]
mod decoded_cache_tests {
    use super::*;

    fn image(value: u8, pixels: u32) -> DecodedImage {
        DecodedImage {
            pixels: RgbaImage::new(pixels, 1, vec![value; pixels as usize * 4]).unwrap(),
            alpha: false,
        }
    }

    #[test]
    fn least_recently_used_images_leave_first_and_hits_share_pixels() {
        let mut cache = DecodedCache {
            limit: 3 * 16,
            ..DecodedCache::default()
        };
        for id in 1..=3 {
            cache.insert(id, &image(id as u8, 4));
        }
        // Touch 1, so 2 is now the oldest.
        let hit = cache.get(1).unwrap();
        assert!(std::ptr::eq(hit.pixels.pixels(), cache.get(1).unwrap().pixels.pixels()));
        cache.insert(4, &image(4, 4));
        assert!(cache.get(2).is_none());
        assert!([1, 3, 4].iter().all(|&id| cache.get(id).is_some()));
        assert_eq!(cache.bytes, 3 * 16);
        // Larger than the whole limit: not stored, nothing evicted.
        cache.insert(5, &image(5, 13));
        assert!(cache.get(5).is_none());
        assert_eq!(cache.entries.len(), 3);
        // Lowering the limit evicts at once; zero disables the cache.
        cache.limit = 0;
        cache.evict_to(0);
        assert!(cache.entries.is_empty() && cache.bytes == 0);
        cache.insert(6, &image(6, 1));
        assert!(cache.get(6).is_none());
    }
}

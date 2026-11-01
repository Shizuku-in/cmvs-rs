//! Image-specific lookup and codec dispatch; shared I/O lives in `resources`.
use crate::{
    FormatError, pb3,
    raster::RgbaImage,
    resources::{RegisteredResources, ResourceArchives, ResourceKind},
};
use std::{collections::BTreeMap, error::Error, path::Path};
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
}
impl ImageArchives {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            archives: ResourceArchives::new(root)?,
            cache: Cache::default(),
        })
    }
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Registered<'a> {
        Registered {
            inner: self.archives.with_paths(paths, ResourceKind::Image),
            cache: &mut self.cache,
        }
    }
}
pub struct Registered<'a> {
    inner: RegisteredResources<'a>,
    cache: &'a mut Cache,
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
        "image codec not implemented; standalone PB3 types 1, 2, 3 and 5 are supported",
    );
    let magic = bytes
        .get(..4)
        .ok_or(FormatError("image file shorter than its 4-byte magic"))?;
    match magic {
        b"PB3B" => {
            let header = pb3::Header::parse(bytes)?;
            match header.kind {
                1 | 2 | 3 | 5 => Ok(Some(DecodedImage {
                    pixels: pb3::decode(bytes)?,
                    alpha: header.bpp == 32,
                })),
                6 => Err(FormatError("PB3 type 6 requires a reference image source")),
                4 | 7 | 8 => Err(unsupported),
                _ => Ok(None),
            }
        }
        [0x89, b'P', b'N', b'G'] | b"MSK0" | b"PB2A" | [b'B', b'M', ..] => Err(unsupported),
        _ => Ok(None),
    }
}

/// Contextual loading through the same registered source for all reference images.
/// Bounds are research limits: 16 active files and 64 MiB total raw/decoded bytes.
pub fn load(
    source: &mut dyn ImageSource,
    name: &[u8],
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
    load_inner(source, name, &mut Vec::new(), &mut 0, false)
}

fn charge(bytes: usize, budget: &mut usize) -> Result<(), Box<dyn Error>> {
    *budget = budget
        .checked_add(bytes)
        .ok_or("PB3 reference budget overflow")?;
    if *budget > crate::MAX_PAYLOAD {
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
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
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
        let decoded = if read.bytes.starts_with(b"PB3B") {
            let h = pb3::Header::parse(&read.bytes)?;
            charge(usize::from(h.width) * usize::from(h.height) * 4, budget)?;
            if let Some(reference) = pb3::reference_name(&read.bytes)? {
                let base = load_inner(source, &reference, chain, budget, true)?
                    .ok_or("PB3 reference codec returned no image")?;
                Some(DecodedImage {
                    pixels: pb3::decode_with_base(&read.bytes, base.pixels)?,
                    alpha: h.bpp == 32,
                })
            } else {
                decode(&read.bytes)?
            }
        } else {
            decode(&read.bytes)?
        };
        Ok(decoded)
    })();
    source.finish_image(read);
    chain.pop();
    result.map_err(|e: Box<dyn Error>| format!("image {}: {e}", name.escape_ascii()).into())
}

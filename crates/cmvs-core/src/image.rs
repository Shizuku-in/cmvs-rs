//! Image-specific lookup and codec dispatch; shared I/O lives in `resources`.
use crate::{
    FormatError,
    filesystem::HostFs,
    msk, pb2, pb3,
    raster::RgbaImage,
    resources::{RegisteredResources, ResourceArchives, ResourceKind},
};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    path::Path,
    sync::Arc,
};
mod cache;
use cache::Cache;
/// 0x2011 / 0x1400717c0: image manager has ten registration slots.
pub const IMAGE_ENTRIES: u32 = ResourceKind::Image.slot_count();
/// Threads an image decode may use besides the calling one, for formats with
/// independently coded parts. They live only for the decode. The default uses none.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeThreads {
    pub extra: usize,
    /// Runs first on each extra thread, with its index below `extra`: a host places
    /// the thread, e.g. on a core away from its audio.
    pub on_start: Option<fn(usize)>,
}
impl DecodeThreads {
    /// Uses up to three extra threads bounded by available CPU parallelism.
    ///
    /// Image decoders split at most four color planes across threads, so higher
    /// counts do not add parallel work.
    pub fn host() -> Self {
        let extra = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(1))
            .min(3);
        Self {
            extra,
            on_start: None,
        }
    }

    /// `run` over `tasks`, results in task order. Each thread takes the next
    /// unclaimed task; a thread the system refuses to start is simply not used.
    pub(crate) fn map<T: Sync, R: Send>(self, tasks: &[T], run: impl Fn(&T) -> R + Sync) -> Vec<R> {
        let extra = self.extra.min(tasks.len().saturating_sub(1));
        if extra == 0 {
            return tasks.iter().map(run).collect();
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let work = || {
            let mut done = Vec::new();
            loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(task) = tasks.get(index) else {
                    return done;
                };
                done.push((index, run(task)));
            }
        };
        let mut done = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..extra)
                .filter_map(|thread| {
                    std::thread::Builder::new()
                        .name("image decode".into())
                        .stack_size(256 << 10)
                        .spawn_scoped(scope, move || {
                            if let Some(on_start) = self.on_start {
                                on_start(thread);
                            }
                            work()
                        })
                        .ok()
                })
                .collect();
            let mut done = work();
            for worker in workers {
                match worker.join() {
                    Ok(results) => done.extend(results),
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
            done
        });
        done.sort_by_key(|(index, _)| *index);
        done.into_iter().map(|(_, result)| result).collect()
    }
}

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
    /// Host setting, not part of the original; results do not depend on it.
    fn decode_threads(&self) -> DecodeThreads {
        DecodeThreads::default()
    }
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
    threads: DecodeThreads,
}
impl ImageArchives {
    pub fn configure_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), crate::cpz::Cpz7Parameters>,
    ) {
        self.archives.configure_parameters(parameters);
    }
    pub fn with_fs(root: impl AsRef<Path>, fs: Arc<dyn HostFs>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            archives: ResourceArchives::with_fs(root, fs)?,
            cache: Cache::default(),
            decoded: DecodedCache::default(),
            threads: DecodeThreads::default(),
        })
    }

    #[cfg(feature = "std-fs")]
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Self::with_fs(root, crate::filesystem::default_fs())
    }
    /// Keeps up to `bytes` of raw compressed image payloads cached via `0x203f`
    /// (`32 MiB` by default; `0` disables raw caching).
    pub fn set_raw_cache_limit(&mut self, bytes: usize) {
        self.cache.set_limit(bytes);
    }
    /// Keeps up to `bytes` of decoded RGBA for images loaded again (0, the default,
    /// disables it). Results are unchanged; repeated loads skip decoding.
    pub fn set_decoded_cache_limit(&mut self, bytes: usize) {
        self.decoded.limit = bytes;
        self.decoded.evict_to(bytes);
    }
    /// Lets image decodes use extra threads. Results are unchanged.
    pub fn set_decode_threads(&mut self, threads: DecodeThreads) {
        self.threads = threads;
    }
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Registered<'a> {
        Registered {
            inner: self.archives.with_paths(paths, ResourceKind::Image),
            cache: &mut self.cache,
            decoded: &mut self.decoded,
            threads: self.threads,
        }
    }
}
pub struct Registered<'a> {
    inner: RegisteredResources<'a>,
    cache: &'a mut Cache,
    decoded: &'a mut DecodedCache,
    threads: DecodeThreads,
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
    fn decode_threads(&self) -> DecodeThreads {
        self.threads
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
    decode_with(bytes, DecodeThreads::default())
}

/// [`decode`] with extra threads for formats that can use them.
pub fn decode_with(
    bytes: &[u8],
    threads: DecodeThreads,
) -> Result<Option<DecodedImage>, FormatError> {
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
                    pixels: pb3::decode_with(bytes, threads)?,
                    alpha: header.bpp == 32,
                })),
                6 | 8 => Err(FormatError(
                    "PB3 types 6/8 require a reference image source",
                )),
                4 => Err(unsupported),
                _ => Ok(None),
            }
        }
        b"PB2A" => {
            let header = pb2::Header::parse(bytes)?;
            Ok(Some(DecodedImage {
                pixels: pb2::decode_with(bytes, threads)?,
                alpha: matches!(header.kind, 5 | 6) || header.bpp == 32,
            }))
        }
        b"MSK0" => Ok(Some(DecodedImage {
            pixels: msk::decode(bytes)?,
            alpha: false,
        })),
        [0x89, b'P', b'N', b'G'] | [b'B', b'M', ..] => Err(unsupported),
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

#[derive(Clone, Copy)]
enum Pb2Selector<'a> {
    Part(&'a [u8]),
    Frame(usize),
}

fn load_pb2_variant(
    source: &mut dyn ImageSource,
    name: &[u8],
    selector: Pb2Selector<'_>,
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
    let key = crate::cpz::lookup_key(name)?;
    let read = source.read_image(name)?.ok_or_else(|| {
        format!(
            "image {} not found in any 0x2011 entry",
            name.escape_ascii()
        )
    })?;
    if !read.bytes.starts_with(b"PB2A") {
        let mut chain = vec![key];
        let mut budget = 0;
        return Ok(
            load_from_read(source, name, read, &mut chain, &mut budget)?.map(|(image, _)| image)
        );
    }
    let header = match pb2::Header::parse(&read.bytes) {
        Ok(header) => header,
        Err(err) => {
            source.finish_image(read);
            return Err(Box::new(err));
        }
    };
    let matches_variant = match selector {
        Pb2Selector::Part(_) => header.kind == 6,
        Pb2Selector::Frame(_) => header.kind == 5,
    };
    if !matches_variant {
        let mut chain = vec![key];
        let mut budget = 0;
        return Ok(
            load_from_read(source, name, read, &mut chain, &mut budget)?.map(|(image, _)| image)
        );
    }
    let result = (|| -> Result<Option<Identified>, Box<dyn Error>> {
        let mut budget = 0;
        charge(read.bytes.len(), &mut budget)?;
        charge(
            usize::from(header.width) * usize::from(header.height) * 4,
            &mut budget,
        )?;
        let threads = source.decode_threads();
        let identity = source.caches_decodes().then(|| {
            let base = crate::raster::content_hash(&read.bytes);
            let mut mixed = Vec::with_capacity(24);
            mixed.extend_from_slice(&base.to_le_bytes());
            match selector {
                Pb2Selector::Part(part) => {
                    mixed.push(6);
                    mixed.extend_from_slice(part);
                }
                Pb2Selector::Frame(frame) => {
                    mixed.push(5);
                    mixed.extend_from_slice(&(frame as u64).to_le_bytes());
                }
            }
            crate::raster::content_hash(&mixed)
        });
        decode_cached(source, identity, || {
            let pixels = match selector {
                Pb2Selector::Part(part) => pb2::decode_part_with(&read.bytes, Some(part), threads)?,
                Pb2Selector::Frame(frame) => {
                    pb2::decode_frame_with_threads(&read.bytes, frame, threads)?
                }
            };
            Ok(Some(DecodedImage {
                pixels,
                alpha: matches!(header.kind, 5 | 6) || header.bpp == 32,
            }))
        })
    })();
    source.finish_image(read);
    Ok(result?.map(|(image, _)| image))
}

/// Decodes `base` selecting `part` when `base` is a PB2A type-6 archive, matching CVNS `0x2064`/`0x2065`.
pub fn load_part(
    source: &mut dyn ImageSource,
    base: &[u8],
    part: &[u8],
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
    load_pb2_variant(source, base, Pb2Selector::Part(part))
}

/// Decodes frame `frame` when `name` is a PB2A type-5 animation, matching CVNS `0x2052`.
pub fn load_frame(
    source: &mut dyn ImageSource,
    name: &[u8],
    frame: usize,
) -> Result<Option<DecodedImage>, Box<dyn Error>> {
    load_pb2_variant(source, name, Pb2Selector::Frame(frame))
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
        // The bytes and reference chain determine the pixels: no need to hash those.
        image.pixels.identify_content(id);
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
    let read = match source.read_image(name)? {
        Some(read) => read,
        None => {
            chain.pop();
            return Err(format!(
                "image {} not found in any 0x2011 entry",
                name.escape_ascii()
            )
            .into());
        }
    };
    load_from_read(source, name, read, chain, budget)
}

fn load_from_read(
    source: &mut dyn ImageSource,
    name: &[u8],
    read: ImageRead,
    chain: &mut Vec<Vec<u8>>,
    budget: &mut usize,
) -> Result<Option<Identified>, Box<dyn Error>> {
    let result = (|| {
        charge(read.bytes.len(), budget)?;
        let threads = source.decode_threads();
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
                decode_cached(source, identity, || Ok(decode_with(&read.bytes, threads)?))?
            }
        } else {
            decode_cached(source, identity, || Ok(decode_with(&read.bytes, threads)?))?
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
        assert!(std::ptr::eq(
            hit.pixels.pixels(),
            cache.get(1).unwrap().pixels.pixels()
        ));
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

    #[test]
    fn host_decode_threads_caps_extra_workers_at_three_planes() {
        let threads = DecodeThreads::host();
        assert!(threads.extra <= 3);
        assert!(threads.on_start.is_none());
        let output = threads.map(&[10, 20, 30, 40], |&n| n * 2);
        assert_eq!(output, vec![20, 40, 60, 80]);
    }

    #[derive(Default)]
    struct BackingFiles {
        files: BTreeMap<Vec<u8>, Vec<u8>>,
        disk_reads: usize,
    }

    impl crate::resources::ResourceSource for BackingFiles {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            self.disk_reads += 1;
            Ok(self.files.get(name).cloned())
        }
    }

    #[derive(Default)]
    struct TrackingImageSource {
        backing: BackingFiles,
        cache: Cache,
        read_calls: usize,
        read_image_calls: usize,
        finish_image_calls: usize,
    }

    impl TrackingImageSource {
        fn reset_call_counts(&mut self) {
            self.read_calls = 0;
            self.read_image_calls = 0;
            self.finish_image_calls = 0;
        }
    }

    impl ImageSource for TrackingImageSource {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            self.read_calls += 1;
            self.cache.read(name, &mut self.backing)
        }
        fn cache_next_read(&mut self) -> Result<(), Box<dyn Error>> {
            self.cache.arm();
            Ok(())
        }
        fn read_image(&mut self, name: &[u8]) -> Result<Option<ImageRead>, Box<dyn Error>> {
            self.read_image_calls += 1;
            self.cache.begin(name, &mut self.backing)
        }
        fn finish_image(&mut self, read: ImageRead) {
            self.finish_image_calls += 1;
            self.cache.finish(read);
        }
    }

    fn lzss_literals(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        (vec![0u8; bytes.len().div_ceil(8)], bytes.to_vec())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_pb2(
        kind: u16,
        width: u16,
        height: u16,
        bpp: u16,
        count: u32,
        diff_offset: u32,
        flags_offset: u32,
        data_offset: u32,
        body: &[u8],
    ) -> Vec<u8> {
        let mut raw = vec![0u8; 32];
        raw.extend_from_slice(body);
        if raw.len() < 33 {
            raw.resize(33, 0);
        }
        let mut tail = [0u8; 31];
        for (i, byte) in tail[4..28].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        let key = 0x5a3cu16;
        tail[28..30].copy_from_slice(&key.to_le_bytes());
        raw.extend_from_slice(&tail);

        let total = raw.len() as u32;
        raw[..4].copy_from_slice(b"PB2A");
        raw[4..8].copy_from_slice(&total.to_le_bytes());
        raw[8..12].copy_from_slice(&count.to_le_bytes());
        raw[12..16].copy_from_slice(&diff_offset.to_le_bytes());
        raw[16..18].copy_from_slice(&kind.to_le_bytes());
        raw[18..20].copy_from_slice(&width.to_le_bytes());
        raw[20..22].copy_from_slice(&height.to_le_bytes());
        raw[22..24].copy_from_slice(&bpp.to_le_bytes());
        raw[24..28].copy_from_slice(&flags_offset.to_le_bytes());
        raw[28..32].copy_from_slice(&data_offset.to_le_bytes());

        for (i, byte) in raw[8..32].iter_mut().enumerate() {
            *byte = byte.wrapping_add(tail[4 + i]);
        }
        for chunk in raw[8..32].as_chunks_mut::<2>().0 {
            let xored = u16::from_le_bytes([chunk[0], chunk[1]]) ^ key;
            chunk.copy_from_slice(&xored.to_le_bytes());
        }
        raw
    }

    fn xor_base_block(planes: [&[u8]; 4]) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        let mut stream = Vec::new();
        for (p, plane) in planes.iter().enumerate() {
            let (flags, data) = lzss_literals(plane);
            let f_off = stream.len() as u32;
            stream.extend_from_slice(&flags);
            let d_off = stream.len() as u32;
            stream.extend_from_slice(&data);
            out[p * 8..p * 8 + 4].copy_from_slice(&f_off.to_le_bytes());
            out[p * 8 + 4..p * 8 + 8].copy_from_slice(&d_off.to_le_bytes());
        }
        out.extend_from_slice(&stream);
        out
    }

    fn synthetic_msk() -> Vec<u8> {
        let mut raw = b"MSK0".to_vec();
        raw.extend(16u32.to_le_bytes());
        raw.extend(2u32.to_le_bytes());
        raw.extend(2u32.to_le_bytes());
        raw.extend_from_slice(&[10, 20, 30, 40]);
        raw
    }

    fn synthetic_pb2_type1() -> Vec<u8> {
        let unpacked = [10u8, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let (flags, data) = lzss_literals(&unpacked);
        let mut body = flags.clone();
        body.extend_from_slice(&data);
        build_pb2(1, 2, 2, 24, 1, 0, 32, 32 + flags.len() as u32, &body)
    }

    fn synthetic_pb2_type5() -> Vec<u8> {
        let mut body = xor_base_block([&[0x10; 64], &[0x03; 64], &[0x05; 64], &[0x0f; 64]]);
        let diff_off = 32 + body.len() as u32;
        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes());
        patch.extend(0u32.to_le_bytes());
        patch.push(0x00);
        patch.extend_from_slice(&[1u8, 2, 3].repeat(64));
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk_size = 16 + pflags.len() + pdata.len();
        body.extend((chunk_size as u32).to_le_bytes());
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);
        build_pb2(5, 8, 8, 32, 2, diff_off, 0, 0, &body)
    }

    fn synthetic_pb2_type6() -> Vec<u8> {
        let xor = xor_base_block([&[0x10; 18], &[0x03; 18], &[0x05; 18], &[0x0f; 18]]);
        let chunk0_size = 4 + 8 + xor.len();
        let mut body = Vec::new();
        body.extend((chunk0_size as u32).to_le_bytes());
        body.extend_from_slice(b"Base\0\0\0\0");
        body.extend_from_slice(&xor);

        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes());
        patch.extend(0u32.to_le_bytes());
        patch.push(0x80);
        patch.extend_from_slice(&[100, 110, 120, 200, 101, 111, 121, 201]);
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk1_size = 4 + 8 + 16 + pflags.len() + pdata.len();
        body.extend((chunk1_size as u32).to_le_bytes());
        body.extend_from_slice(b"Face_A\0\0");
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);
        build_pb2(6, 9, 2, 32, 2, 0, 0, 0, &body)
    }

    #[test]
    fn load_pb2_variant_reads_once_and_finishes_cache_on_fallback_and_pb2() {
        let mut source = TrackingImageSource::default();

        // 1. Non-PB2 image (MSK0) via load_part and cached reload via load_frame.
        source
            .backing
            .files
            .insert(b"mask.msk".to_vec(), synthetic_msk());
        source.cache_next_read().unwrap();
        let msk = load_part(&mut source, b"mask.msk", b"face")
            .unwrap()
            .unwrap();
        assert_eq!(&msk.pixels.pixels()[..4], &[10, 10, 10, 255]);
        assert_eq!(source.read_calls, 0);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 1);

        source.backing.files.clear();
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let msk_cached = load_frame(&mut source, b"mask.msk", 0).unwrap().unwrap();
        assert_eq!(msk_cached, msk);
        assert_eq!(source.read_calls, 0);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 1);

        // 2. PB2 non-matching variant fallback (type-1 via load_part / load_frame).
        source
            .backing
            .files
            .insert(b"still.pb2".to_vec(), synthetic_pb2_type1());
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let still = load_part(&mut source, b"still.pb2", b"part")
            .unwrap()
            .unwrap();
        assert_eq!(&still.pixels.pixels()[..4], &[90, 50, 10, 255]);
        assert_eq!(source.read_calls, 0);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 2);

        source.backing.files.clear();
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let still_cached = load_frame(&mut source, b"still.pb2", 1).unwrap().unwrap();
        assert_eq!(still_cached, still);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 2);

        // 3. PB2 type-6 matching variant via load_part + raw cache preservation.
        source
            .backing
            .files
            .insert(b"char.pb2".to_vec(), synthetic_pb2_type6());
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let part = load_part(&mut source, b"char.pb2", b"face_a")
            .unwrap()
            .unwrap();
        assert!(part.alpha);
        assert_eq!(
            &part.pixels.pixels()[8 * 4..8 * 4 + 4],
            &[120, 110, 100, 200]
        );
        assert_eq!(source.read_calls, 0);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 3);

        source.backing.files.clear();
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let part_cached = load_part(&mut source, b"char.pb2", b"face_a")
            .unwrap()
            .unwrap();
        assert_eq!(part_cached, part);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 3);

        // 4. PB2 type-5 matching variant via load_frame + raw cache preservation.
        source
            .backing
            .files
            .insert(b"anim.pb2".to_vec(), synthetic_pb2_type5());
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let frame1 = load_frame(&mut source, b"anim.pb2", 1).unwrap().unwrap();
        assert!(frame1.alpha);
        assert_eq!(&frame1.pixels.pixels()[..4], &[3, 2, 1, 255]);
        assert_eq!(source.read_calls, 0);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 4);

        source.backing.files.clear();
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        let frame1_cached = load_frame(&mut source, b"anim.pb2", 1).unwrap().unwrap();
        assert_eq!(frame1_cached, frame1);
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
        assert_eq!(source.backing.disk_reads, 4);

        // 5. Truncated PB2A header still finishes the read and caches the raw bytes.
        source
            .backing
            .files
            .insert(b"bad.pb2".to_vec(), b"PB2Ashort".to_vec());
        source.reset_call_counts();
        source.cache_next_read().unwrap();
        assert!(load_part(&mut source, b"bad.pb2", b"base").is_err());
        assert_eq!(source.read_image_calls, 1);
        assert_eq!(source.finish_image_calls, 1);
    }
}

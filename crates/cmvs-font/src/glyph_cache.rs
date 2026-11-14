//! Bounded LRU for rasterized glyphs, keyed by face and all pixel-affecting styles.
use cmvs_core::raster::RgbaImage;
use std::{collections::BTreeMap, sync::Arc};

type Key = (Arc<[u8]>, [u32; 9]);
// A full page may have 512 distinct visible glyph styles. A smaller LRU can miss
// on every glyph when that page is visited in the same order on successive frames.
const CAPACITY: usize = 512;
// 128 maximum-size 132x132 RGBA cells used about 8.51 MiB in the old cache.
// Preserve that working set while bounding large glyphs; draws can share evicted pixels.
const BYTE_LIMIT: usize = 9 * 1024 * 1024;

#[derive(Clone)]
struct Entry {
    image: RgbaImage,
    used: u64,
}

#[derive(Clone, Default)]
pub(super) struct GlyphCache {
    entries: BTreeMap<Key, Entry>,
    bytes: usize,
    clock: u64,
}

impl GlyphCache {
    pub fn get(&mut self, key: &Key) -> Option<&RgbaImage> {
        let entry = self.entries.get_mut(key)?;
        self.clock = self.clock.wrapping_add(1);
        entry.used = self.clock;
        Some(&entry.image)
    }

    pub fn insert(&mut self, key: Key, image: RgbaImage) {
        let bytes = image.pixels().len();
        if bytes > BYTE_LIMIT {
            return;
        }
        self.clock = self.clock.wrapping_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.image.pixels().len();
        }
        while self.entries.len() >= CAPACITY || self.bytes + bytes > BYTE_LIMIT {
            // Hits remain O(log n); scan only on a full-cache insertion, after
            // rasterization succeeds. Wrapping ages preserve order across rollover.
            let oldest = self
                .entries
                .iter()
                .max_by_key(|(_, entry)| self.clock.wrapping_sub(entry.used))
                .expect("full cache")
                .0
                .clone();
            let evicted = self.entries.remove(&oldest).expect("found above");
            self.bytes -= evicted.image.pixels().len();
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                image,
                used: self.clock,
            },
        );
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u32) -> Key {
        let mut style = [0; 9];
        style[0] = code;
        (Arc::from(&b"test face"[..]), style)
    }

    fn image(value: u8) -> RgbaImage {
        RgbaImage::new(1, 1, vec![value, 0, 0, 255]).unwrap()
    }

    #[test]
    fn hot_glyph_survives_streaming_misses_and_clock_rollover() {
        let mut cache = GlyphCache {
            clock: u64::MAX - 64,
            ..Default::default()
        };
        let hot = image(42);
        cache.insert(key(0), hot.clone());
        for i in 1..CAPACITY as u32 {
            cache.insert(key(i), image(i as u8));
        }
        assert!(cache.get(&key(0)).is_some());
        cache.insert(key(CAPACITY as u32), image(128));
        assert!(
            cache.get(&key(1)).is_none(),
            "evict the least recently used"
        );
        assert!(cache.get(&key(2)).is_some(), "do not clear the whole cache");
        for i in CAPACITY as u32 + 1..CAPACITY as u32 * 3 {
            let hit = cache.get(&key(0)).expect("hot glyph stays cached");
            assert!(std::ptr::eq(hit.pixels(), hot.pixels()));
            cache.insert(key(i), image(i as u8));
            assert_eq!(cache.len(), CAPACITY);
        }
        assert!(cache.get(&key(0)).is_some());
    }

    #[test]
    fn replacement_refreshes_recency_without_evicting_another_glyph() {
        let mut cache = GlyphCache::default();
        for i in 0..CAPACITY as u32 {
            cache.insert(key(i), image(i as u8));
        }
        let replacement = image(250);
        cache.insert(key(0), replacement.clone());
        assert_eq!(cache.len(), CAPACITY);
        assert_eq!(cache.entries[&key(0)].image, replacement);
        assert!(cache.entries.contains_key(&key(1)));
        cache.insert(key(CAPACITY as u32), image(128));
        assert!(cache.get(&key(1)).is_none());
        assert_eq!(cache.get(&key(0)), Some(&replacement));
    }

    #[test]
    fn cloned_caches_have_independent_recency() {
        let mut original = GlyphCache::default();
        for i in 0..CAPACITY as u32 {
            original.insert(key(i), image(i as u8));
        }
        let mut cloned = original.clone();
        original.get(&key(0));
        original.insert(key(CAPACITY as u32), image(128));
        cloned.insert(key(CAPACITY as u32), image(128));
        assert!(original.get(&key(0)).is_some());
        assert!(original.get(&key(1)).is_none());
        assert!(cloned.get(&key(0)).is_none());
        assert!(cloned.get(&key(1)).is_some());
    }

    fn large_glyph(value: u8) -> RgbaImage {
        // The largest supported glyph: 128px plus two padding pixels on each side.
        RgbaImage::new(132, 132, vec![value; 132 * 132 * 4]).unwrap()
    }

    #[test]
    fn byte_budget_evicts_the_oldest_large_glyph_and_keeps_shared_pixels_valid() {
        let mut cache = GlyphCache::default();
        let first = large_glyph(0);
        let count = BYTE_LIMIT / first.pixels().len();
        assert!(count < CAPACITY, "large glyphs reach the byte limit first");
        for i in 0..count {
            cache.insert(key(i as u32), large_glyph(i as u8));
        }
        // Retain an evicted glyph as a renderer would, and touch a different glyph.
        let retained = cache.entries[&key(1)].image.clone();
        let hot = cache.get(&key(0)).unwrap().clone();
        cache.insert(key(count as u32), first.clone());
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some());
        assert!(cache.get(&key(count as u32)).is_some());
        assert!(std::ptr::eq(
            cache.get(&key(0)).unwrap().pixels(),
            hot.pixels()
        ));
        assert!(retained.pixels().iter().all(|&v| v == 1));
        assert_eq!(cache.len(), count);
        assert_eq!(cache.bytes, count * first.pixels().len());
        assert!(cache.bytes <= BYTE_LIMIT);
    }

    #[test]
    fn replacements_release_their_previous_byte_charge() {
        let mut cache = GlyphCache::default();
        let large = large_glyph(1);
        let count = BYTE_LIMIT / large.pixels().len();
        for i in 0..count {
            cache.insert(key(i as u32), large.clone());
        }
        let small = image(2);
        cache.insert(key(0), small.clone());
        assert_eq!(cache.bytes, (count - 1) * large.pixels().len() + 4);
        // Shrinking one entry leaves room for another without evicting its neighbor.
        cache.insert(key(count as u32), large.clone());
        assert_eq!(cache.len(), count + 1);
        assert!(cache.entries.contains_key(&key(1)));
        assert_eq!(cache.get(&key(0)), Some(&small));
        // Growing it again must evict the oldest neighbor and keep the new value.
        cache.insert(key(0), large.clone());
        assert_eq!(cache.len(), count);
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some());
        assert_eq!(cache.get(&key(0)), Some(&large));
        assert_eq!(cache.bytes, count * large.pixels().len());
    }

    #[test]
    fn oversized_entries_bypass_the_cache_without_evicting_hot_glyphs() {
        let mut cache = GlyphCache::default();
        let hot = image(42);
        cache.insert(key(0), hot.clone());
        let oversized =
            RgbaImage::new(BYTE_LIMIT as u32 / 4 + 1, 1, vec![0; BYTE_LIMIT + 4]).unwrap();
        cache.insert(key(1), oversized);
        assert!(cache.get(&key(1)).is_none());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.bytes, 4);
        assert!(std::ptr::eq(
            cache.get(&key(0)).unwrap().pixels(),
            hot.pixels()
        ));
    }
}

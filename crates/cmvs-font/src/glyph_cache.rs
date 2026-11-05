//! Bounded LRU for rasterized glyphs, keyed by face and all pixel-affecting styles.
use cmvs_core::raster::RgbaImage;
use std::{collections::BTreeMap, sync::Arc};

type Key = (Arc<[u8]>, [u32; 9]);
const CAPACITY: usize = 128;

#[derive(Clone)]
struct Entry {
    image: RgbaImage,
    used: u64,
}

#[derive(Clone, Default)]
pub(super) struct GlyphCache {
    entries: BTreeMap<Key, Entry>,
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
        self.clock = self.clock.wrapping_add(1);
        if self.entries.len() == CAPACITY && !self.entries.contains_key(&key) {
            // Hits remain O(log n); scan only on a full-cache insertion, after
            // rasterization succeeds. Wrapping ages preserve order across rollover.
            let oldest = self
                .entries
                .iter()
                .max_by_key(|(_, entry)| self.clock.wrapping_sub(entry.used))
                .expect("full cache")
                .0
                .clone();
            self.entries.remove(&oldest);
        }
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
        cache.insert(key(128), image(128));
        assert!(
            cache.get(&key(1)).is_none(),
            "evict the least recently used"
        );
        assert!(cache.get(&key(2)).is_some(), "do not clear the whole cache");
        for i in 129..512 {
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
        cache.insert(key(128), image(128));
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
        original.insert(key(128), image(128));
        cloned.insert(key(128), image(128));
        assert!(original.get(&key(0)).is_some());
        assert!(original.get(&key(1)).is_none());
        assert!(cloned.get(&key(0)).is_none());
        assert!(cloned.get(&key(1)).is_some());
    }
}

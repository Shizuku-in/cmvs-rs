//! Raw image bytes: next-read hint (0x140035a90/0x140036820) and LRU
//! (0x140037370/0x140037470). This is separate from CPZ index and decoded-image storage.
use super::ImageRead;
use crate::{cpz::lookup_key, resources::ResourceSource};
use std::{collections::VecDeque, error::Error};

pub(super) struct Cache {
    armed: bool,
    entries: VecDeque<(Vec<u8>, Vec<u8>)>,
    bytes: usize,
    limit: usize,
    max_entries: usize,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            armed: false,
            entries: VecDeque::new(),
            bytes: 0,
            // 0x140037230 constructor; configurable original startup limits are not modeled.
            limit: 32 * 1024 * 1024,
            // Additional research bound, including zero-length file entries.
            max_entries: 1024,
        }
    }
}
impl Cache {
    pub(super) fn arm(&mut self) {
        self.armed = true;
    }
    pub(super) fn read(
        &mut self,
        name: &[u8],
        source: &mut impl ResourceSource,
    ) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let Some(read) = self.begin(name, source)? else {
            return Ok(None);
        };
        if read.cache_key.is_none() {
            return Ok(Some(read.bytes));
        }
        let bytes = read.bytes.clone();
        self.finish(read);
        Ok(Some(bytes))
    }
    pub(super) fn begin(
        &mut self,
        name: &[u8],
        source: &mut impl ResourceSource,
    ) -> Result<Option<ImageRead>, Box<dyn Error>> {
        // The original resets +24792 before lookup, including missing/failed reads.
        if !std::mem::take(&mut self.armed) {
            return Ok(source.read(name)?.map(|bytes| ImageRead {
                bytes,
                cache_key: None,
            }));
        }
        let key = lookup_key(name)?;
        if let Some(index) = self.entries.iter().position(|(name, _)| *name == key) {
            let entry = self.entries.remove(index).expect("found above");
            let result = entry.1.clone();
            self.entries.push_front(entry);
            return Ok(Some(ImageRead {
                bytes: result,
                cache_key: None,
            }));
        }
        let Some(bytes) = source.read(name)? else {
            return Ok(None);
        };
        Ok(Some(ImageRead {
            bytes,
            cache_key: Some(key),
        }))
    }
    pub(super) fn finish(&mut self, read: ImageRead) {
        let ImageRead {
            bytes,
            cache_key: Some(key),
        } = read
        else {
            return;
        };
        // Recursive reads may have inserted the same key in the meantime.
        if let Some(index) = self.entries.iter().position(|(name, _)| *name == key) {
            self.bytes -= self.entries.remove(index).expect("found above").1.len();
        }
        // An oversized insertion evicts all entries, including itself, in the original.
        // Keep ownership of the returned bytes and never reproduce a freed buffer access.
        if bytes.len() > self.limit {
            self.entries.clear();
            self.bytes = 0;
        } else {
            self.bytes += bytes.len();
            self.entries.push_front((key, bytes));
            while self.bytes > self.limit || self.entries.len() > self.max_entries {
                self.bytes -= self.entries.pop_back().expect("over limit").1.len();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Files(BTreeMap<Vec<u8>, Vec<u8>>);
    impl ResourceSource for Files {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            Ok(self.0.get(name).cloned())
        }
    }
    fn read(cache: &mut Cache, source: &mut Files, name: &[u8]) -> Option<Vec<u8>> {
        cache.arm();
        cache.read(name, source).unwrap()
    }
    #[test]
    fn byte_limit_evicts_least_recently_used_after_hits_promote() {
        let mut cache = Cache {
            limit: 4,
            ..Cache::default()
        };
        let mut source = Files::default();
        for name in [b"a", b"b", b"c"] {
            source.0.insert(name.to_vec(), vec![name[0]; 2]);
        }
        read(&mut cache, &mut source, b"a");
        read(&mut cache, &mut source, b"b");
        source.0.clear();
        assert_eq!(read(&mut cache, &mut source, b"A"), Some(b"aa".to_vec()));
        source.0.insert(b"c".to_vec(), b"cc".to_vec());
        read(&mut cache, &mut source, b"c");
        assert_eq!(read(&mut cache, &mut source, b"b"), None);
        assert_eq!(read(&mut cache, &mut source, b"a"), Some(b"aa".to_vec()));
        assert_eq!(cache.bytes, 4);
    }
    #[test]
    fn oversized_file_clears_cache_without_losing_returned_bytes() {
        let mut cache = Cache {
            limit: 2,
            ..Cache::default()
        };
        let mut source = Files::default();
        source.0.insert(b"a".to_vec(), vec![1]);
        source.0.insert(b"big".to_vec(), vec![2; 3]);
        read(&mut cache, &mut source, b"a");
        assert_eq!(read(&mut cache, &mut source, b"big"), Some(vec![2; 3]));
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }
    #[test]
    fn metadata_is_bounded_and_shift_jis_trail_bytes_are_not_case_folded() {
        let mut cache = Cache {
            max_entries: 2,
            ..Cache::default()
        };
        let mut source = Files::default();
        for name in [b"a".as_slice(), b"\x81A", b"\x81a"] {
            source.0.insert(name.to_vec(), vec![]);
            read(&mut cache, &mut source, name);
        }
        assert_eq!(cache.entries.len(), 2);
        source.0.clear();
        assert_eq!(read(&mut cache, &mut source, b"a"), None);
        assert_eq!(read(&mut cache, &mut source, b"\x81A"), Some(vec![]));
        assert_eq!(read(&mut cache, &mut source, b"\x81a"), Some(vec![]));
    }
    #[test]
    fn decode_reads_insert_in_recursive_completion_order() {
        let mut cache = Cache {
            max_entries: 2,
            ..Cache::default()
        };
        let mut source = Files::default();
        for name in [b"outer", b"inner"] {
            source.0.insert(name.to_vec(), vec![1]);
        }
        cache.arm();
        let outer = cache.begin(b"outer", &mut source).unwrap().unwrap();
        cache.arm();
        let inner = cache.begin(b"inner", &mut source).unwrap().unwrap();
        assert!(cache.entries.is_empty());
        cache.finish(inner);
        cache.finish(outer);
        assert_eq!(cache.entries.front().unwrap().0, b"outer");
        source.0.insert(b"third".to_vec(), vec![3]);
        read(&mut cache, &mut source, b"third");
        source.0.clear();
        assert_eq!(read(&mut cache, &mut source, b"inner"), None);
        assert_eq!(read(&mut cache, &mut source, b"outer"), Some(vec![1]));
    }
}

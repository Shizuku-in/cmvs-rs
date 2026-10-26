use cmvs_rs::resources::{ResourceArchives, ResourceKind, ResourceSource};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cmvs-resources-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for dir in ["a", "b", "c"] {
            fs::create_dir(root.join(dir)).unwrap();
        }
        Self(root)
    }
    fn put(&self, path: &str, bytes: &[u8]) {
        fs::write(self.0.join(path), bytes).unwrap();
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        for dir in ["a", "b", "c"] {
            for name in ["same.bin", "later.bin"] {
                let _ = fs::remove_file(self.0.join(dir).join(name));
            }
            let _ = fs::remove_dir(self.0.join(dir));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
#[test]
fn resource_families_and_slot_limits_are_independent() {
    let root = Root::new();
    root.put("a/same.bin", b"image");
    root.put("b/same.bin", b"SE");
    root.put("c/same.bin", b"BGM");
    root.put("c/later.bin", b"outside SE slots");
    let paths = BTreeMap::from([
        ((0x2011, 9), b"a\\".to_vec()),
        ((0x2012, 3), b"b\\".to_vec()),
        ((0x2012, 4), b"c\\".to_vec()),
        ((0x2013, 3), b"c\\".to_vec()),
    ]);
    let mut archives = ResourceArchives::new(&root.0).unwrap();
    for (kind, expected) in [
        (ResourceKind::Image, b"image".as_slice()),
        (ResourceKind::SoundEffect, b"SE"),
        (ResourceKind::Music, b"BGM"),
    ] {
        assert_eq!(
            archives
                .with_paths(&paths, kind)
                .read(b"same.bin")
                .unwrap()
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::SoundEffect)
            .read(b"later.bin")
            .unwrap(),
        None
    );
}
#[test]
fn audio_roots_search_in_order_and_follow_reregistration() {
    let root = Root::new();
    root.put("a/same.bin", b"first");
    root.put("b/same.bin", b"second");
    root.put("b/later.bin", b"fallback");
    let mut paths = BTreeMap::from([
        ((0x2012, 0), b"a\\".to_vec()),
        ((0x2012, 1), b"b\\".to_vec()),
    ]);
    let mut archives = ResourceArchives::new(&root.0).unwrap();
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::SoundEffect)
            .read(b"same.bin")
            .unwrap()
            .unwrap(),
        b"first"
    );
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::SoundEffect)
            .read(b"later.bin")
            .unwrap()
            .unwrap(),
        b"fallback"
    );
    paths.insert((0x2012, 0), b"c\\".to_vec());
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::SoundEffect)
            .read(b"same.bin")
            .unwrap()
            .unwrap(),
        b"second"
    );
    paths.insert((0x2012, 0), b"missing.cpz".to_vec());
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::SoundEffect)
            .read(b"same.bin")
            .unwrap()
            .unwrap(),
        b"second"
    );
}
#[test]
fn shared_reader_confines_audio_paths_and_rejects_file_roots() {
    let root = Root::new();
    root.put("a/same.bin", b"valid");
    assert!(ResourceArchives::new(root.0.join("a/same.bin")).is_err());
    let paths = BTreeMap::from([((0x2013, 0), b"a\\".to_vec())]);
    let mut archives = ResourceArchives::new(&root.0).unwrap();
    assert!(
        archives
            .with_paths(&paths, ResourceKind::Music)
            .read(b"..\\..\\outside.bin")
            .is_err()
    );
    assert_eq!(
        archives
            .with_paths(&paths, ResourceKind::Music)
            .read(b"absent.bin")
            .unwrap(),
        None
    );
}

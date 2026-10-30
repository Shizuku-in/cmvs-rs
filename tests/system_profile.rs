use cmvs_rs::system_save::{Css1Save, ProfileSystemSave, SystemSaveSink};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn configuration_uses_the_profile_lock_atomic_replacement_and_own_backup() {
    use cmvs_rs::settings::Configuration;
    let root = Root::new();
    fs::write(root.game().join("cmvs.cfg"), b"original read only").unwrap();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    assert!(profile.read_configuration().unwrap().is_none());
    let first = Configuration::new(b"[S]\nX=1\n".to_vec()).unwrap();
    let second = Configuration::new(b"[S]\nX=2\n".to_vec()).unwrap();
    profile.write_configuration(&first).unwrap();
    profile.write_configuration(&second).unwrap();
    assert_eq!(profile.read_configuration().unwrap(), Some(second));
    assert_eq!(
        fs::read(root.profile().join("cmvs.cfg.bak")).unwrap(),
        first.bytes()
    );
    assert_eq!(
        fs::read(root.game().join("cmvs.cfg")).unwrap(),
        b"original read only"
    );
    drop(profile);
    assert!(
        ProfileSystemSave::open(&root.profile(), &root.game())
            .unwrap()
            .read_configuration()
            .unwrap()
            .is_some()
    );
}

#[test]
fn configuration_rejects_invalid_targets_and_ambiguous_missing_primary() {
    use cmvs_rs::settings::Configuration;
    let root = Root::new();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    fs::create_dir(root.profile().join("cmvs.cfg.bak")).unwrap();
    assert!(
        profile
            .write_configuration(&Configuration::new(vec![]).unwrap())
            .is_err()
    );
    fs::remove_dir(root.profile().join("cmvs.cfg.bak")).unwrap();
    fs::write(root.profile().join("cmvs.cfg.bak"), b"x").unwrap();
    assert!(profile.read_configuration().is_err());
    assert!(!root.profile().join("cmvs.cfg").exists());
}

#[cfg(windows)]
#[test]
fn failed_configuration_replacement_preserves_primary_and_backup_for_retry() {
    use cmvs_rs::settings::Configuration;
    use std::os::windows::fs::OpenOptionsExt;
    let root = Root::new();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    let first = Configuration::new(b"[S]\nX=1\n".to_vec()).unwrap();
    let second = Configuration::new(b"[S]\nX=2\n".to_vec()).unwrap();
    profile.write_configuration(&first).unwrap();
    let blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(root.profile().join("cmvs.cfg"))
        .unwrap();
    assert!(profile.write_configuration(&second).is_err());
    assert_eq!(profile.read_configuration().unwrap(), Some(first.clone()));
    assert_eq!(
        fs::read(root.profile().join("cmvs.cfg.bak")).unwrap(),
        first.bytes()
    );
    drop(blocker);
    profile.write_configuration(&second).unwrap();
    assert_eq!(profile.read_configuration().unwrap(), Some(second));
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        // The managed Windows sandbox's virtual Temp denies rename. Use an owned
        // workspace fixture to exercise the actual same-directory rename operation.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "cmvs-profile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("game")).unwrap();
        Self(root)
    }
    fn game(&self) -> PathBuf {
        self.0.join("game")
    }
    fn profile(&self) -> PathBuf {
        self.0.join("nested/profile")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        // Only the uniquely created synthetic test directory may be removed.
        let target = self.0.canonicalize().unwrap();
        assert_eq!(
            target.parent().unwrap(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .canonicalize()
                .unwrap()
        );
        assert!(
            target
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("cmvs-profile-")
        );
        fs::remove_dir_all(target).unwrap();
    }
}
fn fixture(value: i32) -> Vec<u8> {
    let mut payload = vec![0u8; 44864];
    payload[32512..32516].copy_from_slice(&value.to_le_bytes());
    let mut packed = Vec::new();
    for bytes in payload.chunks(8) {
        packed.push(((1u16 << bytes.len()) - 1) as u8);
        packed.extend(bytes);
    }
    let mut raw = vec![0u8; 360];
    raw[..4].copy_from_slice(b"CSS1");
    for (at, value) in [
        (4, 104u32),
        (68, 0x12345678),
        (92, payload.len() as u32),
        (96, packed.len() as u32),
        (100, 360 + packed.len() as u32),
    ] {
        raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    raw[76] = 1;
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, byte) in packed.into_iter().enumerate() {
        raw.push((byte.wrapping_add(64) ^ key[i % key.len()]).rotate_left(2));
    }
    let sum = raw[360..]
        .iter()
        .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
    raw.extend(sum.to_le_bytes());
    raw
}

#[test]
fn repeated_saves_back_up_previous_committed_generation_and_release_session_lock() {
    let root = Root::new();
    let original = fixture(0);
    let original_path = root.game().join("system.dat");
    fs::write(&original_path, &original).unwrap();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    assert!(ProfileSystemSave::open(&root.profile(), &root.game()).is_err());
    for value in 1..=3 {
        profile.write(&fixture(value)).unwrap();
        assert_eq!(fs::read(profile.path()).unwrap(), fixture(value));
        if value == 1 {
            assert!(!profile.backup_path().exists());
        } else {
            assert_eq!(fs::read(profile.backup_path()).unwrap(), fixture(value - 1));
        }
    }
    assert_eq!(fs::read(original_path).unwrap(), original);
    assert!(fs::read_dir(profile.directory()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    drop(profile);
    let reopened = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    assert_eq!(fs::read(reopened.path()).unwrap(), fixture(3));
}

#[test]
fn failed_backup_or_bad_data_keeps_primary_and_allows_retry() {
    let root = Root::new();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    profile.write(&fixture(1)).unwrap();
    fs::create_dir(profile.backup_path()).unwrap();
    assert!(profile.write(&fixture(2)).is_err());
    assert_eq!(fs::read(profile.path()).unwrap(), fixture(1));
    fs::remove_dir(profile.backup_path()).unwrap();
    assert!(profile.write(b"bad CSS1").is_err());
    let mut other_game = fixture(2);
    other_game[68..72].copy_from_slice(&99u32.to_le_bytes());
    assert!(profile.write(&other_game).is_err());
    assert_eq!(fs::read(profile.path()).unwrap(), fixture(1));
    profile.write(&fixture(2)).unwrap();
    assert_eq!(fs::read(profile.backup_path()).unwrap(), fixture(1));
    assert!(fs::read_dir(profile.directory()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn corrupted_primary_does_not_destroy_valid_backup_or_silently_reset() {
    let root = Root::new();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    profile.write(&fixture(1)).unwrap();
    profile.write(&fixture(2)).unwrap();
    fs::write(profile.path(), b"corrupted").unwrap();
    assert!(profile.write(&fixture(3)).is_err());
    assert_eq!(fs::read(profile.path()).unwrap(), b"corrupted");
    assert_eq!(fs::read(profile.backup_path()).unwrap(), fixture(1));
    fs::remove_file(profile.path()).unwrap();
    drop(profile);
    assert!(ProfileSystemSave::open(&root.profile(), &root.game()).is_err());
    // Explicit recovery; no implicit backup selection or replacement of a corrupt save.
    fs::copy(
        root.profile().join("system.bak"),
        root.profile().join("system.dat"),
    )
    .unwrap();
    let profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    assert_eq!(
        Css1Save::decode(&fs::read(profile.path()).unwrap())
            .unwrap()
            .payload()[32512],
        1
    );
}

#[test]
fn game_directories_and_non_file_entries_are_rejected_before_writing() {
    let root = Root::new();
    let forbidden = root.game().join("new/profile");
    assert!(ProfileSystemSave::open(&forbidden, &root.game()).is_err());
    assert!(!root.game().join("new").exists());
    fs::create_dir_all(root.profile()).unwrap();
    fs::create_dir(root.profile().join("system.dat")).unwrap();
    assert!(ProfileSystemSave::open(&root.profile(), &root.game()).is_err());
    fs::remove_dir(root.profile().join("system.dat")).unwrap();
    fs::write(
        root.profile().join(".cmvs-system.lock"),
        b"not an empty lock",
    )
    .unwrap();
    assert!(ProfileSystemSave::open(&root.profile(), &root.game()).is_err());
}

#[cfg(windows)]
#[test]
fn failed_primary_replacement_preserves_old_file_and_synced_backup() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = Root::new();
    let mut profile = ProfileSystemSave::open(&root.profile(), &root.game()).unwrap();
    profile.write(&fixture(1)).unwrap();
    // Allow reading/writing but deny DELETE sharing: backing up can succeed while
    // replacing the still-open primary fails. No synthetic fault in the implementation.
    let blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(profile.path())
        .unwrap();
    assert!(profile.write(&fixture(2)).is_err());
    assert_eq!(fs::read(profile.path()).unwrap(), fixture(1));
    assert_eq!(fs::read(profile.backup_path()).unwrap(), fixture(1));
    drop(blocker);
    profile.write(&fixture(2)).unwrap();
    assert_eq!(fs::read(profile.path()).unwrap(), fixture(2));
    assert_eq!(fs::read(profile.backup_path()).unwrap(), fixture(1));
}

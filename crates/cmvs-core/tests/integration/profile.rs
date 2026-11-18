//! Runtime identity selection and SHA-256 checks.
use cmvs_core::{
    format::digest::{Sha256, parse_hex, sha256, sha256_reader, to_hex},
    profile::{Evidence, FileEvidence, Registry, SelectionError},
};

#[test]
fn sha256_matches_fips_180_4_examples_incrementally_and_from_readers() {
    let cases: [(&[u8], &str); 3] = [
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(to_hex(&sha256(input)), expected);
        assert_eq!(parse_hex(expected), Some(sha256(input)));
    }
    let million = vec![b'a'; 1_000_000];
    let expected = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";
    assert_eq!(to_hex(&sha256(&million)), expected);
    assert_eq!(to_hex(&sha256_reader(&million[..]).unwrap()), expected);
    let mut split = Sha256::new();
    for chunk in million.chunks(997) {
        split.update(chunk);
    }
    assert_eq!(to_hex(&split.finish()), expected);
    assert_eq!(parse_hex("00"), None);
    assert_eq!(parse_hex(&"g".repeat(64)), None);
}

#[test]
fn both_amatsutsumi_startups_select_the_existing_save_identity() {
    let registry = Registry::builtin();
    let archive = FileEvidence {
        path: "script.cpz".into(),
        sha256: parse_hex("92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4")
            .unwrap(),
    };
    for start in [
        "c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0",
        "14a5f200dc0c8dfc00cbc1c3293d09849fd4da060ecdc1d6cbced031691a4986",
    ] {
        let evidence = Evidence {
            start_script: Some(FileEvidence {
                path: "start.ps3".into(),
                sha256: parse_hex(start).unwrap(),
            }),
            script_archive: Some(archive.clone()),
            executables: Vec::new(),
        };
        let selection = registry.select(None, &evidence).unwrap();
        let game = selection.record.unwrap();
        assert_eq!(game.game, "amatsutsumi");
        assert_eq!(game.game_id, 0x1780_0016);
        assert_eq!(game.skip_auto_resume, Some(2198));
        assert_eq!(selection.profile.id, "cmvs-20160201");
    }
}

#[test]
fn registered_game_aliases_select_the_same_profile_and_record_as_its_id() {
    let registry = Registry::builtin();
    for game in registry.games() {
        let evidence = Evidence {
            script_archive: Some(FileEvidence {
                path: game.script_archive.path.clone(),
                sha256: game.script_archive.sha256,
            }),
            ..Evidence::default()
        };
        let selected = registry.select(Some(&game.game), &evidence).unwrap();
        assert_eq!(selected.record, Some(game));
        assert_eq!(
            selected,
            registry.select(Some(&game.profile), &evidence).unwrap()
        );
    }
}

#[test]
fn game_files_are_found_by_fixed_names_only() {
    use cmvs_core::profile::GameFiles;
    let root = crate::support::temp_dir::TempDir::new("profile-files");
    std::fs::create_dir_all(root.join("data/pack")).unwrap();
    assert!(matches!(
        GameFiles::discover(&root),
        Err(SelectionError::MissingFile(_))
    ));
    std::fs::write(root.join("data/pack/start.ps2"), b"PS2A").unwrap();
    assert!(GameFiles::discover(&root).is_err());
    std::fs::write(root.join("data/pack/script.cpz"), b"CPZ6").unwrap();
    let files = GameFiles::discover(&root).unwrap();
    assert_eq!(files.start_name(), b"start.ps2");
    std::fs::write(root.join("data/pack/start.ps3"), b"PS2A").unwrap();
    assert!(matches!(
        GameFiles::discover(&root),
        Err(SelectionError::AmbiguousStartScript)
    ));
    for name in ["start.ps2", "start.ps3", "script.cpz"] {
        std::fs::remove_file(root.join("data/pack").join(name)).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

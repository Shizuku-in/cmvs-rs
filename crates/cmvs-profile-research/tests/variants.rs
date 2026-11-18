//! Research-only validation of executable-bound semantic overrides.
use cmvs_core::{
    format::digest::to_hex,
    profile::{CommandStatus, Semantics, VariantId},
    vm::dispatch,
};
use cmvs_profile_research::ResearchRegistry as Registry;

const ROOT: &str = include_str!("../profiles/cmvs-20160201.profile");
const DERIVED: &str = include_str!("../profiles/cmvs-20171114.profile");
const HASH: &str = "3bccda5d7611412c6938c0c9b06b2195914cfb78763a53f2c235bb6479395ee0";

#[test]
fn declared_variant_round_trips_and_does_not_depend_on_the_profile_name() {
    let renamed = DERIVED.replacen("profile cmvs-20171114", "profile synthetic-variant", 1);
    let registry = Registry::parse(&[ROOT, &renamed], "").unwrap();
    let profile = registry.profile("synthetic-variant").unwrap();
    assert_eq!(profile.variant_bindings().count(), 19);
    for (command, entry) in profile.variant_bindings() {
        assert_eq!(profile.status(command), CommandStatus::Unreviewed);
        let expected = if command == 0x2a5 {
            VariantId::Particles
        } else if matches!(command, 0x2a6 | 0x330..=0x332) {
            VariantId::CollisionParticles
        } else if command == 0x2a8 {
            VariantId::Fragments
        } else if command == 0x2a2 {
            VariantId::FallingBillboards
        } else if command == 0x2ed {
            VariantId::BackgroundAudio
        } else {
            VariantId::FontCells
        };
        assert_eq!(entry.id, expected);
        assert_eq!(profile.semantics(command), Semantics::Variant(expected));
        assert_eq!(to_hex(&entry.binary_sha256), HASH);
        assert!(dispatch::variant_entry(entry.id, command).is_some());
        assert!(
            profile
                .binaries
                .iter()
                .any(|b| b.arch == entry.arch && b.sha256 == entry.binary_sha256)
        );
    }
    let canonical = profile.to_text();
    let reparsed = Registry::parse(&[ROOT, &canonical], "").unwrap();
    assert_eq!(reparsed.profile("synthetic-variant"), Some(profile));
    let stripped = renamed
        .lines()
        .map(|line| line.split(" variant=").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let stripped = stripped.replace("profile synthetic-variant", "profile without-variant");
    let registry = Registry::parse(&[ROOT, &renamed, &stripped], "").unwrap();
    assert_eq!(
        registry
            .profile("synthetic-variant")
            .unwrap()
            .semantics(0x3ac),
        Semantics::Variant(VariantId::FontCells)
    );
    assert_eq!(
        registry.profile("cmvs-20160201").unwrap().semantics(0x3ac),
        Semantics::Root
    );
    assert_eq!(
        registry
            .profile("without-variant")
            .unwrap()
            .semantics(0x3ac),
        Semantics::Root
    );
}

#[test]
fn variant_declarations_reject_unknown_handlers_bad_bindings_and_unsafe_evidence_paths() {
    let declaration = format!("variant=font-cells:x64:{HASH} evidence=games/aoitori/README.md");
    for replacement in [
        declaration.replace("variant=", "research="),
        declaration.replace("font-cells", "unknown"),
        declaration.replace("x64", "unknown"),
        declaration.replace(HASH, &"0".repeat(64)),
        declaration.replace(HASH, "not-a-hash"),
        declaration.replace(&format!(":{HASH}"), ""),
        declaration.replace("games/aoitori/README.md", "../outside.md"),
        declaration.replace("games/aoitori/README.md", "/outside.md"),
        declaration.replace("games/aoitori/README.md", "games/evidence.txt"),
        declaration.replace(" evidence=games/aoitori/README.md", ""),
        format!("{declaration} evidence=duplicate.md"),
    ] {
        assert!(
            Registry::parse(&[ROOT, &DERIVED.replace(&declaration, &replacement)], "").is_err(),
            "{replacement}"
        );
    }
    let row = DERIVED
        .lines()
        .find(|line| line.starts_with("0x2001 "))
        .unwrap();
    assert!(
        Registry::parse(
            &[ROOT, &DERIVED.replace(row, &format!("{row} {declaration}"))],
            ""
        )
        .is_err()
    );
    let row = DERIVED
        .lines()
        .find(|line| line.starts_with("0x23ac "))
        .unwrap();
    assert!(
        Registry::parse(
            &[
                ROOT,
                &DERIVED.replace(row, &row.replace("unreviewed", "presumed"))
            ],
            ""
        )
        .is_err()
    );
}

#[test]
fn old_text_dialog_variant_rejects_a_mismatched_binary_identity() {
    let root = include_str!("../profiles/cmvs-20160201.profile");
    for (name, source) in [
        (
            "cmvs-20071117",
            include_str!("../profiles/cmvs-20071117.profile"),
        ),
        (
            "cmvs-20080719",
            include_str!("../profiles/cmvs-20080719.profile"),
        ),
        (
            "cmvs-20080903",
            include_str!("../profiles/cmvs-20080903.profile"),
        ),
    ] {
        let research = Registry::parse(&[root, source], "").unwrap();
        let mut wrong = research.profile(name).unwrap().clone();
        wrong.binaries[0].sha256 = [0; 32];
        assert!(Registry::parse(&[root, &wrong.to_text()], "").is_err());
    }
}

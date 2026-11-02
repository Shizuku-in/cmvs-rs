use cmvs_rs::{
    digest::to_hex,
    dispatch,
    machine::Machine,
    profile::{
        CommandGate, CommandStatus, Evidence, GateErrorKind, Registry, Semantics, VariantId,
    },
};
mod support;
use support::bytecode::{command, pending, script};

const ROOT: &str = include_str!("../crates/cmvs-core/profiles/cmvs-20160201.profile");
const DERIVED: &str = include_str!("../crates/cmvs-core/profiles/cmvs-20171114.profile");
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
    let declaration =
        format!("variant=font-cells:x64:{HASH} evidence=games/aoitori/vm-semantics.md");
    for replacement in [
        declaration.replace("variant=", "research="),
        declaration.replace("font-cells", "unknown"),
        declaration.replace("x64", "unknown"),
        declaration.replace(HASH, &"0".repeat(64)),
        declaration.replace(HASH, "not-a-hash"),
        declaration.replace(&format!(":{HASH}"), ""),
        declaration.replace("games/aoitori/vm-semantics.md", "../outside.md"),
        declaration.replace("games/aoitori/vm-semantics.md", "/outside.md"),
        declaration.replace("games/aoitori/vm-semantics.md", "games/evidence.txt"),
        declaration.replace(" evidence=games/aoitori/vm-semantics.md", ""),
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
fn declared_variant_obeys_unreviewed_block_on_every_attempt() {
    let registry = Registry::builtin();
    let selection = registry
        .select(Some("cmvs-20171114"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection);
    gate.set_block_unreviewed(true);
    gate.set_block_presumed(true);
    let mut code = vec![];
    command(&mut code, 0x23ac, &[1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let (pc, command) = vm.pending_system().unwrap();
    assert_eq!(
        gate.resolve(&vm, pc, command).err().unwrap().kind,
        GateErrorKind::Unreviewed
    );
    gate.set_block_presumed(false);
    assert_eq!(
        gate.resolve(&vm, pc, command).err().unwrap().kind,
        GateErrorKind::Unreviewed
    );
    gate.set_block_unreviewed(false);
    let result = gate.resolve(&vm, pc, command).unwrap();
    assert_eq!(result.semantics, Semantics::Variant(VariantId::FontCells));
    assert_eq!(result.first_unreviewed, Some(command));
    assert_eq!(gate.counts().unreviewed, 1);
    gate.set_block_unreviewed(true);
    assert_eq!(
        gate.resolve(&vm, pc, command).err().unwrap().kind,
        GateErrorKind::Unreviewed
    );
    assert_eq!(vm.pending_system(), Some((pc, command)));
    assert_eq!(vm.stack().len_bytes(), 4);
}

#[test]
fn embedded_variant_evidence_exists_and_names_the_bound_executable_and_commands() {
    for profile in Registry::builtin().profiles() {
        for (command, entry) in profile.variant_bindings() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("docs")
                .join(&entry.evidence);
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains(&to_hex(&entry.binary_sha256)));
            assert!(text.contains(&format!("0x{:04x}", command + 0x2000)));
        }
    }
}

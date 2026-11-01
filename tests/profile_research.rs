use cmvs_rs::{
    digest::to_hex,
    dispatch,
    machine::Machine,
    profile::{
        CommandGate, CommandStatus, Evidence, GateErrorKind, Registry, ResearchId, Semantics,
    },
};
mod support;
use support::bytecode::{command, pending, script};

const ROOT: &str = include_str!("../crates/cmvs-core/profiles/cmvs-20160201.profile");
const DERIVED: &str = include_str!("../crates/cmvs-core/profiles/cmvs-20171114.profile");
const HASH: &str = "3bccda5d7611412c6938c0c9b06b2195914cfb78763a53f2c235bb6479395ee0";

#[test]
fn declared_research_round_trips_and_does_not_depend_on_the_profile_name() {
    let renamed = DERIVED.replacen("profile cmvs-20171114", "profile synthetic-research", 1);
    let registry = Registry::parse(&[ROOT, &renamed], "").unwrap();
    let profile = registry.profile("synthetic-research").unwrap();
    assert_eq!(profile.research_overrides().count(), 8);
    for (command, entry) in profile.research_overrides() {
        assert_eq!(profile.status(command), CommandStatus::Unreviewed);
        assert_eq!(
            profile.semantics(command),
            Semantics::Research(ResearchId::FontCells)
        );
        assert_eq!(to_hex(&entry.binary_sha256), HASH);
        assert!(dispatch::research_entry(entry.id, command).is_some());
        assert!(
            profile
                .binaries
                .iter()
                .any(|b| b.arch == entry.arch && b.sha256 == entry.binary_sha256)
        );
    }
    let canonical = profile.to_text();
    let reparsed = Registry::parse(&[ROOT, &canonical], "").unwrap();
    assert_eq!(reparsed.profile("synthetic-research"), Some(profile));
    let stripped = renamed
        .lines()
        .map(|line| line.split(" research=").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let registry = Registry::parse(&[ROOT, &stripped], "").unwrap();
    assert_eq!(
        registry
            .profile("synthetic-research")
            .unwrap()
            .semantics(0x3ac),
        Semantics::Root
    );
}

#[test]
fn research_declarations_reject_unknown_handlers_bad_bindings_and_unsafe_evidence_paths() {
    let declaration =
        format!("research=font-cells:x64:{HASH} evidence=games/aoitori/vm-semantics.md");
    for replacement in [
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
fn declared_research_still_requires_unreviewed_permission_on_every_attempt() {
    let registry = Registry::builtin();
    let selection = registry
        .select(Some("cmvs-20171114"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection);
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
    gate.set_allow_presumed(true);
    assert_eq!(
        gate.resolve(&vm, pc, command).err().unwrap().kind,
        GateErrorKind::Unreviewed
    );
    gate.set_allow_unreviewed(true);
    let result = gate.resolve(&vm, pc, command).unwrap();
    assert_eq!(result.semantics, Semantics::Research(ResearchId::FontCells));
    assert_eq!(result.first_unreviewed, Some(command));
    assert_eq!(gate.counts().unreviewed, 1);
    gate.set_allow_unreviewed(false);
    assert_eq!(
        gate.resolve(&vm, pc, command).err().unwrap().kind,
        GateErrorKind::Unreviewed
    );
    assert_eq!(vm.pending_system(), Some((pc, command)));
    assert_eq!(vm.stack().len_bytes(), 4);
}

#[test]
fn embedded_research_evidence_exists_and_names_the_bound_executable_and_commands() {
    for profile in Registry::builtin().profiles() {
        for (command, entry) in profile.research_overrides() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("docs")
                .join(&entry.evidence);
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains(&to_hex(&entry.binary_sha256)));
            assert!(text.contains(&format!("0x{:04x}", command + 0x2000)));
        }
    }
}

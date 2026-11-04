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
fn asukimi_queries_use_the_down_edge_and_extra_actions_can_be_consumed() {
    let profile = Registry::builtin().profile("cmvs-20071117").unwrap();
    let queries = [
        0x19c, 0x19e, 0x1a0, 0x1a2, 0x1a4, 0x1a6, 0x1a8, 0x1aa, 0x1ac, 0x1ae, 0x1b0, 0x1b2, 0x1b4,
        0x1b6, 0x1b8, 0x1ba, 0x1bc, 0x2f0, 0x2f2, 0x2f4,
    ];
    let actions = [
        15, 16, 0, 1, 7, 8, 6, 9, 10, 11, 12, 13, 14, 2, 3, 4, 5, 17, 18, 19,
    ];
    for (query, action) in queries.into_iter().zip(actions) {
        let binding = profile.variant(query).unwrap();
        assert_eq!(binding.id, VariantId::InputPress);
        assert_eq!(
            to_hex(&binding.binary_sha256),
            "caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b"
        );
        for (pressed, released, held) in [
            (true, false, true),
            (false, true, false),
            (false, false, true),
        ] {
            let mut code = vec![];
            command(&mut code, query + 0x2000, &[]);
            command(&mut code, query + 0x2000, &[]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            let mut input = cmvs_rs::input::ActionInput::default();
            input.set(action, held, pressed, released).unwrap();
            for _ in 0..2 {
                pending(&mut vm);
                let mut hosts = dispatch::Hosts {
                    input: Some(dispatch::InputHosts {
                        input: &mut input,
                        skip: false,
                        clock: None,
                    }),
                    ..Default::default()
                };
                assert_eq!(
                    dispatch::run(profile.semantics(query), &mut vm, &mut hosts).unwrap(),
                    Some(dispatch::Flow::Done)
                );
                assert_eq!(vm.state().system_integer(0), Some(u32::from(pressed)));
                assert_eq!(vm.state().system_integer(4), Some(u32::from(held)));
                assert_eq!(vm.stack().len_bytes(), 0);
            }
        }
    }
    for query in [0x2f0, 0x2f2, 0x2f4] {
        let mut code = vec![];
        command(&mut code, query + 0x2001, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut input = cmvs_rs::input::ActionInput::default();
        let action = 17 + usize::from((query - 0x2f0) / 2);
        input.set(action, true, true, false).unwrap();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        let mut hosts = dispatch::Hosts {
            input: Some(dispatch::InputHosts {
                input: &mut input,
                skip: false,
                clock: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            dispatch::run(profile.semantics(query + 1), &mut vm, &mut hosts).unwrap(),
            Some(dispatch::Flow::Done)
        );
        assert!(!input.pressed(action));
        assert!(input.held(action));
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
}

#[test]
fn font_cells_bindings_are_limited_to_the_inspected_commands() {
    let remake = Registry::builtin().profile("cmvs-20221105").unwrap();
    for command in [0x2a8, 0x2ae] {
        assert_eq!(
            remake.semantics(command),
            Semantics::Variant(VariantId::Fragments)
        );
    }
    for (id, hash) in [
        (
            "cmvs-20190305",
            "02b71aac3d3d1ced1d0312b6c37748cf12df9cffd9585fb07787e6ca43199f00",
        ),
        (
            "cmvs-20190405",
            "c1742b5873e6d03f5cc7bc6f5b3b23655ec67e7cddf307deefa75349caed33be",
        ),
        (
            "cmvs-20211006",
            "58e993a82e741ea73d3d1b5d89363dcaf435e72e6d505324e1de0c0ba5486631",
        ),
        (
            "cmvs-20221105",
            "dfbabd2abac5055d3ab2144a983b6c441b87b294d19607e9a4829e2ea954e86c",
        ),
    ] {
        let profile = Registry::builtin().profile(id).unwrap();
        let bindings: Vec<_> = profile
            .variant_bindings()
            .filter(|(_, binding)| binding.id == VariantId::FontCells)
            .map(|(command, _)| command)
            .collect();
        let mut expected = vec![0x11d, 0x388, 0x38b, 0x3ac, 0x3b8, 0x3b9, 0x3bb];
        if matches!(id, "cmvs-20211006" | "cmvs-20221105") {
            expected.extend([0x10b, 0x152, 0x154, 0x157, 0x159]);
            expected.sort_unstable();
        }
        assert_eq!(bindings, expected);
        assert_eq!(profile.status(0x3ac), CommandStatus::Unreviewed);
        assert_eq!(
            profile.semantics(0x3ac),
            Semantics::Variant(VariantId::FontCells)
        );
        assert_eq!(
            profile.semantics(0x10b),
            if matches!(id, "cmvs-20211006" | "cmvs-20221105") {
                Semantics::Variant(VariantId::FontCells)
            } else {
                Semantics::Root
            }
        );
        let binding = profile.variant(0x3ac).unwrap();
        assert_eq!(to_hex(&binding.binary_sha256), hash);
        let mut code = vec![];
        command(&mut code, 0x23ac, &[u32::MAX]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        assert_eq!(
            dispatch::run(
                profile.semantics(0x3ac),
                &mut vm,
                &mut dispatch::Hosts::default()
            )
            .unwrap(),
            Some(dispatch::Flow::Unavailable)
        );
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.pc(), pc);
        let mut messages = cmvs_rs::message::MessageHost::default();
        let mut input = cmvs_rs::input::ActionInput::default();
        let mut hosts = dispatch::Hosts {
            text: Some(dispatch::TextHosts {
                messages: &mut messages,
                input: &mut input,
                flags: Default::default(),
            }),
            ..Default::default()
        };
        assert_eq!(
            dispatch::run(profile.semantics(0x3ac), &mut vm, &mut hosts).unwrap(),
            Some(dispatch::Flow::Done)
        );
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
    }
}

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

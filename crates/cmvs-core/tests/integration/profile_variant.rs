use crate::support;
use cmvs_core::{
    format::digest::to_hex, profile::CommandGate, profile::CommandStatus, profile::Evidence,
    profile::GateErrorKind, profile::Registry, profile::Semantics, profile::VariantId,
    vm::dispatch, vm::machine::Machine,
};
use support::bytecode::{command, pending, script};

#[test]
fn window_sequence_append_copies_names_and_preserves_results_at_capacity() {
    let profile = Registry::builtin().profile("cmvs-20101211").unwrap();
    let semantics = profile.semantics(0x28b);
    assert_eq!(
        semantics,
        Semantics::Variant(VariantId::SequenceWindowImage)
    );
    assert!(!dispatch::ROOT.iter().any(|entry| entry.command == 0x28b));
    let mut code = vec![];
    for child in 0..18 {
        command(&mut code, 0x228b, &[child, 0x8000_0003, 100]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut sequence = cmvs_core::runtime::SequenceHost::default();
    vm.state_mut().set_string(3, b"first.pb3").unwrap();
    vm.state_mut().set_system_integer(0, 77).unwrap();
    for index in 0..17 {
        pending(&mut vm);
        let mut hosts = dispatch::Hosts {
            sequence: Some(dispatch::SequenceHosts {
                host: &mut sequence,
                execution: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            dispatch::run(semantics, &mut vm, &mut hosts).unwrap(),
            Some(dispatch::Flow::Done)
        );
        vm.state_mut().set_string(3, b"second.pb3").unwrap();
        assert_eq!(sequence.header().count, (index + 1).min(16));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
    assert!(
        matches!(&sequence.entries()[0], Some(cmvs_core::runtime::SequenceEntry::ReplaceWindowImage { child: 0, name, duration: 100, .. }) if name == b"first.pb3")
    );
    let entries = sequence.entries().clone();
    vm.state_mut().set_string(3, b"x").unwrap();
    pending(&mut vm);
    // A full queue still resolves its string reference before discarding the append.
    vm.state_mut().set_string(3, &[]).unwrap();
    let mut hosts = dispatch::Hosts {
        sequence: Some(dispatch::SequenceHosts {
            host: &mut sequence,
            execution: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        dispatch::run(semantics, &mut vm, &mut hosts).unwrap(),
        Some(dispatch::Flow::Done)
    );
    assert_eq!(sequence.entries(), &entries);
}

#[test]
fn inspected_early_queries_use_the_down_edge_and_extra_actions_can_be_consumed() {
    for (id, hash) in [
        (
            "cmvs-20101211",
            "158409c71cf14c3dedb7d051585e5f48e56dbc621bb19e56d084cd4fe8d4352f",
        ),
        (
            "cmvs-20071117",
            "caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b",
        ),
        (
            "cmvs-20080719",
            "afe3b3a895e7b3ba5f78e6f6464296f1d028e87307c8de5a568b88fd6086491e",
        ),
        (
            "cmvs-20090815",
            "36fa780a5d2b8df4ac6a6e48a2f78708b79aa10d22eb575a5ae74742d902f71a",
        ),
        (
            "cmvs-20080903",
            "f77ecbfd39d1076c66915544d73b9c2d2c5876fbabeaef3d589050b6094d13ac",
        ),
    ] {
        let profile = Registry::builtin().profile(id).unwrap();
        assert!(
            profile
                .binaries
                .iter()
                .any(|binary| to_hex(&binary.sha256) == hash)
        );
        let queries = [
            0x19c, 0x19e, 0x1a0, 0x1a2, 0x1a4, 0x1a6, 0x1a8, 0x1aa, 0x1ac, 0x1ae, 0x1b0, 0x1b2,
            0x1b4, 0x1b6, 0x1b8, 0x1ba, 0x1bc, 0x2f0, 0x2f2, 0x2f4,
        ];
        let actions = [
            15, 16, 0, 1, 7, 8, 6, 9, 10, 11, 12, 13, 14, 2, 3, 4, 5, 17, 18, 19,
        ];
        for (query, action) in queries.into_iter().zip(actions) {
            assert_eq!(
                profile.semantics(query),
                Semantics::Variant(VariantId::InputPress)
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
                let mut input = cmvs_core::host::input::ActionInput::default();
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
            let mut input = cmvs_core::host::input::ActionInput::default();
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
        assert!(
            profile
                .binaries
                .iter()
                .any(|binary| to_hex(&binary.sha256) == hash)
        );
        let bindings: Vec<_> = (0..profile.statuses().len() as u16)
            .filter(|&command| {
                profile.semantics(command) == Semantics::Variant(VariantId::FontCells)
            })
            .collect();
        assert_eq!(
            bindings,
            [
                0x10b, 0x11d, 0x152, 0x154, 0x157, 0x159, 0x388, 0x38b, 0x3ac, 0x3b8, 0x3b9, 0x3bb
            ]
        );
        assert_eq!(profile.status(0x3ac), CommandStatus::Unreviewed);
        for command in [0x10b, 0x3ac] {
            assert_eq!(
                profile.semantics(command),
                Semantics::Variant(VariantId::FontCells)
            );
        }
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
        let mut messages = cmvs_core::render::message::MessageHost::default();
        let mut input = cmvs_core::host::input::ActionInput::default();
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

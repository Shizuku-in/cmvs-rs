use crate::support;
use cmvs_core::{
    host::input::ActionInput, profile::CommandGate, profile::Registry,
    render::message::MessageHost, vm::dispatch, vm::dispatch::Flow, vm::dispatch::Hosts,
    vm::dispatch::TextHosts, vm::machine::Machine,
};
use support::bytecode::{command, pending, script};

#[test]
fn mapped_cvns_command_reports_its_script_opcode_for_helper_errors() {
    let profile = Registry::builtin().profile("cvns-20090203").unwrap();
    for (args, reason) in [
        ([999, 0], "resource path slot outside supported range"),
        ([0, 99], "string reference out of bounds"),
    ] {
        let mut code = Vec::new();
        command(&mut code, 0x2021, &args);
        let mut vm = Machine::new(script(&code)).unwrap();
        vm.set_command_map(profile.command_map());
        pending(&mut vm);
        assert_eq!(vm.pending_system(), Some((16, 0x11)));
        let err = dispatch::run(
            profile.semantics(0x21),
            &mut vm,
            &mut Hosts {
                bootstrap: Some(&mut Default::default()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("code+0x10 opcode=0x2021: {reason}")
        );
        assert_eq!(err.as_decode().unwrap().reason, reason);
    }
}

#[test]
fn missing_capabilities_preserve_pending_commands_and_arguments() {
    for entry in dispatch::ROOT.iter().chain(dispatch::CVNS) {
        let mut code = vec![];
        command(&mut code, entry.command + 0x2000, &[77]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 42).unwrap();
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(
                cmvs_core::profile::Semantics::Root,
                &mut vm,
                &mut Hosts::default()
            )
            .unwrap(),
            Some(Flow::Unavailable),
            "{}",
            entry.name
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 77);
        assert_eq!(vm.state().system_integer(0), Some(42));
    }
}

#[test]
fn screen_coordinates_change_only_the_selected_draw_mode() {
    use cmvs_core::{profile::Semantics, render::scene::SceneHost};
    let profile = Registry::builtin().profile("cmvs-20080719").unwrap();
    for child in [u32::MAX, 255] {
        for mode in [0, 1, 2, u32::MAX] {
            let mut code = vec![];
            for (op, args) in [
                (0x2020, vec![20]),
                (0x2022, vec![20, 255]),
                (0x2040, vec![20]),
                (0x2050, vec![20, 255]),
                (0x2045, vec![20, child, 37, 81]),
                (0x204e, vec![20, child, mode]),
            ] {
                command(&mut code, op, &args);
            }
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            let mut scene = SceneHost::for_profile(profile);
            for _ in 0..5 {
                pending(&mut vm);
                dispatch::run(
                    Semantics::Root,
                    &mut vm,
                    &mut Hosts {
                        scene: Some(&mut scene),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            let mut expected = scene.object(20).unwrap().clone();
            let object = if child == u32::MAX {
                &mut expected
            } else {
                expected.children.get_mut(&255).unwrap()
            };
            object
                .draw
                .as_mut()
                .unwrap()
                .set_coordinate_mode(u32::from(mode != 0));
            pending(&mut vm);
            // The root has no implementation of this entry; only the hash-bound override runs.
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
                None
            );
            assert_eq!(
                dispatch::run(
                    profile.semantics(0x4e),
                    &mut vm,
                    &mut Hosts {
                        scene: Some(&mut scene),
                        ..Default::default()
                    }
                )
                .unwrap(),
                Some(Flow::Done)
            );
            assert_eq!(scene.object(20), Some(&expected));
            assert_eq!(vm.stack().len_bytes(), 0);
        }
    }
}

#[test]
fn screen_coordinates_reject_invalid_targets_without_consuming_arguments() {
    use cmvs_core::{profile::Semantics, render::scene::SceneHost};
    let profile = Registry::builtin().profile("cmvs-20080719").unwrap();
    for args in [
        vec![20, u32::MAX, 1],
        vec![20, 256, 1],
        vec![21, u32::MAX, 1],
        vec![256, u32::MAX, 1],
        vec![20, 0, 1],
        vec![20, u32::MAX],
        vec![],
    ] {
        let mut code = vec![];
        command(&mut code, 0x2020, &[20]);
        command(&mut code, 0x204e, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::for_profile(profile);
        pending(&mut vm);
        dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                scene: Some(&mut scene),
                ..Default::default()
            },
        )
        .unwrap();
        pending(&mut vm);
        let before = scene.objects().clone();
        assert!(
            dispatch::run(
                profile.semantics(0x4e),
                &mut vm,
                &mut Hosts {
                    scene: Some(&mut scene),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0x4e));
    }
}

#[test]
fn migrated_tables_have_unique_owners_and_overrides_are_build_bound() {
    for table in [dispatch::ROOT, dispatch::CVNS]
        .into_iter()
        .chain(dispatch::VARIANT_TABLES.iter().copied())
    {
        let owners: std::collections::BTreeSet<_> = table.iter().map(|e| e.command).collect();
        assert_eq!(owners.len(), table.len());
        for entry in table {
            if entry.capability == dispatch::Capability::ParticleImages {
                assert!(dispatch::uses_images(entry.command));
            }
        }
    }
    let registry = Registry::builtin();
    for id in ["cmvs-20160201", "cmvs-20150405"] {
        assert!(dispatch::FONT_CELLS.iter().all(|entry| {
            registry.profile(id).unwrap().semantics(entry.command)
                == cmvs_core::profile::Semantics::Root
        }));
    }
    let derived = registry.profile("cmvs-20171114").unwrap();
    // The face setter has only been inspected for REALIVE.
    assert_eq!(
        derived.semantics(0x388),
        cmvs_core::profile::Semantics::Root
    );
    for entry in dispatch::FONT_CELLS.iter().filter(|e| e.command != 0x388) {
        assert_eq!(
            derived.semantics(entry.command),
            cmvs_core::profile::Semantics::Variant(cmvs_core::profile::VariantId::FontCells)
        );
        assert_eq!(
            derived.status(entry.command),
            cmvs_core::profile::CommandStatus::Unreviewed
        );
    }
}

#[test]
fn missing_text_capability_preserves_pending_override_and_never_uses_root() {
    let mut code = vec![];
    command(&mut code, 0x23ac, &[1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let profile = Registry::builtin().profile("cmvs-20171114").unwrap();
    let before = vm.stack().len_bytes();
    assert_eq!(
        dispatch::run(profile.semantics(0x3ac), &mut vm, &mut Hosts::default()).unwrap(),
        Some(Flow::Unavailable)
    );
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
    assert_eq!(vm.stack().len_bytes(), before);
    let root = Registry::builtin().profile("cmvs-20160201").unwrap();
    assert_eq!(
        dispatch::run(root.semantics(0x3ac), &mut vm, &mut Hosts::default()).unwrap(),
        None
    );
}

#[test]
fn font_cell_mode_requires_research_gate_and_short_stack_is_transactional() {
    let mut code = vec![];
    command(&mut code, 0x23ac, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let registry = Registry::builtin();
    let selection = registry
        .select(
            Some("cmvs-20171114"),
            &cmvs_core::profile::Evidence::default(),
        )
        .unwrap();
    let mut gate = CommandGate::new(selection);
    gate.set_block_unreviewed(true);
    let (pc, command) = vm.pending_system().unwrap();
    assert!(gate.check(&vm, pc, command).is_err());
    gate.set_block_unreviewed(false);
    assert_eq!(gate.check(&vm, pc, command).unwrap(), Some(0x3ac));
    let mut messages = MessageHost::default();
    let mut input = ActionInput::default();
    let error = dispatch::run(
        registry.profile("cmvs-20171114").unwrap().semantics(0x3ac),
        &mut vm,
        &mut Hosts {
            text: Some(TextHosts {
                messages: &mut messages,
                input: &mut input,
                flags: Default::default(),
            }),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.as_decode().unwrap().reason, "enabled");
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
}

#[test]
fn state_dispatch_errors_name_arguments_and_preserve_vm_and_hosts() {
    use cmvs_core::{
        host::random::CrtRandom, profile::Semantics, runtime::MessageFlags, runtime::VmTimers,
    };
    for (opcode, args, reason) in [
        (0x200b, vec![], "timer slot"),
        (0x200c, vec![10], "timer slot outside 0..9"),
        (0x20d4, vec![], "divisor"),
        (0x20d4, vec![0], "rand() % 0 faults in the original"),
        (0x22d0, vec![1], "value"),
        (0x22eb, vec![0], "unused value"),
        (0x22e5, vec![], "mode"),
        (0x2340, vec![], "bypass"),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        vm.state_mut().set_system_integer(0, 42).unwrap();
        let mut timers = VmTimers::default();
        timers.begin_run(19);
        let mut flags = MessageFlags::default();
        let mut random = CrtRandom::new(7);
        let error = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                timers: Some(&mut timers),
                flags: Some(&mut flags),
                random: Some(&mut random),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.as_decode().unwrap().reason, reason);
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(42));
        assert_eq!(timers.values(), &[19; 10]);
        assert_eq!(flags, MessageFlags::default());
        assert_eq!(random, CrtRandom::new(7));
    }
}

#[test]
fn shared_flag_queries_discard_unknown_value_words_without_reading_them() {
    use cmvs_core::{profile::Semantics, runtime::MessageFlags};
    for opcode in [0x22d0u16, 0x22e5, 0x22eb] {
        let mut code = vec![];
        // Reserving eight bytes initializes only the top word. After the mode
        // push, the query's second word exists but has unknown bytes.
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend(77u32.to_le_bytes());
        command(&mut code, opcode, &[0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut flags = MessageFlags::default();
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    flags: Some(&mut flags),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(flags, MessageFlags::default());
    }
}

#[test]
fn old_text_dialog_commits_accept_cancel_and_empty_results() {
    use cmvs_core::host::platform::RecordingBackend;
    for name in ["cmvs-20071117", "cmvs-20080719", "cmvs-20080903"] {
        let profile = Registry::builtin().profile(name).unwrap();
        for (encoding, answer, result, text) in [
            (
                cmvs_core::TextEncoding::ShiftJis,
                Some(b"A\x82\xa0x".to_vec()),
                1,
                b"\x82\xa0".as_slice(),
            ),
            (
                cmvs_core::TextEncoding::ShiftJis,
                None,
                0,
                b"old comment".as_slice(),
            ),
            (
                cmvs_core::TextEncoding::ShiftJis,
                Some(vec![]),
                1,
                b"old comment".as_slice(),
            ),
            (
                cmvs_core::TextEncoding::Gbk,
                Some(b"A\xd6\xd0x\xce\\".to_vec()),
                1,
                b"\xd6\xd0\xce\\".as_slice(),
            ),
            (
                cmvs_core::TextEncoding::Utf8,
                Some("A中xé文".as_bytes().to_vec()),
                1,
                "中é文".as_bytes(),
            ),
        ] {
            let mut code = vec![];
            command(
                &mut code,
                0x213c,
                &[0x80000000, 0x80000001, 0x80000002, 32, 0x80000003, 1],
            );
            let program = script(&code);
            let mut vm = Machine::new_with_encoding(&program, encoding).unwrap();
            for (i, value) in [b"Title".as_slice(), b"Prompt", b"old comment", b"before"]
                .iter()
                .enumerate()
            {
                vm.state_mut().set_string(i as u32, value).unwrap();
            }
            pending(&mut vm);
            let mut backend = RecordingBackend::default();
            backend.answer_text_dialog(answer);
            let mut hosts = Hosts {
                device_frame: Some(&mut backend),
                ..Default::default()
            };
            assert_eq!(
                dispatch::run(profile.semantics(0x13c), &mut vm, &mut hosts).unwrap(),
                Some(Flow::Done)
            );
            assert_eq!(vm.state().system_integer(0), Some(result));
            assert_eq!(vm.state().string(3), Some(text));
            assert_eq!(vm.stack().len_bytes(), 0);
            assert!(vm.pending_system().is_none());
            assert_eq!(backend.text_dialogs()[0].initial, b"old comment");
            assert_eq!(backend.text_dialogs()[0].max_bytes, 32);
        }
    }
}

#[test]
fn old_text_dialog_waits_and_rejects_invalid_results_atomically() {
    use cmvs_core::{
        host::platform::PlatformBackend, host::platform::PlatformRequest,
        host::platform::RecordingBackend, host::platform::TextDialogRequest,
        host::platform::TextDialogResult,
    };
    struct Pending;
    impl PlatformBackend for Pending {
        fn apply(
            &mut self,
            _: &PlatformRequest,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unreachable!()
        }
        fn poll_text_dialog(
            &mut self,
            _: &TextDialogRequest,
        ) -> Result<TextDialogResult, Box<dyn std::error::Error + Send + Sync>> {
            Ok(TextDialogResult::Pending)
        }
    }
    let profile = Registry::builtin().profile("cmvs-20080719").unwrap();
    let mut code = vec![];
    command(
        &mut code,
        0x213c,
        &[0x80000000, 0x80000000, 0x80000000, 32, 0x80000000, 0],
    );
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    vm.state_mut().set_string(0, b"unchanged").unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    pending(&mut vm);
    let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
    assert_eq!(
        dispatch::run(
            profile.semantics(0x13c),
            &mut vm,
            &mut Hosts {
                device_frame: Some(&mut Pending),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Wait)
    );
    let mut backend = RecordingBackend::default();
    for answer in [vec![b'x'; 256], b"bad\0comment".to_vec()] {
        backend.answer_text_dialog(Some(answer));
        assert!(
            dispatch::run(
                profile.semantics(0x13c),
                &mut vm,
                &mut Hosts {
                    device_frame: Some(&mut backend),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(9));
        assert_eq!(vm.state().string(0), Some(b"unchanged".as_slice()));
    }
    assert_eq!(
        dispatch::run(
            cmvs_core::profile::Semantics::Root,
            &mut vm,
            &mut Hosts::default()
        )
        .unwrap(),
        None
    );
}

#[test]
fn old_text_dialog_treats_negative_wrapped_bank_initial_as_empty_without_reading_output() {
    use cmvs_core::host::platform::RecordingBackend;

    fn string_expr_nested_bank(code: &mut Vec<u8>, index: i32) {
        code.extend(0x0201u16.to_le_bytes());
        code.extend(0x0121u16.to_le_bytes());
        code.extend(0x0200u16.to_le_bytes());
        code.extend(0x0100u16.to_le_bytes());
        code.extend((index as u32).to_le_bytes());
        code.extend(0x020fu16.to_le_bytes());
        code.extend(0x020fu16.to_le_bytes());
    }

    let profile = Registry::builtin().profile("cvns-20050727").unwrap();
    let mut code = vec![];
    // Push 0, uninitialized slot 15, 96, Nested(0x121, -29), Title, Prompt, 0x21b2.
    command(&mut code, 0x21b2, &[0, 0x8000_000f, 96]);
    code.truncate(code.len() - 2);
    string_expr_nested_bank(&mut code, -29);
    code.extend(0x0430u16.to_le_bytes());
    for raw in [0x8000_0001u32, 0x8000_0000] {
        code.extend(0x0440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(raw.to_le_bytes());
    }
    code.extend(0x21b2u16.to_le_bytes());

    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    vm.set_command_map(profile.command_map());
    vm.state_mut().set_string_slots(256);
    vm.state_mut().set_string(0, b"Title").unwrap();
    vm.state_mut().set_string(1, b"Prompt").unwrap();
    // Leave slot 15 uninitialized to verify write-only output is not read.
    pending(&mut vm);
    let mut backend = RecordingBackend::default();
    backend.answer_text_dialog(Some(vec![0x82, 0xa0])); // Shift-JIS "あ"
    assert_eq!(
        dispatch::run(
            profile.semantics(0x1b2),
            &mut vm,
            &mut Hosts {
                device_frame: Some(&mut backend),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(backend.text_dialogs().len(), 1);
    assert!(backend.text_dialogs()[0].initial.is_empty());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.state().string(15), Some(&[0x82, 0xa0][..]));

    // Ordinary small Frame offsets on `initial` must still fail cleanly without mutating state.
    let mut bad = vec![];
    command(
        &mut bad,
        0x21b2,
        &[0x8000_0000, 0x8000_0001, 0xc000_0004, 96, 0x8000_000f, 0],
    );
    let program = script(&bad);
    let mut vm = Machine::new(&program).unwrap();
    vm.set_command_map(profile.command_map());
    vm.state_mut().set_string_slots(256);
    vm.state_mut().set_string(0, b"Title").unwrap();
    vm.state_mut().set_string(1, b"Prompt").unwrap();
    pending(&mut vm);
    assert!(
        dispatch::run(
            profile.semantics(0x1b2),
            &mut vm,
            &mut Hosts {
                device_frame: Some(&mut backend),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(vm.state().string(15).is_none());
}

use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts},
    gesture::PointerGesture,
    machine::Machine,
    profile::Semantics,
    ps3::Script,
    save_menu::SaveMenuHost,
    snapshot::{SaveInfo, SaveTitle},
};
#[allow(dead_code)]
mod support;
use support::bytecode::{command, pending};

fn script_bytes(code: &[u8]) -> Vec<u8> {
    let text = b"title\0";
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(text)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&script_bytes(code)).unwrap()
}

#[test]
fn metadata_entries_require_their_own_capability_and_complete_once() {
    let cases: Vec<(u16, Vec<u32>, u32)> = vec![
        (0x2350, vec![1], 99),
        (0x2351, vec![1, 0], 99),
        (0x2352, vec![1], 99),
        (0x2353, vec![], u32::MAX),
        (0x2356, vec![1], 99),
        (0x2357, vec![1], 99),
        (0x2358, vec![1], 99),
        (0x2359, vec![], 0),
        (0x235a, vec![45], 99),
        (0x235b, vec![123], 99),
        (0x2360, vec![], u32::MAX),
        (0x2362, vec![1], 0),
        (0x2364, vec![1], 99),
        (0x2365, vec![], 0),
        (0x2366, vec![2, 7, 11, 12, 13, 14], 99),
        (0x2367, vec![], 99),
        (0x2368, vec![], u32::MAX),
        (0x22cd, vec![0], 99),
        (0x22ce, vec![], 1),
        (0x22d3, vec![1, 1], 99),
        (0x22d4, vec![1, 1], 99),
        (0x22ee, vec![], 0),
        (0x2162, vec![0], 99),
        (0x22b9, vec![71], 99),
        (0x22ba, vec![72, 73], 99),
        (0x22d1, vec![1, 0], 99),
        (0x22a0, vec![u32::MAX], 99),
    ];
    let entries: Vec<_> = dispatch::ROOT
        .iter()
        .filter(|e| {
            matches!(
                e.capability,
                Capability::Gesture
                    | Capability::Popup
                    | Capability::SaveTitle
                    | Capability::SaveInfo
                    | Capability::Effects
                    | Capability::EffectObjects
            )
        })
        .collect();
    use std::collections::BTreeSet;
    assert_eq!(
        cases.iter().map(|c| c.0).collect::<BTreeSet<_>>(),
        entries.iter().map(|e| e.command + 0x2000).collect()
    );
    for (opcode, args, result) in cases {
        let entry = entries
            .iter()
            .find(|e| e.command + 0x2000 == opcode)
            .unwrap();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        for slot in 0..7 {
            vm.state_mut().set_system_integer(slot, 99).unwrap();
        }
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        let mut gesture = PointerGesture::default();
        let mut popup = SaveMenuHost::default();
        let mut title = SaveTitle::default();
        let mut info = SaveInfo::default();
        let mut effects = true;
        let mut hosts = Hosts {
            gesture: (entry.capability == Capability::Gesture).then_some(&mut gesture),
            popup: (entry.capability == Capability::Popup).then_some(&mut popup),
            save_title: (entry.capability == Capability::SaveTitle).then_some(&mut title),
            save_info: (entry.capability == Capability::SaveInfo).then_some(&mut info),
            effects: (entry.capability == Capability::Effects).then_some(&mut effects),
            effect_objects: entry.capability == Capability::EffectObjects,
            ..Default::default()
        };
        assert_eq!(
            dispatch::run(Semantics::ResearchFontCells, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.pc(), code.len());
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(result));
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            None
        );
        match opcode {
            0x2350 => assert_eq!(gesture.field(0), Some(1)),
            0x2351 => {
                assert_eq!(gesture.field(24), Some(1));
                assert_eq!(gesture.field(28), Some(0));
            }
            0x2352 => assert_eq!(gesture.field(8), Some(1)),
            0x2356..=0x2358 => assert_eq!(gesture.field(12 + 4 * (opcode - 0x2356)), Some(1)),
            0x235a => assert_eq!(gesture.field(36), Some(45)),
            0x235b => assert_eq!(gesture.field(32), Some(123)),
            0x2364 => assert_eq!(gesture.field(4), Some(1)),
            0x2366 => assert_eq!(
                [228, 232, 236, 240, 244].map(|at| gesture.field(at)),
                [1, 11, 12, 13, 14].map(Some)
            ),
            0x22cd => assert!(!popup.popup_enabled()),
            0x22d3 => assert_eq!(popup.confirmations(), [true, false]),
            0x22d4 => assert_eq!(popup.confirmations(), [false, true]),
            0x2162 => assert_eq!(title.bytes(), b"title"),
            0x22b9 => assert_eq!(info.values(), [Some(71), None, None]),
            0x22ba => assert_eq!(info.values(), [None, Some(72), Some(73)]),
            0x22d1 => assert!(!effects),
            _ => {}
        }
        if opcode == 0x2359 {
            assert_eq!(vm.state().system_integer(4), Some(0));
        }
        if opcode == 0x2365 || opcode == 0x2368 {
            assert_eq!(vm.state().system_integer(6), Some(0));
        }
    }
}

#[test]
fn popup_and_effect_queries_ignore_unknown_words_but_setters_require_them() {
    for opcode in [0x22d1u16, 0x22d3, 0x22d4] {
        for mode in [0u32, 1] {
            let mut code = vec![];
            code.extend(0x440u16.to_le_bytes());
            code.extend(8u16.to_le_bytes());
            code.extend(77u32.to_le_bytes());
            command(&mut code, opcode, &[mode]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            vm.state_mut().set_system_integer(0, 99).unwrap();
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let mut effects = true;
            let mut popup = SaveMenuHost::default();
            popup.set_confirmations(true, true);
            let outcome = dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    popup: Some(&mut popup),
                    effects: Some(&mut effects),
                    ..Default::default()
                },
            );
            if mode == 0 {
                assert_eq!(outcome.unwrap(), Some(Flow::Done));
                assert_eq!(vm.state().system_integer(0), Some(1));
                assert_eq!(vm.stack().len_bytes(), 4);
            } else {
                assert_eq!(outcome.unwrap_err().reason, "value");
                assert_eq!(
                    (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                    before
                );
                assert_eq!(vm.state().system_integer(0), Some(99));
            }
            assert!(effects);
            assert_eq!(popup.confirmations(), [true, true]);
        }
    }
}

#[test]
fn metadata_failures_preserve_pending_command_results_and_existing_fields() {
    for (opcode, args, reason) in [
        (0x2351, vec![1], "second flag"),
        (0x2366, vec![0, 1, 2, 3, 4], "height"),
        (0x2162, vec![], "title"),
        (
            0x2162,
            vec![0x80000001],
            "save title of 128+ bytes sets an unmodeled flag",
        ),
        (0x22b9, vec![], "first word"),
        (0x22ba, vec![1], "second word"),
        (0x22cd, vec![], "enabled"),
        (0x22d3, vec![0], "unused value"),
        (0x22d1, vec![1], "value"),
        (0x22a0, vec![], "slot"),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 99).unwrap();
        vm.state_mut().set_string(1, &[b'x'; 128]).unwrap();
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut gesture = PointerGesture::default();
        let original = gesture.clone();
        let mut popup = SaveMenuHost::default();
        let mut title = SaveTitle::default();
        let mut info = SaveInfo::default();
        let mut effects = true;
        let error = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                gesture: Some(&mut gesture),
                popup: Some(&mut popup),
                save_title: Some(&mut title),
                save_info: Some(&mut info),
                effects: Some(&mut effects),
                effect_objects: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert_eq!(gesture, original);
        assert!(title.bytes().is_empty());
        assert_eq!(info.values(), [None; 3]);
        assert!(effects && popup.popup_enabled());
        assert_eq!(popup.confirmations(), [false; 2]);
    }
}

#[test]
fn trace_metadata_requires_recording_and_preserves_unsupported_capabilities() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-metadata-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let input = root.join("synthetic.ps3");
    for (opcode, args, log, supported) in [
        (0x2350u16, vec![1], "# pointer gesture config 0x2350;", true),
        (0x22ce, vec![], "# popup menu 0x22ce:", true),
        (
            0x2162,
            vec![0],
            "# save title 0x2162 = title; recorded only",
            true,
        ),
        (0x22a0, vec![0], "# effect object release 0x22a0:", true),
        (0x22b9, vec![1], "# save info", false),
        (0x22d1, vec![0, 0], "# effects setting", false),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        command(&mut code, 0x2003, &[]);
        let raw = script_bytes(&code);
        fs::write(&input, &raw).unwrap();
        for record in [false, true] {
            let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
            cli.arg(&input).args(["--profile", "cmvs-20160201"]);
            if record {
                cli.arg("--record-platform").arg(&root);
            }
            let output = cli.output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let available = record && supported;
            assert_eq!(stdout.contains(log), available, "{stdout}");
            let stop = if available { 0x2003 } else { opcode };
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&input).unwrap(), raw);
    }
    fs::remove_file(input).unwrap();
    fs::remove_dir(root).unwrap();
}

use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts},
    history::HistoryHost,
    machine::Machine,
    profile::Semantics,
    ps3::Script,
    settings::SettingsHost,
};
#[allow(dead_code)]
mod support;
use support::bytecode::{command, pending};

const TEXT: &[u8] = b"abc\0ABC\0xy\0z\0mask%z3a.pb3\0";
fn script_bytes(code: &[u8]) -> Vec<u8> {
    let mut raw = b"PS2A".to_vec();
    for value in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        TEXT.len() as u32,
        0,
        0,
        (code.len() + TEXT.len()) as u32,
        0,
    ] {
        raw.extend(value.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(TEXT)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&script_bytes(code)).unwrap()
}

#[test]
fn each_service_uses_only_its_declared_capability_and_completes_once() {
    let mut cases: Vec<(u16, Vec<u32>, u32)> = vec![
        (0x20d0, vec![0x80000000], 0x80000000),
        (0x20d1, vec![5, 10, 2], 2),
        (0x20d2, vec![u32::MAX, 2], 2),
        (0x20d3, vec![u32::MAX, 2], u32::MAX),
        (0x20f1, vec![0, 4], 1),
        (0x20f3, vec![0x80000002, 0, 1, 2], 2),
        (0x20f8, vec![0], 3),
        (0x20fc, vec![0x80000003, 13, 12], 99),
        (0x212f, vec![], 99),
        (0x2130, vec![], 0),
        (0x2135, vec![0, 0], u32::MAX),
        (0x2136, vec![0, 4, 8, 11, 1, 2, 3, 4, 5, 6], 99),
    ];
    cases.extend((0x2131..=0x2134).map(|op| (op, vec![0, 0x80000002], 99)));
    cases.extend(
        [
            0x22d5, 0x22df, 0x22e0, 0x22e1, 0x22e2, 0x22e3, 0x22e4, 0x22e6, 0x22e7, 0x22e8, 0x22e9,
            0x22f8,
        ]
        .map(|op| (op, vec![1, u32::MAX], 99)),
    );
    cases.push((0x22de, vec![1, 0], 99));
    let entries: Vec<_> = dispatch::ROOT
        .iter()
        .filter(|e| {
            matches!(
                e.capability,
                Capability::Settings | Capability::History | Capability::Strings | Capability::Math
            )
        })
        .collect();
    assert_eq!(cases.len(), entries.len());
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
        vm.state_mut().set_system_integer(0, 99).unwrap();
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        let mut settings = SettingsHost::default();
        let mut history = HistoryHost::default();
        let mut hosts = Hosts {
            settings: (entry.capability == Capability::Settings).then_some(&mut settings),
            history: (entry.capability == Capability::History).then_some(&mut history),
            strings: entry.capability == Capability::Strings,
            math: entry.capability == Capability::Math,
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
            Some(Flow::Done),
            "0x{opcode:04x}"
        );
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.pc(), code.len());
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(result));
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            None
        );
        match opcode {
            0x20f3 => assert_eq!(vm.state().string(2), Some(b"bc".as_slice())),
            0x20fc => assert_eq!(vm.state().string(3), Some(b"mask012a.pb3".as_slice())),
            0x2131..=0x2134 => assert_eq!(vm.state().string(2), Some(b"".as_slice())),
            0x2136 => {
                assert_eq!(history.entries().len(), 1);
                assert_eq!(
                    history.entries()[0].strings,
                    [
                        b"abc".to_vec(),
                        b"ABC".to_vec(),
                        b"xy".to_vec(),
                        b"z".to_vec()
                    ]
                );
                assert_eq!(history.entries()[0].words, [1, 2, 3, 4, 5, 6]);
            }
            0x22de => assert_eq!(settings.font_face(), b"abc"),
            _ => {}
        }
    }
}

#[test]
fn setting_queries_preserve_the_difference_between_ignored_and_required_words() {
    for opcode in [
        0x22d5u16, 0x22de, 0x22df, 0x22e0, 0x22e1, 0x22e2, 0x22e3, 0x22e4, 0x22e6, 0x22e7, 0x22e8,
        0x22e9, 0x22f8,
    ] {
        let mut code = vec![];
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend(77u32.to_le_bytes());
        command(&mut code, opcode, &[0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 99).unwrap();
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut settings = SettingsHost::default();
        let original = settings.clone();
        let outcome = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                settings: Some(&mut settings),
                ..Default::default()
            },
        );
        if matches!(opcode, 0x22de | 0x22e1..=0x22e3) {
            assert_eq!(
                outcome.unwrap_err().as_decode().unwrap().reason,
                if opcode == 0x22de {
                    "font reference"
                } else {
                    "value"
                }
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(vm.state().system_integer(0), Some(99));
        } else {
            assert_eq!(outcome.unwrap(), Some(Flow::Done));
            assert_eq!(vm.stack().len_bytes(), 4);
            assert_eq!(vm.pending_system(), None);
        }
        assert_eq!(settings, original);
    }
}

#[test]
fn service_parameter_failures_preserve_vm_destination_and_hosts() {
    for (opcode, args, reason) in [
        (0x20d1, vec![1], "lower bound"),
        (0x20f1, vec![0], "second string"),
        (0x20f3, vec![0x80000002, 0, 0], "byte count"),
        (0x20f8, vec![], "string"),
        (0x20fc, vec![0x80000002, 13], "format value"),
        (0x2131, vec![0], "destination"),
        (0x2135, vec![0], "word index"),
        (0x2136, vec![0, 4, 8], "string 3"),
        (0x22de, vec![1], "font reference"),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 99).unwrap();
        vm.state_mut().set_string(2, b"original").unwrap();
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut settings = SettingsHost::default();
        let original = settings.clone();
        let mut history = HistoryHost::default();
        let error = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                settings: Some(&mut settings),
                history: Some(&mut history),
                strings: true,
                math: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.as_decode().unwrap().reason, reason);
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert_eq!(vm.state().string(2), Some(b"original".as_slice()));
        assert_eq!(settings, original);
        assert!(history.entries().is_empty());
    }
}

#[test]
fn trace_keeps_service_capabilities_and_math_support_unchanged() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-services-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let input = root.join("synthetic.ps3");
    for (opcode, args, log) in [
        (0x20d0u16, vec![7], "# math"),
        (0x20f8, vec![0], "# text 0x20f8: system_integer[0]=Some(3)"),
        (0x2130, vec![], "# history entries=0"),
        (0x22d5, vec![0, 0], "# configuration 0x22d5;"),
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
            let available = record && opcode != 0x20d0;
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

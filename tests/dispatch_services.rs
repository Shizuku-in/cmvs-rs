use cmvs_rs::{
    dispatch::{self, Flow, Hosts},
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

use cmvs_rs::{
    dispatch::{self, Flow, Hosts},
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
                assert_eq!(outcome.unwrap_err().as_decode().unwrap().reason, "value");
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
                scene: Some(&mut cmvs_rs::scene::SceneHost::default()),
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
        assert_eq!(gesture, original);
        assert!(title.bytes().is_empty());
        assert_eq!(info.values(), [None; 3]);
        assert!(effects && popup.popup_enabled());
        assert_eq!(popup.confirmations(), [false; 2]);
    }
}

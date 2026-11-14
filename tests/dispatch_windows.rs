use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts, InputHosts},
    image::ImageSource,
    input::ActionInput,
    machine::Machine,
    message::MessageHost,
    profile::Semantics,
    ps3::Script,
};
use std::error::Error;
#[allow(dead_code)]
mod support;
#[path = "support/window_cases.rs"]
mod window_cases;
use support::bytecode::{command, pending};
use window_cases::{cases, image_bytes, setup};

#[test]
fn hassaku_window_scale_preserves_raw_words_and_rejects_missing_targets_atomically() {
    let profile = cmvs_rs::profile::Registry::builtin()
        .profile("cmvs-20101211")
        .unwrap();
    let semantics = profile.semantics(0x18b);
    assert!(matches!(semantics, Semantics::Variant(_)));
    assert!(!dispatch::ROOT.iter().any(|entry| entry.command == 0x18b));
    for child in [u32::MAX, 16] {
        let mut windows = Windows::default();
        windows.setup();
        let mut code = vec![];
        command(&mut code, 0x218b, &[0, child, 0x8000_0000, 0x7fc0_1234]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        let before = windows.messages.window(0).unwrap().object.clone();
        assert_eq!(
            dispatch::run(semantics, &mut vm, &mut windows.hosts(Capability::Windows)).unwrap(),
            Some(Flow::Done)
        );
        let mut expected = before;
        let target = if child == u32::MAX {
            &mut expected
        } else {
            expected.children.get_mut(&16).unwrap()
        };
        target.draw.as_mut().unwrap().words[18..20].copy_from_slice(&[0x8000_0000, 0x7fc0_1234]);
        assert_eq!(windows.messages.window(0).unwrap().object, expected);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
    for args in [
        vec![1, u32::MAX, 1, 2],
        vec![0, 255, 1, 2],
        vec![0, 256, 1, 2],
        vec![0, u32::MAX, 1],
    ] {
        let mut windows = Windows::default();
        windows.setup();
        let mut code = vec![];
        command(&mut code, 0x218b, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = windows.snapshot();
        let pending = vm.pending_system();
        assert!(
            dispatch::run(semantics, &mut vm, &mut windows.hosts(Capability::Windows)).is_err()
        );
        assert_eq!(windows.snapshot(), before);
        assert_eq!(vm.pending_system(), pending);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

#[test]
fn natsuuta_window_position_and_opacity_tracks_reuse_the_scene_tracks() {
    use cmvs_rs::scene::{OpacityKey, PositionKey};
    let profile = cmvs_rs::profile::Registry::builtin()
        .profile("cmvs-20100314")
        .unwrap();
    for command in [0x1ca, 0x1cb, 0x1cc, 0x1d4, 0x1d5, 0x1d6] {
        assert!(matches!(profile.semantics(command), Semantics::Variant(_)));
    }
    // HASSAKU binds neither family.
    let hassaku = cmvs_rs::profile::Registry::builtin()
        .profile("cmvs-20101211")
        .unwrap();
    assert_eq!(hassaku.semantics(0x1d4), Semantics::Root);
    assert_eq!(hassaku.semantics(0x1ca), Semantics::Root);
    for child in [u32::MAX, 16] {
        let mut windows = Windows::default();
        windows.setup();
        let mut code = vec![];
        let x = (-30i32) as u32;
        command(&mut code, 0x21d4, &[0, child]);
        command(&mut code, 0x21d5, &[0, child, 2, u32::MAX, 0, 100, x, 40]);
        command(&mut code, 0x21d6, &[0, child, 2]);
        command(&mut code, 0x21ca, &[0, child, 300]);
        command(
            &mut code,
            0x21cb,
            &[0, child, 1, u32::MAX, 5, 100, (-260i32) as u32],
        );
        command(&mut code, 0x21cc, &[0, child, 1]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        for op in [0x1d4, 0x1d5, 0x1d6, 0x1ca, 0x1cb, 0x1cc] {
            pending(&mut vm);
            assert_eq!(
                dispatch::run(
                    profile.semantics(op),
                    &mut vm,
                    &mut windows.hosts(Capability::Windows)
                )
                .unwrap(),
                Some(Flow::Done)
            );
            assert_eq!(vm.stack().len_bytes(), 0);
        }
        assert_eq!(vm.state().system_integer(0), Some(77));
        let object = &windows.messages.window(0).unwrap().object;
        let target = if child == u32::MAX {
            object
        } else {
            &object.children[&16]
        };
        let draw = target.draw.as_ref().unwrap();
        let motion = draw.position_motion.as_ref().unwrap();
        assert_eq!(
            motion.keys[2],
            PositionKey {
                next: -1,
                xy: [-30, 40],
                mode: 0,
                duration: 100,
            }
        );
        let playback = motion.playback.as_ref().unwrap();
        assert_eq!((playback.start, playback.current), (2, 2));
        let track = draw.opacity_track.as_ref().unwrap();
        assert_eq!(track.base, 44);
        assert!(track.playing);
        assert_eq!(
            track.keys[1],
            OpacityKey {
                next: -1,
                value: 4,
                hold: 5,
                duration: 100,
            }
        );
    }
    // Slots outside 0..3, missing children and keys outside the tracks fail atomically.
    for (op, args) in [
        (0x21d4, vec![4, u32::MAX]),
        (0x21d4, vec![0, 17]),
        (0x21ca, vec![4, u32::MAX, 0]),
        (0x21cb, vec![0, u32::MAX, 8, 0, 0, 0, 0]),
    ] {
        let mut windows = Windows::default();
        windows.setup();
        let mut code = vec![];
        if op == 0x21cb {
            command(&mut code, 0x21ca, &[0, u32::MAX, 0]);
        }
        command(&mut code, op, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        if op == 0x21cb {
            pending(&mut vm);
            dispatch::run(
                profile.semantics(0x1ca),
                &mut vm,
                &mut windows.hosts(Capability::Windows),
            )
            .unwrap();
        }
        pending(&mut vm);
        let before = windows.snapshot();
        let pending = vm.pending_system();
        assert!(
            dispatch::run(
                profile.semantics(op - 0x2000),
                &mut vm,
                &mut windows.hosts(Capability::Windows)
            )
            .is_err()
        );
        assert_eq!(windows.snapshot(), before);
        assert_eq!(vm.pending_system(), pending);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

fn bytes(code: &[u8]) -> Vec<u8> {
    let text = b"\0a.pb3\0";
    let mut raw = b"PS2A".to_vec();
    for word in [
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
        raw.extend(word.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(text)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&bytes(code)).unwrap()
}

#[derive(Default)]
struct Images {
    reads: usize,
    fail: bool,
}
impl ImageSource for Images {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        assert_eq!(name, b"a.pb3");
        self.reads += 1;
        if self.fail {
            return Err(
                std::io::Error::new(std::io::ErrorKind::NotFound, "image unavailable").into(),
            );
        }
        Ok(Some(image_bytes()))
    }
}
#[derive(Default)]
struct Windows {
    messages: MessageHost,
    input: ActionInput,
    images: Images,
}
impl Windows {
    fn setup(&mut self) {
        let setup = setup();
        let mut code = vec![];
        for (op, args) in &setup {
            command(&mut code, *op, args);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        self.messages.set_record_output(true);
        self.input.set_pointer(20, 30);
        for _ in &setup {
            pending(&mut vm);
            if dispatch::uses_windows(vm.pending_system().unwrap().1) {
                assert_eq!(
                    dispatch::run(
                        Semantics::Root,
                        &mut vm,
                        &mut self.hosts(Capability::Windows)
                    )
                    .unwrap(),
                    Some(Flow::Done)
                );
            } else {
                assert!(self.messages.try_handle(&mut vm).unwrap());
            }
        }
    }
    fn hosts(&mut self, capability: Capability) -> Hosts<'_> {
        Hosts {
            messages: Some(&mut self.messages),
            input: matches!(
                capability,
                Capability::WindowPointer | Capability::WindowButtons
            )
            .then_some(InputHosts {
                input: &mut self.input,
                skip: false,
                clock: None,
            }),
            window_images: (capability == Capability::WindowImages)
                .then_some(&mut self.images as &mut dyn ImageSource),
            ..Default::default()
        }
    }
    fn snapshot(&self) -> String {
        format!("{:?}{:?}", self.messages, self.input)
    }
}

#[test]
fn missing_inputs_sources_and_short_stacks_preserve_window_and_input_state() {
    for (opcode, args, capability) in cases() {
        for partial in [false, true] {
            let mut windows = Windows::default();
            windows.setup();
            let supplied = if partial { args.len() - 1 } else { 0 };
            let mut code = vec![];
            command(&mut code, opcode, &args[..supplied]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let state = windows.snapshot();
            let error = dispatch::run(Semantics::Root, &mut vm, &mut windows.hosts(capability))
                .unwrap_err();
            assert_eq!(error.as_decode().unwrap().opcode, Some(opcode));
            assert_eq!((error.pc, error.command), (before.0, opcode - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(windows.snapshot(), state);
            assert_eq!(windows.images.reads, 0);
        }
        if capability == Capability::Windows {
            continue;
        }
        for missing_window in [false, true] {
            let mut windows = Windows::default();
            windows.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let state = windows.snapshot();
            let mut hosts = windows.hosts(capability);
            if missing_window {
                hosts.messages = None;
            } else if capability == Capability::WindowImages {
                hosts.window_images = None;
            } else {
                hosts.input = None;
            }
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(windows.snapshot(), state);
            assert_eq!(windows.images.reads, 0);
        }
    }
}

#[test]
fn image_errors_keep_typed_cause_and_leave_window_targets_retryable() {
    for opcode in [0x2170, 0x2397] {
        let mut windows = Windows::default();
        windows.setup();
        windows.images.fail = true;
        let mut code = vec![];
        command(&mut code, opcode, &[0, 1]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let state = windows.snapshot();
        let error = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut windows.hosts(Capability::WindowImages),
        )
        .unwrap_err();
        assert_eq!(
            error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            error.to_string(),
            format!("PC=0x{:x} 0x{opcode:04x}: image unavailable", before.0)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(windows.snapshot(), state);
        windows.images.fail = false;
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut windows.hosts(Capability::WindowImages)
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(windows.images.reads, 2);
    }
}

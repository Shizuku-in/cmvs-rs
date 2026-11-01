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
            windows: Some(&mut self.messages),
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
fn exact_window_batch_completes_with_only_the_required_context() {
    for (capability, count) in [
        (Capability::Windows, 46),
        (Capability::WindowPointer, 1),
        (Capability::WindowButtons, 1),
        (Capability::WindowImages, 2),
    ] {
        let expected: std::collections::BTreeSet<_> = cases()
            .iter()
            .filter(|(_, _, cap)| *cap == capability)
            .map(|(op, _, _)| op - 0x2000)
            .collect();
        let actual = dispatch::ROOT
            .iter()
            .filter(|e| e.capability == capability)
            .map(|e| e.command)
            .collect();
        assert_eq!(expected.len(), count);
        assert_eq!(expected, actual);
    }
    assert!(!dispatch::uses_windows(0x383));
    for (opcode, args, capability) in cases() {
        let mut windows = Windows::default();
        windows.setup();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let state = windows.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(
                Semantics::ResearchFontCells,
                &mut vm,
                &mut windows.hosts(capability)
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(windows.snapshot(), state);
        assert_eq!(windows.images.reads, 0);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut windows.hosts(capability)).unwrap(),
            Some(Flow::Done),
            "{opcode:04x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let state = windows.snapshot();
        let reads = windows.images.reads;
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut windows.hosts(capability)).unwrap(),
            None
        );
        assert_eq!(windows.snapshot(), state);
        assert_eq!(windows.images.reads, reads);
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
                hosts.windows = None;
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

#[test]
fn trace_still_has_no_window_capability_for_any_batch_command() {
    use std::{fs, process::Command};
    let root =
        std::env::temp_dir().join(format!("cmvs-cli-dispatch-windows-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("windows.ps3");
    for (opcode, args, _) in cases() {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        command(&mut code, 0x2003, &[]);
        let raw = bytes(&code);
        fs::write(&path, &raw).unwrap();
        for record in [false, true] {
            let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
            cli.arg(&path).args(["--profile", "cmvs-20160201"]);
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
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{opcode:04x};")),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

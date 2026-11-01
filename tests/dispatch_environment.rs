use cmvs_rs::{
    bootstrap::Bootstrap,
    capture::ScreenCapture,
    dispatch::{self, Capability, Flow, FunctionKeys, Hosts, PlatformHosts},
    input::ActionInput,
    machine::Machine,
    message::MessageHost,
    platform::{PlatformAssets, PlatformEvent, PlatformHost, RecordingBackend},
    profile::Semantics,
    ps3::Script,
    raster::RgbaImage,
    scene::SceneHost,
    settings::SettingsHost,
};
use std::error::Error;
#[path = "support/environment_cases.rs"]
mod environment_cases;
#[allow(dead_code)]
mod support;
use environment_cases::cases;
use support::bytecode::{command, pending};
fn bytes(code: &[u8]) -> Vec<u8> {
    let text = b"\0name\0icon.ico\0cursor.cur\0";
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

#[derive(Debug, Default)]
struct Assets {
    reads: usize,
    fail: bool,
}
impl PlatformAssets for Assets {
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        self.reads += 1;
        if self.fail {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "platform unavailable",
            )
            .into());
        }
        Ok(if name == b"icon.ico" {
            environment_cases::ico()
        } else {
            environment_cases::cur()
        })
    }
}
#[derive(Default)]
struct State {
    bootstrap: Bootstrap,
    platform: PlatformHost,
    capture: ScreenCapture,
    scene: SceneHost,
    backend: RecordingBackend,
    assets: Assets,
    messages: MessageHost,
    settings: SettingsHost,
    input: ActionInput,
}
impl State {
    fn setup(&mut self) {
        let mut code = vec![];
        for (op, args) in [(0x2020, vec![11]), (0x2040, vec![11])] {
            command(&mut code, op, &args);
        }
        let scene_script = script(&code);
        let mut vm = Machine::new(&scene_script).unwrap();
        for _ in 0..2 {
            pending(&mut vm);
            assert!(self.scene.try_handle(&mut vm).unwrap());
        }
        let mut code = vec![];
        command(&mut code, 0x2098, &[15]);
        command(&mut code, 0x2160, &[1]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert!(self.bootstrap.try_handle(&mut vm).unwrap().is_some());
        pending(&mut vm);
        assert!(
            self.platform
                .try_handle(&mut vm, &mut self.backend, &mut self.assets, &[])
                .unwrap()
                .is_some()
        );
        let image = RgbaImage::new(2, 1, vec![1, 2, 3, 0, 4, 5, 6, 128]).unwrap();
        self.capture.take(image.clone()).unwrap();
        self.backend.present_frame(image);
        self.backend.answer_font_dialog(None);
    }
    fn hosts(&mut self, cap: Capability) -> Hosts<'_> {
        let mut h = Hosts::default();
        match cap {
            Capability::Bootstrap => h.bootstrap = Some(&mut self.bootstrap),
            Capability::FunctionKeys => {
                h.function_keys = Some(FunctionKeys::Startup(&mut self.bootstrap))
            }
            Capability::Platform => {
                h.platform = Some(PlatformHosts {
                    host: &mut self.platform,
                    backend: &mut self.backend,
                    assets: &mut self.assets,
                    cursors: self.bootstrap.cursor_names(),
                })
            }
            Capability::Capture => {
                h.capture = Some(&mut self.capture);
                h.scene = Some(&mut self.scene);
            }
            Capability::CaptureDevice => {
                h.capture = Some(&mut self.capture);
                h.scene = Some(&mut self.scene);
                h.device_frame = Some(&mut self.backend);
            }
            Capability::FontDialog => {
                h.messages = Some(&mut self.messages);
                h.settings = Some(&mut self.settings);
                h.device_frame = Some(&mut self.backend);
            }
            _ => unreachable!(),
        }
        h
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            self.bootstrap,
            self.capture,
            self.scene,
            self.backend.requests(),
            self.backend.font_dialogs(),
            self.assets,
            self.messages,
            self.settings,
            self.input,
            self.platform.title(),
            (
                self.platform.scene_title(),
                self.platform.cursor(),
                self.platform.icon(),
                self.platform.thumbnail()
            )
        )
    }
}
fn expected(opcode: u16) -> Flow {
    match opcode {
        0x2164 | 0x216e => Flow::Platform(PlatformEvent::Yield),
        0x208f | 0x2099 | 0x2160 | 0x2161 => Flow::Platform(PlatformEvent::Applied),
        0x2163 | 0x216b..=0x216d | 0x216f | 0x22b0 | 0x22c8..=0x22ca | 0x22fc | 0x22fd | 0x22ff => {
            Flow::Platform(PlatformEvent::Configured)
        }
        _ => Flow::Done,
    }
}
#[test]
fn environment_batch_has_exact_owners_and_only_completes_once() {
    assert_eq!(cases().len(), 43);
    for (opcode, args, cap) in cases() {
        let entries: Vec<_> = dispatch::ROOT
            .iter()
            .filter(|e| e.command == opcode - 0x2000)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].capability, cap);
        let mut state = State::default();
        state.setup();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(Semantics::ResearchFontCells, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(state.snapshot(), snapshot);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(expected(opcode)),
            "{opcode:x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            None
        );
        assert_eq!(state.snapshot(), snapshot);
    }
}
#[test]
fn short_environment_arguments_preserve_state_and_have_named_errors() {
    for (opcode, args, cap) in cases().into_iter().filter(|(_, args, _)| !args.is_empty()) {
        for len in [0, args.len() - 1] {
            let mut state = State::default();
            state.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args[..len]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let snapshot = state.snapshot();
            let vars = format!("{:?}", vm.state());
            let e = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap_err();
            assert_eq!((e.pc, e.command), (before.0, opcode - 0x2000));
            assert_eq!(e.as_decode().unwrap().opcode, Some(opcode));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
            assert_eq!(format!("{:?}", vm.state()), vars);
        }
    }
}
#[test]
fn partial_capture_and_font_contexts_never_invoke_the_device() {
    for (opcode, args, cap) in cases().into_iter().filter(|(_, _, cap)| {
        matches!(
            cap,
            Capability::FontDialog | Capability::Capture | Capability::CaptureDevice
        )
    }) {
        for missing in 0..3 {
            let mut state = State::default();
            state.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let snapshot = state.snapshot();
            let mut h = state.hosts(cap);
            if cap == Capability::FontDialog {
                match missing {
                    0 => h.messages = None,
                    1 => h.settings = None,
                    _ => h.device_frame = None,
                }
            } else {
                if missing == 2 && cap == Capability::Capture {
                    continue;
                }
                match missing {
                    0 => h.capture = None,
                    1 => h.scene = None,
                    _ => h.device_frame = None,
                }
            }
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut h).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
        }
    }
}
#[test]
fn function_key_policy_is_unique_explicit_and_preserves_each_environment() {
    for report in [false, true] {
        let mut bootstrap = Bootstrap::default();
        let mut input = ActionInput::default();
        let mut code = vec![];
        command(&mut code, 0x234d, &[]);
        command(&mut code, 0x234e, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    function_keys: Some(if report {
                        FunctionKeys::Input(&mut input)
                    } else {
                        FunctionKeys::Startup(&mut bootstrap)
                    }),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        input.press_function_keys(7);
        pending(&mut vm);
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    input: Some(dispatch::InputHosts {
                        input: &mut input,
                        skip: false,
                        clock: None
                    }),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(
            vm.state().system_integer(0),
            Some(if report { 1 } else { 0 })
        );
        assert_eq!(
            bootstrap.settings().get(&0x234d).copied(),
            if report { None } else { Some(1) }
        );
    }
}
#[test]
fn platform_io_failures_keep_original_cause_and_retry_without_committing_host() {
    for opcode in [0x208f, 0x2099] {
        let mut state = State::default();
        state.setup();
        state.assets.fail = true;
        let mut code = vec![];
        command(&mut code, opcode, &[if opcode == 0x208f { 6 } else { 1 }]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let requests = state.backend.requests().to_vec();
        let e = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut state.hosts(Capability::Platform),
        )
        .unwrap_err();
        assert_eq!(
            e.source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(e.to_string(), "platform unavailable");
        assert_eq!(state.backend.requests(), requests);
        assert_eq!(state.platform.cursor(), 0);
        assert_eq!(state.platform.icon(), None);
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        state.assets.fail = false;
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut state.hosts(Capability::Platform)
            )
            .unwrap(),
            Some(Flow::Platform(PlatformEvent::Applied))
        );
        assert_eq!(state.assets.reads, 2);
    }
}

#[test]
fn trace_environment_capabilities_follow_bootstrap_and_recording_flags() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-environment-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("icon.ico"), environment_cases::ico()).unwrap();
    let path = root.join("environment.ps3");
    for (opcode, mut args, cap) in cases() {
        if opcode == 0x2099 {
            args[0] = 0;
        }
        for record in [false, true] {
            for bootstrap in [false, true] {
                let mut code = vec![];
                if record && opcode == 0x2161 {
                    command(&mut code, 0x2160, &[1]);
                }
                command(&mut code, opcode, &args);
                command(&mut code, 0x2003, &[]);
                let raw = bytes(&code);
                fs::write(&path, &raw).unwrap();
                let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
                cli.arg(&path).args(["--profile", "cmvs-20160201"]);
                if record {
                    cli.arg("--record-platform").arg(&root);
                }
                if bootstrap {
                    cli.arg("--bootstrap");
                }
                let out = cli.output().unwrap();
                let stdout = String::from_utf8(out.stdout).unwrap();
                assert!(
                    out.status.success(),
                    "{opcode:x} {record} {bootstrap}: {stdout}\n{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let enabled = match cap {
                    Capability::Bootstrap => bootstrap,
                    Capability::FunctionKeys => record || bootstrap,
                    Capability::Platform => record,
                    _ => false,
                };
                let stop = if enabled { 0x2003 } else { opcode };
                assert!(
                    stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                    "{stdout}"
                );
                assert_eq!(fs::read(&path).unwrap(), raw);
            }
        }
    }
    fs::remove_file(path).unwrap();
    fs::remove_file(root.join("icon.ico")).unwrap();
    fs::remove_dir(root).unwrap();
}

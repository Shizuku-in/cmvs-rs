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
                h.function_keys = Some(FunctionKeys::Runtime(&mut self.bootstrap))
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
#[test]
fn partial_capture_and_font_contexts_never_invoke_the_device() {
    for (opcode, args, cap) in [
        (0x213d, vec![], Capability::FontDialog),
        (0x203c, vec![11, u32::MAX], Capability::Capture),
        (0x2304, vec![], Capability::Capture),
        (0x2305, vec![], Capability::Capture),
        (0x2306, vec![], Capability::Capture),
        (0x203e, vec![11, u32::MAX], Capability::CaptureDevice),
    ] {
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
                        FunctionKeys::Runtime(&mut bootstrap)
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
fn function_key_public_apis_preserve_both_explicit_policy_effects() {
    // Compatibility APIs retain both effects. ROOT chooses through an explicit
    // Runtime/Input context, covered separately by the environment batch tests.
    let mut code = vec![];
    command(&mut code, 0x234d, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut bootstrap = cmvs_rs::bootstrap::Bootstrap::default();
    assert!(bootstrap.try_handle(&mut vm).unwrap().is_some());
    assert_eq!(bootstrap.settings().get(&0x234d), Some(&1));
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut input = cmvs_rs::input::ActionInput::default();
    assert!(cmvs_rs::input::try_handle(&mut vm, &mut input).unwrap());
    input.press_function_keys(1);
    let mut code = vec![];
    command(&mut code, 0x234e, &[]);
    let query = support::bytecode::script(&code);
    let mut vm = Machine::new(&query).unwrap();
    pending(&mut vm);
    assert!(cmvs_rs::input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
}

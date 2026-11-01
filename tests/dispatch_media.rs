use cmvs_rs::{
    audio::RecordingAudio,
    camera::CameraHost,
    dispatch::{self, Capability, Flow, Hosts, MoviePlayerHosts, MovieSceneHosts, SequenceHosts},
    image::ImageSource,
    machine::Machine,
    movie::{MovieHost, MovieSource, OpenedMovie},
    profile::Semantics,
    ps3::Script,
    resources::ResourceSource,
    scene::SceneHost,
    sequence::{SequenceContext, SequenceHost},
};
use std::error::Error;
#[allow(dead_code)]
#[path = "support/cmv9.rs"]
mod cmv9;
#[path = "support/media_cases.rs"]
mod media_cases;
#[allow(dead_code)]
mod support;
#[allow(dead_code)]
#[path = "support/window_cases.rs"]
mod window_cases;
use media_cases::cases;
use support::bytecode::{command, pending};
fn bytes(code: &[u8]) -> Vec<u8> {
    let text = b"a.cmv\0";
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

#[derive(Default, Debug)]
struct Files {
    reads: usize,
    fail: bool,
}
impl Files {
    fn read_data(&mut self, data: Vec<u8>) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.reads += 1;
        if self.fail {
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "media unavailable").into())
        } else {
            Ok(Some(data))
        }
    }
}
impl ResourceSource for Files {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.read_data(cmv9::movie_frames(4, 15, 1, false))
    }
}
impl ImageSource for Files {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.read_data(window_cases::image_bytes())
    }
}
impl MovieSource for Files {
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>> {
        Ok(self
            .read_data(media_cases::cmv(4, 15, 1, true))?
            .map(|bytes| {
                (
                    name.to_vec(),
                    cmvs_rs::cmv::CmvIndex::read(&mut std::io::Cursor::new(bytes)).unwrap(),
                )
            }))
    }
}
#[derive(Default)]
struct State {
    movies: MovieHost,
    scene: SceneHost,
    sequence: SequenceHost,
    camera: CameraHost,
    source: Files,
    audio: RecordingAudio,
}
impl State {
    fn hosts(&mut self, cap: Capability) -> Hosts<'_> {
        let mut hosts = Hosts::default();
        match cap {
            Capability::Sequence => {
                hosts.sequence = Some(SequenceHosts {
                    host: &mut self.sequence,
                    execution: Some(SequenceContext {
                        scene: &mut self.scene,
                        images: &mut self.source,
                        camera: Some(&mut self.camera),
                    }),
                })
            }
            Capability::MovieState => hosts.movies = Some(&mut self.movies),
            Capability::MovieStart => {
                hosts.movies = Some(&mut self.movies);
                hosts.scene_movie_playback = true;
            }
            Capability::MovieScene => {
                hosts.movies = Some(&mut self.movies);
                hosts.movie_scene = Some(MovieSceneHosts {
                    scene: &mut self.scene,
                    source: &mut self.source,
                });
            }
            Capability::MoviePlayer => {
                hosts.movies = Some(&mut self.movies);
                hosts.movie_player = Some(MoviePlayerHosts {
                    source: &mut self.source,
                    settings: Default::default(),
                    backend: &mut self.audio,
                });
            }
            _ => unreachable!(),
        }
        hosts
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?}",
            self.movies,
            self.scene,
            self.sequence,
            self.camera,
            self.source,
            self.audio.requests()
        )
    }
}
#[test]
fn media_commands_require_their_own_capability_and_complete_once() {
    assert_eq!(cases().len(), 20);
    for (opcode, args, cap) in cases() {
        let entries: Vec<_> = dispatch::ROOT
            .iter()
            .filter(|e| e.command == opcode - 0x2000)
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].capability, cap);
        let mut state = State::default();
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
            Some(Flow::Done),
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
fn missing_media_dependencies_and_short_arguments_preserve_pending_state() {
    for (opcode, args, cap) in cases() {
        for len in [0, args.len().saturating_sub(1)] {
            if args.is_empty() {
                break;
            }
            let mut state = State::default();
            let mut code = vec![];
            command(&mut code, opcode, &args[..len]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let snapshot = state.snapshot();
            let e = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap_err();
            assert_eq!((e.pc, e.command), (before.0, opcode - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
        }
        if !matches!(
            cap,
            Capability::MovieScene | Capability::MovieStart | Capability::MoviePlayer
        ) {
            continue;
        }
        for missing_host in [false, true] {
            let mut state = State::default();
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let snapshot = state.snapshot();
            let mut hosts = state.hosts(cap);
            if missing_host {
                hosts.movies = None;
            } else {
                hosts.movie_scene = None;
                hosts.movie_player = None;
                hosts.scene_movie_playback = false;
            }
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
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
fn movie_source_failures_keep_typed_cause_and_allow_retry() {
    for (opcode, args, cap) in cases()
        .into_iter()
        .filter(|(op, _, _)| matches!(op, 0x2308 | 0x20c0))
    {
        let mut state = State::default();
        state.source.fail = true;
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let snapshot = format!(
            "{:?} {:?} {:?}",
            state.movies,
            state.scene,
            state.audio.requests()
        );
        let e = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap_err();
        assert_eq!(
            e.source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(e.to_string(), "media unavailable");
        assert_eq!((e.pc, e.command), (before.0, opcode - 0x2000));
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(
            format!(
                "{:?} {:?} {:?}",
                state.movies,
                state.scene,
                state.audio.requests()
            ),
            snapshot
        );
        state.source.fail = false;
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(state.source.reads, 2);
    }
}
#[test]
fn sequence_finish_retains_successful_prefix_and_retries_failed_action() {
    let mut state = State::default();
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![11]),
        (0x2040, vec![11]),
        (0x2030, vec![11, 0]),
        (0x2286, vec![0]),
        (
            0x2289,
            vec![11, 10f32.to_bits(), 20f32.to_bits(), 30f32.to_bits(), 0, 0],
        ),
        (0x2288, vec![11, 0, 32]),
        (0x2286, vec![10]),
        (0x2282, vec![]),
        (0x2284, vec![]),
    ] {
        command(&mut code, op, &args);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    for _ in 0..2 {
        pending(&mut vm);
        assert!(state.scene.try_handle(&mut vm).unwrap());
    }
    pending(&mut vm);
    assert_eq!(
        dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                images: Some(dispatch::ImageHosts {
                    scene: &mut state.scene,
                    source: &mut state.source,
                    change_images: true
                }),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    state.source.reads = 0;
    for _ in 0..5 {
        pending(&mut vm);
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut state.hosts(Capability::Sequence)
            )
            .unwrap(),
            Some(Flow::Done)
        );
    }
    pending(&mut vm);
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    state.source.fail = true;
    let e = dispatch::run(
        Semantics::Root,
        &mut vm,
        &mut state.hosts(Capability::Sequence),
    )
    .unwrap_err();
    assert_eq!(
        e.source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(
        &state.scene.object(11).unwrap().draw.as_ref().unwrap().words[11..14],
        &[10f32.to_bits(), 20f32.to_bits(), 30f32.to_bits()]
    );
    assert_eq!(state.sequence.header().index, 2);
    assert_eq!(state.sequence.header().active, 1);
    assert_eq!(
        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
        before
    );
    state.source.fail = false;
    assert_eq!(
        dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut state.hosts(Capability::Sequence)
        )
        .unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(state.sequence.header(), Default::default());
    assert!(state.sequence.entries()[2].is_some());
    assert_eq!(state.source.reads, 2);
    assert!(state.scene.object(11).unwrap().image.is_some());
}

#[test]
fn movie_commands_ignore_only_the_unused_parameter_values() {
    for create in [true, false] {
        let mut code = vec![];
        // Reserve the unused word without assigning it a value.
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend((if create { 0u32 } else { u32::MAX }).to_le_bytes());
        if !create {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(u32::MAX.to_le_bytes());
        }
        for value in if create { [0u32, 11, 0] } else { [0u32, 0, 0] } {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend((if create { 0x2308u16 } else { 0x2309 }).to_le_bytes());
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut state = State::default();
        let cap = if create {
            Capability::MovieScene
        } else {
            Capability::MovieStart
        };
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), if create { 0 } else { 4 });
    }
}

#[test]
fn trace_preserves_media_capabilities_and_stops() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-dispatch-media-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("media.ps3");
    for (opcode, args, cap) in cases() {
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
            if record && opcode == 0x20c0 {
                assert!(!output.status.success());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("movie directory 0x2015 was not registered")
                );
                continue;
            }
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                stdout.contains(&format!(
                    "# STOP pending system=0x{:04x};",
                    if record
                        && matches!(
                            cap,
                            Capability::Sequence | Capability::MovieState | Capability::MoviePlayer
                        )
                    {
                        0x2003
                    } else {
                        opcode
                    }
                )),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

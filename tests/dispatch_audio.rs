use cmvs_rs::{
    audio::{AudioBackend, AudioHost, AudioRequest, RecordingAudio, VoiceDuckingSettings},
    dispatch::{self, AudioHosts, Capability, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
};
use std::error::Error;
mod support;
use support::bytecode::{command, pending, script};

fn cases() -> Vec<(u16, Vec<u32>)> {
    let name = 0x80000000;
    let mut cases = vec![
        (0x20a0, vec![name, 0, 1]),
        (0x20a1, vec![]),
        (0x20a2, vec![20]),
        (0x20a4, vec![name, name, 20, 1]),
        (0x20a6, vec![0, name, 256, 1]),
        (0x20a8, vec![0]),
        (0x20a9, vec![0, 20]),
        (0x20aa, vec![0]),
        (0x20af, vec![0, name, 256, 1, 2f32.to_bits()]),
        (0x20b0, vec![0, name, 256, 1, 1]),
        (0x20b1, vec![0, 1]),
        (0x20b3, vec![0]),
        (0x20b5, vec![]),
        (0x20b6, vec![0, 20]),
        (0x20b7, vec![0]),
        (0x20ba, vec![0, name, 256, 20, 1, 1]),
        (0x2241, vec![0]),
    ];
    cases.extend((0x2238..=0x223f).map(|op| (op, if op % 2 == 0 { vec![1] } else { vec![] })));
    cases.extend((0x22d6..=0x22dd).map(|op| (op, vec![1, 0])));
    cases
}

fn run(
    vm: &mut Machine<'_>,
    audio: &mut AudioHost,
    backend: &mut dyn AudioBackend,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    dispatch::run(
        Semantics::Root,
        vm,
        &mut Hosts {
            audio: Some(AudioHosts { audio, backend }),
            ..Default::default()
        },
    )
}

#[test]
fn all_audio_entries_require_the_pair_and_complete_once() {
    let cases = cases();
    let expected: std::collections::BTreeSet<_> = cases.iter().map(|(op, _)| op - 0x2000).collect();
    let actual = dispatch::ROOT
        .iter()
        .filter(|e| e.capability == Capability::Audio)
        .map(|e| e.command)
        .collect();
    assert_eq!(expected, actual);
    assert_eq!(expected.len(), 33);
    for (op, args) in cases {
        let mut code = vec![];
        command(&mut code, op, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"a.ogg").unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        let mut audio = AudioHost::default();
        let mut backend = RecordingAudio::default();
        assert_eq!(
            dispatch::run(
                Semantics::Research(cmvs_rs::profile::ResearchId::FontCells),
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut audio,
                        backend: &mut backend
                    }),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert!(backend.requests().is_empty());
        assert_eq!(
            run(&mut vm, &mut audio, &mut backend).unwrap(),
            Some(Flow::Done),
            "{op:04x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let requests = backend.requests().to_vec();
        assert_eq!(run(&mut vm, &mut audio, &mut backend).unwrap(), None);
        assert_eq!(backend.requests(), requests);
    }
}

struct Reject;
impl AudioBackend for Reject {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        unreachable!()
    }
    fn configure_voice_ducking(&mut self, _: VoiceDuckingSettings) -> Result<(), Box<dyn Error>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "device rejected configuration",
        )
        .into())
    }
}

#[test]
fn errors_keep_location_typed_source_and_retry_state() {
    let mut code = vec![];
    command(&mut code, 0x2238, &[50]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    let mut audio = AudioHost::default();
    let original = audio.voice_ducking();
    let error = run(&mut vm, &mut audio, &mut Reject).unwrap_err();
    assert_eq!((error.pc, error.command), (before.0, 0x238));
    assert!(error.as_decode().is_none());
    assert_eq!(error.to_string(), "device rejected configuration");
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
        before
    );
    assert_eq!(audio.voice_ducking(), original);
    assert_eq!(
        run(&mut vm, &mut audio, &mut RecordingAudio::default()).unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(audio.voice_ducking().percent, 50);

    let mut code = vec![];
    command(&mut code, 0x2238, &[]);
    let script = support::bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let error = run(&mut vm, &mut audio, &mut Reject).unwrap_err();
    let decode = error.as_decode().unwrap();
    assert_eq!(decode.reason, "value");
    assert_eq!(decode.opcode, Some(0x2238));
    assert_eq!(decode.to_string(), error.to_string());
    assert_eq!(error.source().unwrap().downcast_ref(), Some(decode));
    assert_eq!(audio.voice_ducking().percent, 50);
}

#[test]
fn audio_queries_only_ignore_unused_use_flag_values() {
    for opcode in 0x22d6..=0x22dd {
        let mut code = vec![];
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend(77u32.to_le_bytes());
        command(&mut code, opcode, &[0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let result = run(
            &mut vm,
            &mut AudioHost::default(),
            &mut RecordingAudio::default(),
        );
        if opcode <= 0x22d8 {
            assert_eq!(result.unwrap(), Some(Flow::Done));
            assert_eq!(vm.state().system_integer(0), Some(1));
            assert_eq!(vm.stack().len_bytes(), before.2 - 8);
        } else {
            assert_eq!(result.unwrap_err().as_decode().unwrap().reason, "value");
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
        }
    }
}

#[test]
fn trace_preserves_optional_audio_and_stop_diagnostics() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-dispatch-audio-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("audio.ps3");
    for (opcode, mut args) in cases() {
        for arg in &mut args {
            if *arg == 0x80000000 {
                *arg = 0;
            }
        }
        let mut code = vec![];
        command(&mut code, opcode, &args);
        command(&mut code, 0x2003, &[]);
        let text = b"a.ogg\0";
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
            let stop = if record { 0x2003 } else { opcode };
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                "{stdout}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), raw);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

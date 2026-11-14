use cmvs_rs::{
    audio::{AudioBackend, AudioHost, AudioRequest, RecordingAudio, VoiceDuckingSettings},
    dispatch::{self, AudioHosts, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
};
use std::error::Error;
mod support;
use support::bytecode::{command, pending, script};

#[test]
fn cvns_master_percentage_reaches_the_mixer_with_integer_rounding_and_clamping() {
    for (percent, gain) in [(0, 0), (1, 2), (50, 128), (100, 256), (u32::MAX, 256)] {
        let mut code = vec![];
        command(&mut code, 0x24ac, &[percent]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut audio = AudioHost::default();
        assert!(run(&mut vm, &mut audio, &mut Reject).is_err());
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );

        let mut backend = RecordingAudio::default();
        assert_eq!(
            run(&mut vm, &mut audio, &mut backend).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(backend.requests(), &[AudioRequest::SetMixerGain { gain }]);
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!((vm.pending_system(), vm.stack().len_bytes()), (None, 0));
    }
}

#[test]
fn preloaded_playback_obeys_se_gate_and_keeps_results_and_normal_channels() {
    use cmvs_rs::{audio::AudioSettings, profile::Registry};
    let profile = Registry::builtin().profile("cmvs-20071117").unwrap();
    for enabled in [false, true] {
        for (scale, volume) in [(0, 128), (u32::MAX, 128), (256, 128), (512, 256)] {
            let mut code = vec![];
            command(&mut code, 0x224a, &[23, scale, 7]);
            command(&mut code, 0x224b, &[23]);
            command(&mut code, 0x224f, &[23]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            vm.state_mut().set_system_integer(0, 77).unwrap();
            let mut audio = AudioHost::new(AudioSettings {
                se_enabled: enabled,
                se: 128,
                ..Default::default()
            });
            let mut backend = RecordingAudio::default();
            for cmd in [0x24a, 0x24b, 0x24f] {
                pending(&mut vm);
                assert_eq!(
                    dispatch::run(
                        profile.semantics(cmd),
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
                    Some(Flow::Done)
                );
                assert_eq!(vm.state().system_integer(0), Some(77));
                assert_eq!(vm.stack().len_bytes(), 0);
            }
            let expected = if enabled {
                vec![
                    AudioRequest::PlayPreloaded {
                        slot: 23,
                        looped: true,
                        volume,
                    },
                    AudioRequest::StopPreloaded { slot: 23 },
                    AudioRequest::SetPreloadedVolume {
                        slot: 23,
                        volume: 128,
                    },
                ]
            } else {
                vec![AudioRequest::StopPreloaded { slot: 23 }]
            };
            assert_eq!(backend.requests(), expected);
            assert!((0..6).all(|i| audio.channel(i).is_none()));
        }
    }
}

#[test]
fn preload_variant_reports_load_results_and_releases_independent_slots() {
    use cmvs_rs::profile::{Registry, VariantId};
    struct Pool {
        loaded: bool,
        fail: bool,
    }
    impl AudioBackend for Pool {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            unreachable!()
        }
        fn preload(&mut self, slot: u8, name: &[u8]) -> Result<bool, Box<dyn Error>> {
            assert_eq!(slot, 23);
            assert!(name.is_empty());
            if self.fail {
                return Err("backend failure".into());
            }
            Ok(self.loaded)
        }
        fn release_preloaded(&mut self, slot: u8) -> Result<(), Box<dyn Error>> {
            assert_eq!(slot, 23);
            self.loaded = false;
            Ok(())
        }
    }
    let profile = Registry::builtin().profile("cmvs-20071117").unwrap();
    for loaded in [false, true] {
        let mut code = vec![];
        command(&mut code, 0x2248, &[23, 0x80000000]);
        command(&mut code, 0x2249, &[23]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"").unwrap();
        let mut audio = AudioHost::default();
        let mut backend = Pool { loaded, fail: true };
        pending(&mut vm);
        assert_eq!(run(&mut vm, &mut audio, &mut backend).unwrap(), None);
        let before = (vm.pc(), vm.stack().len_bytes());
        let semantics = profile.semantics(0x248);
        assert_eq!(semantics, Semantics::Variant(VariantId::AudioPreload));
        let execute =
            |vm: &mut Machine<'_>, audio: &mut AudioHost, backend: &mut Pool, semantics| {
                dispatch::run(
                    semantics,
                    vm,
                    &mut Hosts {
                        audio: Some(AudioHosts { audio, backend }),
                        ..Default::default()
                    },
                )
            };
        assert!(execute(&mut vm, &mut audio, &mut backend, semantics).is_err());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
        backend.fail = false;
        execute(&mut vm, &mut audio, &mut backend, semantics).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(u32::from(loaded)));
        pending(&mut vm);
        execute(&mut vm, &mut audio, &mut backend, profile.semantics(0x249)).unwrap();
        assert!(!backend.loaded);
        assert_eq!(vm.state().system_integer(0), Some(u32::from(loaded)));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert!((0..6).all(|i| audio.channel(i).is_none()));
    }
    for slot in [24, u32::MAX] {
        let mut code = vec![];
        command(&mut code, 0x2248, &[slot, 0x80000000]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"").unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.stack().len_bytes());
        assert!(
            dispatch::run(
                profile.semantics(0x248),
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut AudioHost::default(),
                        backend: &mut RecordingAudio::default()
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
    }
}

#[test]
fn se_restart_is_variant_only_and_preserves_record_and_results() {
    use cmvs_rs::{
        audio::AudioSettings,
        profile::{Registry, VariantId},
    };
    for id in ["cmvs-20071117", "cmvs-20080719"] {
        let semantics = Registry::builtin().profile(id).unwrap().semantics(0xb4);
        assert_eq!(semantics, Semantics::Variant(VariantId::SeRestart));
        for enabled in [true, false] {
            let mut audio = AudioHost::new(AudioSettings {
                se_enabled: enabled,
                ..Default::default()
            });
            let mut backend = RecordingAudio::default();
            for looped in [7, 0] {
                let mut code = vec![];
                command(&mut code, 0x20b4, &[3, looped]);
                let script = script(&code);
                let mut vm = Machine::new(&script).unwrap();
                pending(&mut vm);
                vm.state_mut().set_system_integer(0, 123).unwrap();
                assert_eq!(run(&mut vm, &mut audio, &mut backend).unwrap(), None);
                assert_eq!(
                    dispatch::run(semantics, &mut vm, &mut Hosts::default()).unwrap(),
                    Some(Flow::Unavailable)
                );
                assert_eq!(
                    dispatch::run(
                        semantics,
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
                    Some(Flow::Done)
                );
                assert_eq!(vm.stack().len_bytes(), 0);
                assert_eq!(vm.state().system_integer(0), Some(123));
                let record = audio.channel(3).unwrap();
                assert!(record.flag);
                assert_eq!(record.looped, looped != 0);
                assert!(record.name.is_empty());
                assert_eq!(record.volume, 0);
                if enabled {
                    assert_eq!(
                        backend.requests().last(),
                        Some(&AudioRequest::RestartSe {
                            channel: 3,
                            looped: looped != 0
                        })
                    );
                } else {
                    assert!(backend.requests().is_empty());
                }
            }
        }
    }
}

#[test]
fn se_restart_rejects_bad_arguments_and_unsupported_backends_before_commit() {
    use cmvs_rs::profile::{Semantics, VariantId};
    struct Unsupported;
    impl AudioBackend for Unsupported {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            Ok(false)
        }
    }
    for args in [vec![], vec![3], vec![6, 0], vec![u32::MAX, 1], vec![3, 1]] {
        let mut code = vec![];
        command(&mut code, 0x20b4, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut audio = AudioHost::default();
        assert!(
            dispatch::run(
                Semantics::Variant(VariantId::SeRestart),
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut audio,
                        backend: &mut Unsupported
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert!((0..6).all(|i| audio.channel(i).is_none()));
    }
}

#[test]
fn se_gain_uses_bound_dispatch_without_consuming_results_or_changing_channel_records() {
    use cmvs_rs::{
        audio::AudioSettings,
        profile::{Registry, VariantId},
    };
    let semantics = Registry::builtin()
        .profile("cmvs-20080719")
        .unwrap()
        .semantics(0xb8);
    assert_eq!(semantics, Semantics::Variant(VariantId::SeGain));
    for enabled in [true, false] {
        for multiplier in [0, 32, 256, u32::MAX] {
            let mut audio = AudioHost::new(AudioSettings {
                se_enabled: enabled,
                ..Default::default()
            });
            let mut backend = RecordingAudio::default();
            let mut code = vec![];
            command(&mut code, 0x20b8, &[5, multiplier]);
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            pending(&mut vm);
            vm.state_mut().set_system_integer(0, 123).unwrap();
            assert_eq!(run(&mut vm, &mut audio, &mut backend).unwrap(), None);
            assert_eq!(
                dispatch::run(
                    semantics,
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
                Some(Flow::Done)
            );
            assert_eq!(
                backend.requests(),
                &[AudioRequest::SetSeGain {
                    channel: 5,
                    multiplier
                }]
            );
            assert_eq!(vm.stack().len_bytes(), 0);
            assert_eq!(vm.state().system_integer(0), Some(123));
            assert!((0..6).all(|slot| audio.channel(slot).is_none()));
        }
    }
}

#[test]
fn se_gain_rejects_invalid_parameters_and_backend_failure_before_commit() {
    use cmvs_rs::profile::{Registry, Semantics};
    struct Unsupported;
    impl AudioBackend for Unsupported {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            Ok(false)
        }
    }
    let semantics = Registry::builtin()
        .profile("cmvs-20080719")
        .unwrap()
        .semantics(0xb8);
    assert_ne!(semantics, Semantics::Root);
    for args in [
        vec![],
        vec![1],
        vec![6, 32],
        vec![u32::MAX, 32],
        vec![1, 32],
    ] {
        let mut code = vec![];
        command(&mut code, 0x20b8, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let mut audio = AudioHost::default();
        assert!(
            dispatch::run(
                semantics,
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut audio,
                        backend: &mut Unsupported
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert!((0..6).all(|slot| audio.channel(slot).is_none()));
    }
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
fn background_audio_override_queries_writes_and_gates_without_stopping_playback() {
    for id in ["cmvs-20190305", "cmvs-20190405"] {
        use cmvs_rs::profile::VariantId;
        let mut code = vec![];
        for args in [[0, 999], [7, 0], [0, 0], [1, u32::MAX]] {
            command(&mut code, 0x22ed, &args);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut audio = AudioHost::default();
        let mut backend = RecordingAudio::default();
        let semantics = cmvs_rs::profile::Registry::builtin()
            .profile(id)
            .unwrap()
            .semantics(0x2ed);
        assert_eq!(semantics, Semantics::Variant(VariantId::BackgroundAudio));
        for (index, expected) in [true, false, false, true].into_iter().enumerate() {
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            assert_eq!(
                dispatch::run(semantics, &mut vm, &mut Hosts::default()).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(run(&mut vm, &mut audio, &mut backend).unwrap(), None);
            assert_eq!(
                dispatch::run(
                    semantics,
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
                Some(Flow::Done)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                (before.0 + 2, None, 0)
            );
            assert_eq!(audio.background_output(), expected);
            assert_eq!(
                vm.state().system_integer(0).unwrap(),
                u32::from(if index % 2 == 0 { expected } else { !expected })
            );
            if index == 1 {
                assert!(audio.output_enabled());
                audio.sync_output_activity(false, &mut backend).unwrap();
                assert!(!audio.output_enabled());
                audio.sync_output_activity(true, &mut backend).unwrap();
                assert!(audio.output_enabled());
                audio.sync_output_activity(false, &mut backend).unwrap();
            }
        }
        assert!(audio.output_enabled());
        assert_eq!(
            backend.requests(),
            &[
                AudioRequest::SetOutputEnabled { enabled: false },
                AudioRequest::SetOutputEnabled { enabled: true },
                AudioRequest::SetOutputEnabled { enabled: false },
                AudioRequest::SetOutputEnabled { enabled: true },
            ]
        );
    }
}

#[test]
fn background_audio_errors_preserve_state_and_allow_retry() {
    use cmvs_rs::profile::VariantId;
    let semantics = Semantics::Variant(VariantId::BackgroundAudio);
    let mut audio = AudioHost::default();
    let mut backend = RecordingAudio::default();
    audio.sync_output_activity(false, &mut backend).unwrap();
    for args in [vec![1, 0], vec![1], vec![0]] {
        let mut code = vec![];
        command(&mut code, 0x22ed, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert!(
            dispatch::run(
                semantics,
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut audio,
                        backend: &mut Reject
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert!(audio.background_output());
        assert!(audio.output_enabled());
        if args.len() == 2 {
            dispatch::run(
                semantics,
                &mut vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: &mut audio,
                        backend: &mut backend,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(!audio.background_output());
            assert!(!audio.output_enabled());
            assert!(audio.sync_output_activity(true, &mut Reject).is_err());
            assert!(!audio.output_enabled());
            audio.sync_output_activity(true, &mut backend).unwrap();
            assert!(audio.output_enabled());
            // A new host isolates the short-stack cases from the successful write.
            audio = AudioHost::default();
            audio.sync_output_activity(false, &mut backend).unwrap();
        }
    }
}

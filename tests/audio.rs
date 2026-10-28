use cmvs_rs::{
    audio::{
        AudioBackend, AudioHost, AudioRequest, AudioSettings, BgmState, RecordingAudio, SeChannel,
    },
    machine::{Machine, Step},
    ps3::Script,
};
use std::error::Error;

fn push(code: &mut Vec<u8>, value: u32) {
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(value.to_le_bytes());
}
/// One 0x20b0 call per (channel, text offset, volume, loop, flag) tuple.
fn script(calls: &[(u32, u32, u32, u32, u32)], text: &[u8]) -> Script {
    let mut code = vec![];
    for &(channel, name, volume, looped, flag) in calls {
        push(&mut code, flag);
        push(&mut code, looped);
        push(&mut code, volume);
        code.extend(0x201u16.to_le_bytes());
        code.extend(0x120u16.to_le_bytes());
        code.extend(name.to_le_bytes());
        code.extend(0x20fu16.to_le_bytes());
        code.extend(0x430u16.to_le_bytes());
        push(&mut code, channel);
        code.extend(0x20b0u16.to_le_bytes());
    }
    program(&code, text)
}
fn program(code: &[u8], text: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend_from_slice(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}

fn start_code(channel: u32, looped: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, looped);
    push(&mut code, channel);
    code.extend(0x20b1u16.to_le_bytes());
    code
}

#[test]
fn prepared_sound_start_preserves_name_volume_and_result_and_records_loop_flag() {
    let script = script(&[(5, 0, 64, 0, 0)], b"prepared.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    for looped in [7, 0] {
        let script = program(&start_code(5, looped), b"");
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!(
            host.channel(5),
            Some(&SeChannel {
                name: b"prepared.ogg".to_vec(),
                volume: 64,
                looped: looped != 0,
                flag: true
            })
        );
        assert_eq!(
            backend.requests().last(),
            Some(&AudioRequest::StartSe {
                channel: 5,
                looped: looped != 0
            })
        );
    }
}

#[test]
fn disabled_se_start_updates_zero_initialized_record_without_a_device_request() {
    let script = program(&start_code(0, 1), b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        se_enabled: false,
        ..AudioSettings::default()
    });
    pending(&mut vm);
    host.try_handle(&mut vm, &mut Broken).unwrap();
    assert_eq!(
        host.channel(0),
        Some(&SeChannel {
            name: vec![],
            volume: 0,
            looped: true,
            flag: true
        })
    );
    assert_eq!(vm.state().system_integer(0), None);
    assert!(host.channel(1).is_none());
}

#[test]
fn start_backend_failure_is_transactional_and_retry_keeps_original_request() {
    let script = script(&[(0, 0, 128, 1, 0)], b"a.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut Started).unwrap();
    let before = host.channel(0).cloned();
    let script = program(&start_code(0, 0), b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(host.try_handle(&mut vm, &mut Broken).is_err());
    assert_eq!(host.channel(0), before.as_ref());
    assert_eq!(vm.pending_system(), Some((pc, 0xb1)));
    assert_eq!(vm.stack().len_bytes(), 8);
    assert_eq!(vm.state().system_integer(0), None);
    let mut backend = RecordingAudio::default();
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(
        backend.requests(),
        &[AudioRequest::StartSe {
            channel: 0,
            looped: false
        }]
    );
    assert_eq!(host.channel(0).unwrap().name, b"a.ogg");
    assert!(host.channel(0).unwrap().flag);
}

#[test]
fn start_rejects_invalid_channels_and_truncated_stack_without_effects() {
    let mut truncated = vec![];
    push(&mut truncated, 0);
    truncated.extend(0x20b1u16.to_le_bytes());
    for code in [start_code(6, 0), start_code(u32::MAX, 0), truncated] {
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut host = AudioHost::default();
        let mut backend = RecordingAudio::default();
        let pc = vm.pc();
        assert!(host.try_handle(&mut vm, &mut backend).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0xb1)));
        assert!((0..6).all(|i| host.channel(i).is_none()));
        assert!(backend.requests().is_empty());
        assert_eq!(vm.state().system_integer(0), None);
    }
}
fn stop_code(channel: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, channel);
    code.extend(0x20b3u16.to_le_bytes());
    code
}

fn voice_stop_code(channel: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, channel);
    code.extend(0x20a8u16.to_le_bytes());
    code
}

#[test]
fn voice_stop_routes_all_four_channels_without_touching_se_bgm_or_results() {
    let mut host = AudioHost::new(AudioSettings {
        se_enabled: false,
        bgm_enabled: false,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    for channel in 0..4 {
        let mut code = vec![];
        push(&mut code, 0x1234);
        code.extend(voice_stop_code(channel));
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        for slot in 0..7 {
            vm.state_mut().set_system_integer(slot, 99).unwrap();
        }
        pending(&mut vm);
        let pc = vm.pc();
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!(
            backend.requests().last(),
            Some(&AudioRequest::StopVoice {
                channel: channel as u8
            })
        );
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x1234);
        for slot in 0..7 {
            assert_eq!(vm.state().system_integer(slot), Some(99));
        }
        for slot in 0..6 {
            assert!(host.channel(slot).is_none());
        }
        assert_eq!(host.bgm(), &BgmState::default());
    }
}

#[test]
fn voice_stop_failure_can_retry_and_invalid_channels_never_reach_backend() {
    let script = program(&voice_stop_code(2), b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    let command = vm.pending_system();
    assert!(host.try_handle(&mut vm, &mut Broken).is_err());
    assert_eq!(vm.pending_system(), command);
    assert_eq!(vm.stack().len_bytes(), 4);
    let mut backend = RecordingAudio::default();
    assert!(host.try_handle(&mut vm, &mut backend).unwrap());
    assert_eq!(
        backend.requests(),
        &[AudioRequest::StopVoice { channel: 2 }]
    );
    for code in [
        voice_stop_code(4),
        voice_stop_code(u32::MAX),
        0x20a8u16.to_le_bytes().to_vec(),
    ] {
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let command = vm.pending_system();
        let len = vm.stack().len_bytes();
        let requests = backend.requests().len();
        assert!(host.try_handle(&mut vm, &mut backend).is_err());
        assert_eq!(vm.pending_system(), command);
        assert_eq!(vm.stack().len_bytes(), len);
        assert_eq!(backend.requests().len(), requests);
    }
}

#[test]
fn stop_clears_name_and_flag_but_keeps_loop_volume_and_result() {
    let script = script(&[(3, 0, 64, 1, 1)], b"loop.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    let script = program(&stop_code(3), b"");
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 77).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(host.try_handle(&mut vm, &mut backend).unwrap());
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
    assert_eq!(vm.state().system_integer(0), Some(77));
    assert_eq!(
        host.channel(3),
        Some(&SeChannel {
            name: vec![],
            looped: true,
            volume: 64,
            flag: false
        })
    );
    assert_eq!(
        backend.requests().last(),
        Some(&AudioRequest::StopSe { channel: 3 })
    );
}

#[test]
fn stop_reaches_the_device_even_with_se_disabled_and_untouched_records() {
    let script = program(&stop_code(0), b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        se_enabled: false,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(backend.requests(), &[AudioRequest::StopSe { channel: 0 }]);
    assert_eq!(host.channel(0), Some(&SeChannel::default()));
    assert!(host.channel(1).is_none());
    assert_eq!(vm.state().system_integer(0), None);
}

#[test]
fn stop_all_clears_only_names_in_order_even_with_se_disabled() {
    for enabled in [false, true] {
        let mut host = AudioHost::new(AudioSettings {
            se_enabled: enabled,
            ..AudioSettings::default()
        });
        let setup = script(&[(0, 0, 64, 1, 1), (4, 0, 128, 0, 1)], b"loop.ogg\0");
        let mut vm = Machine::new(&setup).unwrap();
        for _ in 0..2 {
            pending(&mut vm);
            host.try_handle(&mut vm, &mut Started).unwrap();
        }
        let before: Vec<_> = (0..6)
            .map(|i| host.channel(i).cloned().unwrap_or_default())
            .collect();
        let mut code = vec![];
        push(&mut code, 0x12345678);
        code.extend(0x20b5u16.to_le_bytes());
        code.extend(0x20b5u16.to_le_bytes());
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        for slot in 0..7 {
            vm.state_mut().set_system_integer(slot, 100 + slot).unwrap();
        }
        let mut backend = RecordingAudio::default();
        for _ in 0..2 {
            pending(&mut vm);
            let pc = vm.pc();
            host.try_handle(&mut vm, &mut backend).unwrap();
            assert_eq!(vm.pc(), pc + 2);
            assert_eq!(vm.stack().len_bytes(), 4);
            assert_eq!(vm.stack().peek_u32(4).unwrap(), 0x12345678);
            for slot in 0..7 {
                assert_eq!(vm.state().system_integer(slot), Some(100 + slot));
            }
            for (i, old) in before.iter().enumerate() {
                let mut expected = old.clone();
                expected.name.clear();
                assert_eq!(host.channel(i), Some(&expected));
            }
        }
        let expected: Vec<_> = (0..6)
            .cycle()
            .take(12)
            .map(|channel| AudioRequest::StopSe { channel })
            .collect();
        assert_eq!(backend.requests(), expected);
    }
}

#[test]
fn stop_all_failure_keeps_successful_prefix_and_retries_pending_command() {
    struct FailsAtThree(Vec<u8>);
    impl AudioBackend for FailsAtThree {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            unreachable!()
        }
        fn stop(&mut self, channel: u8) -> Result<(), Box<dyn Error>> {
            if channel == 3 {
                return Err("synthetic stop failure".into());
            }
            self.0.push(channel);
            Ok(())
        }
    }
    let setup = script(&[(1, 0, 64, 1, 1), (3, 0, 128, 0, 1)], b"loop.ogg\0");
    let mut host = AudioHost::default();
    let mut vm = Machine::new(&setup).unwrap();
    for _ in 0..2 {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut Started).unwrap();
    }
    let failed_before = host.channel(3).cloned();
    let script = program(&0x20b5u16.to_le_bytes(), b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let pending_command = vm.pending_system();
    let mut backend = FailsAtThree(vec![]);
    assert!(host.try_handle(&mut vm, &mut backend).is_err());
    assert_eq!(backend.0, [0, 1, 2]);
    assert_eq!(vm.pending_system(), pending_command);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), None);
    assert!(host.channel(1).unwrap().name.is_empty());
    assert!(host.channel(1).unwrap().flag);
    assert_eq!(host.channel(3), failed_before.as_ref());
    assert!(host.channel(4).is_none());
    assert!(host.channel(5).is_none());
    let mut backend = RecordingAudio::default();
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(vm.pending_system(), None);
    assert_eq!(backend.requests().len(), 6);
    assert!((0..6).all(|i| host.channel(i).unwrap().name.is_empty()));
    assert!(host.channel(3).unwrap().flag);
}

#[test]
fn stop_backend_failure_and_invalid_channels_keep_the_command_pending() {
    let script = script(&[(1, 0, 0, 0, 1)], b"a.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut Started).unwrap();
    let before = host.channel(1).cloned();
    let script = program(&stop_code(1), b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    // Broken has no stop, so the default trait method reports it as unimplemented.
    assert!(host.try_handle(&mut vm, &mut Broken).is_err());
    assert_eq!(host.channel(1), before.as_ref());
    assert_eq!(vm.pending_system(), Some((pc, 0xb3)));
    assert_eq!(vm.stack().len_bytes(), 4);
    let mut backend = RecordingAudio::default();
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(backend.requests(), &[AudioRequest::StopSe { channel: 1 }]);
    assert!(host.channel(1).unwrap().name.is_empty());

    for code in [
        stop_code(6),
        stop_code(u32::MAX),
        0x20b3u16.to_le_bytes().to_vec(),
    ] {
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut host = AudioHost::default();
        let mut backend = RecordingAudio::default();
        let pc = vm.pc();
        assert!(host.try_handle(&mut vm, &mut backend).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0xb3)));
        assert!((0..6).all(|i| host.channel(i).is_none()));
        assert!(backend.requests().is_empty());
    }
}

fn se_fade_code(channel: u32, fade: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, fade);
    push(&mut code, channel);
    code.extend(0x20b6u16.to_le_bytes());
    code
}

#[test]
fn se_fade_out_reaches_the_device_and_clears_only_the_name() {
    for se_enabled in [true, false] {
        let script = script(&[(1, 0, 64, 1, 1)], b"loop.ogg\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::new(AudioSettings {
            se_enabled,
            ..AudioSettings::default()
        });
        let mut backend = RecordingAudio::default();
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        let script = program(&se_fade_code(1, 500), b"");
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
        assert_eq!(vm.state().system_integer(0), Some(77));
        // Unlike 0x20b3, the start flag survives.
        assert_eq!(
            host.channel(1),
            Some(&SeChannel {
                name: vec![],
                looped: true,
                volume: 64,
                flag: true
            })
        );
        // SE_USE is not consulted.
        assert_eq!(
            backend.requests().last(),
            Some(&AudioRequest::FadeOutSe {
                channel: 1,
                fade_ms: 500
            })
        );
    }
}

#[test]
fn se_fade_out_failures_and_invalid_arguments_keep_the_command_pending() {
    let script = program(&se_fade_code(2, 0), b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    let pc = vm.pc();
    // Broken has no SE fade-out, so the default trait method reports it as unimplemented.
    assert!(host.try_handle(&mut vm, &mut Broken).is_err());
    assert_eq!(vm.pending_system(), Some((pc, 0xb6)));
    assert_eq!(vm.stack().len_bytes(), 8);
    assert!(host.channel(2).is_none());
    let mut backend = RecordingAudio::default();
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(
        backend.requests(),
        &[AudioRequest::FadeOutSe {
            channel: 2,
            fade_ms: 0
        }]
    );
    assert_eq!(host.channel(2), Some(&SeChannel::default()));
    let mut short = vec![];
    push(&mut short, 1);
    short.extend(0x20b6u16.to_le_bytes());
    for code in [se_fade_code(6, 10), se_fade_code(u32::MAX, 10), short] {
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        let mut host = AudioHost::default();
        let mut backend = RecordingAudio::default();
        assert!(host.try_handle(&mut vm, &mut backend).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0xb6)));
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert!((0..6).all(|i| host.channel(i).is_none()));
        assert!(backend.requests().is_empty());
    }
}

fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
struct Started;
impl AudioBackend for Started {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
}
struct Broken;
impl AudioBackend for Broken {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Err("synthetic device failure".into())
    }
}

#[test]
fn recording_backend_keeps_requests_and_reports_not_started() {
    let script = script(
        &[(0, 0, 0, 0, 0), (5, 11, 128, 1, 1)],
        b"sys301.ogg\0loop.ogg\0",
    );
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for _ in 0..2 {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(0));
    }
    assert_eq!(
        backend.requests(),
        [
            AudioRequest::PlaySe {
                channel: 0,
                name: b"sys301.ogg".to_vec(),
                volume: 256,
                looped: false,
                flag: false,
                fade_in_ms: 0
            },
            AudioRequest::PlaySe {
                channel: 5,
                name: b"loop.ogg".to_vec(),
                volume: 128,
                looped: true,
                flag: true,
                fade_in_ms: 0
            }
        ]
    );
    assert_eq!(
        host.channel(5),
        Some(&SeChannel {
            name: b"loop.ogg".to_vec(),
            looped: true,
            volume: 128,
            flag: true
        })
    );
}

#[test]
fn settings_scale_volume_and_disabled_se_only_records_the_channel() {
    let script = script(&[(1, 0, 64, 0, 0), (2, 0, 0, 0, 0)], b"a.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        se_enabled: true,
        master: 128,
        se: 128,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut Started).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
    let mut disabled = AudioHost::new(AudioSettings {
        se_enabled: false,
        ..AudioSettings::default()
    });
    pending(&mut vm);
    disabled.try_handle(&mut vm, &mut backend).unwrap();
    assert!(backend.requests().is_empty());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(disabled.channel(2).unwrap().name, b"a.ogg");

    let mut record = RecordingAudio::default();
    let script = self::script(&[(1, 0, 64, 0, 0)], b"a.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    AudioHost::new(AudioSettings {
        se_enabled: true,
        master: 128,
        se: 128,
        ..AudioSettings::default()
    })
    .try_handle(&mut vm, &mut record)
    .unwrap();
    // (128 * 128 >> 8) = 64, then (64 * 64 >> 8) = 16.
    assert!(matches!(
        record.requests()[0],
        AudioRequest::PlaySe { volume: 16, .. }
    ));
}

#[test]
fn invalid_channel_long_name_and_backend_errors_keep_the_command_pending() {
    let mut text = b"ok.ogg\0".to_vec();
    text.extend([b'x'; 32]);
    text.push(0);
    for (call, backend_fails) in [
        ((6, 0, 0, 0, 0), false),
        ((0, 7, 0, 0, 0), false),
        ((0, 0, 0, 0, 0), true),
    ] {
        let script = script(&[call], &text);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::default();
        pending(&mut vm);
        let outcome = if backend_fails {
            host.try_handle(&mut vm, &mut Broken)
        } else {
            host.try_handle(&mut vm, &mut RecordingAudio::default())
        };
        assert!(outcome.is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0xb0));
        assert_eq!(vm.stack().len_bytes(), 20);
        assert!((0..6).all(|i| host.channel(i).is_none()));
        assert_eq!(vm.state().system_integer(0), None);
    }
}

/// 0x20a4 with (first, second, fade, loop), or 0x20a0 with (first, fade, loop) when
/// `second` is None; names are text offsets.
fn bgm_code(first: u32, second: Option<u32>, fade: u32, looped: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, looped);
    push(&mut code, fade);
    let mut names = vec![first];
    names.extend(second);
    for &name in names.iter().rev() {
        code.extend(0x201u16.to_le_bytes());
        code.extend(0x120u16.to_le_bytes());
        code.extend(name.to_le_bytes());
        code.extend(0x20fu16.to_le_bytes());
        code.extend(0x430u16.to_le_bytes());
    }
    code.extend(
        if second.is_some() {
            0x20a4u16
        } else {
            0x20a0u16
        }
        .to_le_bytes(),
    );
    code
}
const BGM_TEXT: &[u8] = b"bgm41v.ogg\0bgm41.ogg\0";

#[test]
fn bgm_without_fade_stops_both_channels_and_starts_on_channel_one() {
    let mut code = bgm_code(0, Some(11), 0, 1);
    code.extend(bgm_code(11, None, 0, 0));
    let script = program(&code, BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 5).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        master: 128,
        bgm: 128,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    for expected in [
        (0, b"bgm41v.ogg".to_vec(), b"bgm41.ogg".to_vec(), true),
        (1, b"bgm41.ogg".to_vec(), vec![], false),
    ] {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        let (previous, primary, secondary, looped) = expected;
        assert_eq!(
            backend.requests().last(),
            Some(&AudioRequest::PlayBgm {
                channel: 1,
                previous,
                primary: primary.clone(),
                secondary: secondary.clone(),
                volume: 64,
                fade_ms: 0,
                looped,
                flag_1576: 0
            })
        );
        assert_eq!(
            host.bgm(),
            &BgmState {
                primary,
                secondary,
                channel: 1,
                playing: true
            }
        );
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(5));
}

#[test]
fn bgm_fades_alternate_channels_and_disabled_bgm_only_updates_state() {
    let mut code = vec![];
    for _ in 0..3 {
        code.extend(bgm_code(11, None, 500, 1));
    }
    let script = program(&code, BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for (channel, previous) in [(1, 0), (0, 1)] {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert!(matches!(
            backend.requests().last(),
            Some(AudioRequest::PlayBgm { channel: c, previous: p, fade_ms: 500, .. })
                if (*c, *p) == (channel, previous)
        ));
    }
    // Disabled BGM flips the channel and keeps the names, without a device request.
    let mut disabled = AudioHost::new(AudioSettings {
        bgm_enabled: false,
        ..AudioSettings::default()
    });
    pending(&mut vm);
    disabled.try_handle(&mut vm, &mut Broken).unwrap();
    assert_eq!(
        disabled.bgm(),
        &BgmState {
            primary: b"bgm41.ogg".to_vec(),
            secondary: vec![],
            channel: 1,
            playing: false
        }
    );
}

#[test]
fn bgm_errors_keep_the_command_pending() {
    let mut text = BGM_TEXT.to_vec();
    text.extend([b'x'; 32]);
    text.push(0);
    for (code, broken) in [
        (bgm_code(0, Some(21), 0, 0), false), // 32-byte second name
        (bgm_code(0, None, 0, 0), true),      // backend failure
    ] {
        let script = program(&code, &text);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::default();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        let outcome = if broken {
            host.try_handle(&mut vm, &mut Broken)
        } else {
            host.try_handle(&mut vm, &mut RecordingAudio::default())
        };
        assert!(outcome.is_err());
        assert_eq!(vm.pc(), pc);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(host.bgm(), &BgmState::default());
    }
}

fn fade_out_code(fade: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, fade);
    code.extend(0x20a2u16.to_le_bytes());
    code
}

#[test]
fn bgm_fade_out_clears_names_fades_the_current_channel_and_flips_it() {
    let mut code = bgm_code(0, Some(11), 0, 1);
    code.extend(fade_out_code(2000));
    let script = program(&code, BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 5).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for _ in 0..2 {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
    }
    assert_eq!(
        backend.requests().last(),
        Some(&AudioRequest::FadeOutBgm {
            channel: 1,
            fade_ms: 2000
        })
    );
    assert_eq!(
        host.bgm(),
        &BgmState {
            channel: 0,
            ..BgmState::default()
        }
    );
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(5));
}

#[test]
fn bgm_fade_out_without_bgm_or_backend_support() {
    // Disabled BGM clears the state without a device request or channel flip.
    let mut code = bgm_code(11, None, 0, 0);
    code.extend(fade_out_code(0));
    let script = program(&code, BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        bgm_enabled: false,
        ..AudioSettings::default()
    });
    for _ in 0..2 {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut Broken).unwrap();
    }
    assert_eq!(
        host.bgm(),
        &BgmState {
            channel: 1,
            ..BgmState::default()
        }
    );
    // A backend without fade-out support keeps the command and state for retry.
    let script = program(&fade_out_code(100), b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(host.try_handle(&mut vm, &mut Broken).is_err());
    assert_eq!(vm.pending_system(), Some((pc, 0xa2)));
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(host.bgm(), &BgmState::default());
}

#[test]
fn bgm_stop_releases_both_channels_and_pops_nothing() {
    let mut code = bgm_code(11, None, 500, 1);
    push(&mut code, 77);
    code.extend(0x20a1u16.to_le_bytes());
    let script = program(&code, BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for _ in 0..2 {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
    }
    assert_eq!(backend.requests().last(), Some(&AudioRequest::StopBgm));
    assert_eq!(host.bgm(), &BgmState::default());
    // 0x4000: the unrelated word below stays on the stack.
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 77);
    // Disabled BGM keeps the channel and sends nothing.
    let mut disabled = AudioHost::new(AudioSettings {
        bgm_enabled: false,
        ..AudioSettings::default()
    });
    let script = program(&bgm_code(11, None, 500, 1), BGM_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    disabled.try_handle(&mut vm, &mut Broken).unwrap();
    let mut stop = vec![];
    stop.extend(0x20a1u16.to_le_bytes());
    let script = program(&stop, b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    disabled.try_handle(&mut vm, &mut Broken).unwrap();
    assert_eq!(disabled.bgm().channel, 1);
    assert!(disabled.bgm().primary.is_empty());
}

/// One 0x20ba call: like 0x20b0 with the fade-in time between volume and loop.
fn fade_in_code(
    channel: u32,
    name: u32,
    volume: u32,
    fade: u32,
    looped: u32,
    flag: u32,
) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, flag);
    push(&mut code, looped);
    push(&mut code, fade);
    push(&mut code, volume);
    code.extend(0x201u16.to_le_bytes());
    code.extend(0x120u16.to_le_bytes());
    code.extend(name.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    code.extend(0x430u16.to_le_bytes());
    push(&mut code, channel);
    code.extend(0x20bau16.to_le_bytes());
    code
}

#[test]
fn fade_in_se_records_the_fade_and_shifts_loop_and_flag() {
    let mut code = fade_in_code(3, 0, 128, 1500, 1, 1);
    code.extend(fade_in_code(4, 0, 0, 700, 0, 0));
    let script = program(&code, b"wave.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for _ in 0..2 {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(
        backend.requests(),
        [
            AudioRequest::PlaySe {
                channel: 3,
                name: b"wave.ogg".to_vec(),
                volume: 128,
                looped: true,
                flag: true,
                fade_in_ms: 1500
            },
            AudioRequest::PlaySe {
                channel: 4,
                name: b"wave.ogg".to_vec(),
                volume: 256,
                looped: false,
                flag: false,
                fade_in_ms: 700
            }
        ]
    );
    // The channel record matches 0x20b0's: the fade-in time is not kept.
    assert_eq!(
        host.channel(3),
        Some(&SeChannel {
            name: b"wave.ogg".to_vec(),
            looped: true,
            volume: 128,
            flag: true
        })
    );
    // A short stack (no flag word) keeps the command pending.
    let mut short = fade_in_code(3, 0, 128, 1500, 1, 1);
    short.drain(..8);
    let script = program(&short, b"wave.ogg\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let bytes = vm.stack().len_bytes();
    assert!(
        AudioHost::default()
            .try_handle(&mut vm, &mut backend)
            .is_err()
    );
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0xba));
    assert_eq!(vm.stack().len_bytes(), bytes);
}

/// 0x20a6: SP-16 start, SP-12 volume, SP-8 name (text offset), SP-4 channel.
fn voice_code(channel: u32, name: u32, volume: u32, start: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, start);
    push(&mut code, volume);
    code.extend(0x201u16.to_le_bytes());
    code.extend(0x120u16.to_le_bytes());
    code.extend(name.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    code.extend(0x430u16.to_le_bytes());
    push(&mut code, channel);
    code.extend(0x20a6u16.to_le_bytes());
    code
}
/// Offsets: 0 a short name, 6 a 32-byte name.
const VOICE_TEXT: &[u8] = b"v.ogg\0abcdefghijklmnopqrstuvwxyz012345\0";
struct Voiced;
impl AudioBackend for Voiced {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn play_voice(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
}

#[test]
fn voice_play_scales_volume_reports_loading_and_forgets_the_pcm_length() {
    let settings = AudioSettings {
        voice: 128,
        ..AudioSettings::default()
    };
    for (volume, effective) in [(0, 128), (u32::MAX, 128), (128, 64)] {
        let script = program(&voice_code(2, 0, volume, 1), VOICE_TEXT);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(4, 77).unwrap();
        let mut host = AudioHost::new(settings);
        let mut backend = RecordingAudio::default();
        pending(&mut vm);
        let pc = vm.pc();
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
        assert_eq!(
            backend.requests(),
            &[AudioRequest::PlayVoice {
                channel: 2,
                name: b"v.ogg".to_vec(),
                volume: effective,
                start: true
            }]
        );
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.state().system_integer(4), None);
        assert!((0..6).all(|i| host.channel(i).is_none()));
    }
    let script = program(&voice_code(0, 0, 0, 0), VOICE_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    AudioHost::default()
        .try_handle(&mut vm, &mut Voiced)
        .unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
}

#[test]
fn disabled_voice_reports_failure_without_a_request_or_name_check() {
    let script = program(&voice_code(3, 6, 0, 1), VOICE_TEXT);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(4, 77).unwrap();
    let mut host = AudioHost::new(AudioSettings {
        voice_enabled: false,
        ..AudioSettings::default()
    });
    pending(&mut vm);
    host.try_handle(&mut vm, &mut Broken).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert_eq!(vm.state().system_integer(4), Some(77));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn voice_play_errors_keep_the_command_pending() {
    let mut short = vec![];
    push(&mut short, 0);
    push(&mut short, 0);
    short.extend(0x20a6u16.to_le_bytes());
    for (code, backend_supported) in [
        (voice_code(4, 0, 0, 1), true),
        (voice_code(u32::MAX, 0, 0, 1), true),
        (voice_code(0, 6, 0, 1), true),
        (short, true),
        (voice_code(1, 0, 0, 1), false),
    ] {
        let script = program(&code, VOICE_TEXT);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_system_integer(0, 9).unwrap();
        pending(&mut vm);
        let (pending_command, bytes) = (vm.pending_system(), vm.stack().len_bytes());
        let mut host = AudioHost::default();
        let mut recording = RecordingAudio::default();
        let result = if backend_supported {
            host.try_handle(&mut vm, &mut recording)
        } else {
            host.try_handle(&mut vm, &mut Broken)
        };
        assert!(result.is_err());
        assert_eq!(vm.pending_system(), pending_command);
        assert_eq!(vm.stack().len_bytes(), bytes);
        assert_eq!(vm.state().system_integer(0), Some(9));
        assert!(recording.requests().is_empty());
    }
}

/// One 0x20b7 (SE) or 0x20aa (voice) position query of `channel`.
fn position_code(op: u16, channel: u32) -> Vec<u8> {
    let mut code = vec![];
    push(&mut code, channel);
    code.extend(op.to_le_bytes());
    code
}
/// Fixed positions per SE and voice channel; records each (voice, channel) query.
struct Positions {
    se: [Option<u32>; 6],
    voice: [Option<u32>; 4],
    queries: std::cell::RefCell<Vec<(bool, u8)>>,
}
impl AudioBackend for Positions {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Err("unexpected play".into())
    }
    fn se_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        self.queries.borrow_mut().push((false, channel));
        Ok(self.se[usize::from(channel)])
    }
    fn voice_position(&self, channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        self.queries.borrow_mut().push((true, channel));
        Ok(self.voice[usize::from(channel)])
    }
}

#[test]
fn position_queries_report_milliseconds_or_minus_one_per_channel_family() {
    let queries = [(0x20b7, 2), (0x20b7, 5), (0x20aa, 0), (0x20aa, 3)];
    let mut code = vec![];
    for (op, channel) in queries {
        code.extend(position_code(op, channel));
    }
    let script = program(&code, b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let mut backend = Positions {
        se: [None, None, Some(1234), None, None, None],
        voice: [Some(0), None, None, Some(u32::MAX - 1)],
        queries: Default::default(),
    };
    for expected in [1234, u32::MAX, 0, u32::MAX - 1] {
        pending(&mut vm);
        vm.state_mut().set_system_integer(4, 77).unwrap();
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.state().system_integer(4), Some(77));
        assert_eq!((vm.pending_system(), vm.stack().len_bytes()), (None, 0));
    }
    assert_eq!(vm.pc(), code.len());
    assert_eq!(
        *backend.queries.borrow(),
        [(false, 2), (false, 5), (true, 0), (true, 3)]
    );
    assert_eq!(host.channel(2), None);
    // The recording backend never loads a sound, so every slot reports -1.
    let mut vm = Machine::new(&script).unwrap();
    let mut recording = RecordingAudio::default();
    for _ in queries {
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 7).unwrap();
        assert!(host.try_handle(&mut vm, &mut recording).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    }
    assert!(recording.requests().is_empty());
}

#[test]
fn position_queries_reject_large_channels_and_keep_backend_errors_pending() {
    let silent = || Positions {
        se: [None; 6],
        voice: [None; 4],
        queries: Default::default(),
    };
    for (op, channel) in [(0x20b7, 6), (0x20b7, u32::MAX), (0x20aa, 4)] {
        let script = program(&position_code(op, channel), b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 7).unwrap();
        let mut backend = silent();
        assert!(
            AudioHost::default()
                .try_handle(&mut vm, &mut backend)
                .is_err()
        );
        assert!(backend.queries.borrow().is_empty());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(op - 0x2000));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.state().system_integer(0), Some(7));
    }
    // A backend without the query, or a missing channel argument, keeps the command.
    for (code, bytes) in [
        (position_code(0x20b7, 0), 4),
        (0x20aau16.to_le_bytes().to_vec(), 0),
    ] {
        let script = program(&code, b"");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let error = AudioHost::default().try_handle(&mut vm, &mut Started);
        assert!(error.is_err());
        assert!(vm.pending_system().is_some());
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

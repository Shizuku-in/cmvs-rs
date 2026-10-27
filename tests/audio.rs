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

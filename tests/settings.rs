use cmvs_rs::{
    machine::Machine,
    settings::{DEFAULT_FONT_FACE, SettingsHost},
};
#[path = "support/bytecode.rs"]
mod bytecode;
use bytecode::{command, pending, script};

#[test]
fn bold_setting_normalizes_ini_and_vm_values_and_preserves_write_result() {
    assert!(!SettingsHost::default().text_output().bold);
    assert!(!SettingsHost::from_ini(&[]).text_output().bold);
    assert!(
        SettingsHost::from_ini(b"[MESSAGE_INFO]\nTEXT_BOLD=-1\n")
            .text_output()
            .bold
    );
    let mut code = vec![];
    for raw in [u32::MAX, 0, 2] {
        command(&mut code, 0x22e0, &[1, raw]);
        command(&mut code, 0x22e0, &[0, 0]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut settings = SettingsHost::default();
    for expected in [1, 0, 1] {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn read_skip_setting_loads_ini_and_normalizes_writes_without_changing_results() {
    assert!(!SettingsHost::default().read_skip());
    assert!(SettingsHost::from_ini(&[]).read_skip());
    assert!(!SettingsHost::from_ini(b"[MESSAGE_INFO]\nREAD_SKIP=0\n").read_skip());
    assert!(SettingsHost::from_ini(b"[MESSAGE_INFO]\nREAD_SKIP=-1\n").read_skip());
    let mut code = vec![];
    for value in [u32::MAX, 0, 1] {
        command(&mut code, 0x22e4, &[1, value]);
        command(&mut code, 0x22e4, &[0, 0]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut settings = SettingsHost::default();
    for expected in [1, 0, 1] {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn text_settings_round_trip_clamps_preserve_results_and_keep_outline_signed_remainder() {
    for (op, raw, expected) in [
        (0x22df, u32::MAX, 1),
        (0x22df, 0x8000_0000, 0xffff_fffe),
        (0x22df, 5, 2),
        (0x22e1, u32::MAX, 0),
        (0x22e1, 11, 10),
        (0x22e2, 3, 2),
        (0x22e2, u32::MAX, 0),
        (0x22e3, 5, 4),
        (0x22e3, u32::MAX, 0),
    ] {
        let mut code = vec![];
        command(&mut code, op, &[1, raw]);
        command(&mut code, op, &[0, 0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = SettingsHost::default();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        pending(&mut vm);
        host.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
    }
    let host = SettingsHost::from_ini(
        b"[MESSAGE_INFO]\nTEXT_WAIT=7\nTEXT_GANG=2\nEFFECT_TYPE=0\nEFFECT_TIMER=4\n",
    );
    assert_eq!(
        host.text_output(),
        cmvs_rs::settings::TextOutputSettings {
            bold: false,
            speed: 7,
            outline: 2,
            reveal_mode: 0,
            reveal_duration: 4
        }
    );
    assert_eq!(
        SettingsHost::from_ini(b"[MESSAGE_INFO]\nTEXT_WAIT=-1\n")
            .text_output()
            .speed,
        u32::MAX
    );
}

#[test]
fn auto_startup_parameters_retain_raw_words_and_apply_text_delay_formula() {
    use cmvs_rs::settings::AutoAdvanceSettings;
    let host = SettingsHost::from_ini(b"[MESSAGE_INFO]\nAUTO_MODE_1=-1\nAUTO_MODE_2=10\nAUTO_MODE_3=4\nAUTO_MODE_4=7\nAUTO_VOICE_SYNC=0\n");
    assert_eq!(
        host.auto_advance(),
        AutoAdvanceSettings {
            base: u32::MAX,
            per_glyph: 10,
            acceleration: 4,
            voice_wait: 7,
            sync_voice: false
        }
    );
    let simple = AutoAdvanceSettings {
        base: 1,
        per_glyph: 10,
        ..Default::default()
    };
    assert_eq!(simple.text_delay_ms(4).unwrap(), 300);
    assert_eq!(
        AutoAdvanceSettings {
            acceleration: 4,
            ..simple
        }
        .text_delay_ms(4)
        .unwrap(),
        294
    );
    assert_eq!(
        AutoAdvanceSettings {
            base: 1000,
            acceleration: 4,
            ..simple
        }
        .text_delay_ms(4)
        .unwrap(),
        100200
    );
    assert!(simple.text_delay_ms(4097).is_err());
    assert_eq!(
        AutoAdvanceSettings {
            base: u32::MAX,
            ..Default::default()
        }
        .text_delay_ms(0)
        .unwrap(),
        u32::MAX - 99
    );
}

#[test]
fn audio_use_switches_normalize_and_skip_repeated_writes() {
    use cmvs_rs::audio::{AudioHost, RecordingAudio};
    for op in 0x22d6..=0x22d8 {
        let mut code = vec![];
        for value in [1, 0, 0, u32::MAX, u32::MAX] {
            command(&mut code, op, &[1, value]);
            command(&mut code, op, &[0, 0]);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::default();
        let mut backend = RecordingAudio::default();
        for value in [1, 0, 0, 1, 1] {
            vm.state_mut().set_system_integer(0, 77).unwrap();
            pending(&mut vm);
            host.try_handle(&mut vm, &mut backend).unwrap();
            assert_eq!(vm.state().system_integer(0), Some(77));
            pending(&mut vm);
            host.try_handle(&mut vm, &mut backend).unwrap();
            assert_eq!(vm.state().system_integer(0), Some(value));
            assert_eq!(vm.stack().len_bytes(), 0);
        }
        // Only SE needs to release/rebuild empty device slots. BGM without a
        // remembered name and voice gating need no device operation.
        assert_eq!(backend.requests().len(), if op == 0x22d7 { 2 } else { 0 });
    }
}

#[test]
fn bgm_switch_preserves_names_and_restarts_the_pair_without_fade() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, RecordingAudio};
    let mut code = vec![];
    command(&mut code, 0x20a4, &[0x8000_0000, 0x8000_0001, 100, 0]);
    command(&mut code, 0x22d6, &[1, 0]);
    command(&mut code, 0x22d6, &[1, 1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"intro.ogg").unwrap();
    vm.state_mut().set_string(1, b"loop.ogg").unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(backend.requests().last(), Some(&AudioRequest::StopBgm));
    assert_eq!(host.bgm().primary, b"intro.ogg");
    assert_eq!(host.bgm().secondary, b"loop.ogg");
    assert_eq!(host.bgm().channel, 0);
    assert!(!host.bgm().playing);
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(
        backend.requests().last(),
        Some(&AudioRequest::PlayBgm {
            channel: 1,
            previous: 0,
            primary: b"intro.ogg".to_vec(),
            secondary: b"loop.ogg".to_vec(),
            volume: 256,
            fade_ms: 0,
            looped: true,
            flag_1576: 0,
        })
    );
    assert!(host.bgm().playing && host.settings().bgm_enabled);
    assert_eq!(host.bgm().channel, 1);
}

#[test]
fn se_switch_rebuilds_named_slots_and_only_restarts_flagged_loops() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, AudioSettings, RecordingAudio};
    let mut code = vec![];
    for (index, looped, flag) in [(0, 1, 1), (1, 0, 1), (2, 1, 0), (3, 0, 0)] {
        command(&mut code, 0x20b0, &[index, 0x8000_0000, 64, looped, flag]);
    }
    command(&mut code, 0x22d7, &[1, 0]);
    command(&mut code, 0x22d7, &[1, 1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"synthetic.ogg").unwrap();
    let mut host = AudioHost::new(AudioSettings {
        master: 128,
        se: 128,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    for _ in 0..4 {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
    }
    let before: Vec<_> = (0..6).map(|i| host.channel(i).cloned()).collect();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert_eq!(
        backend.requests().last(),
        Some(&AudioRequest::ConfigureSe {
            channels: Default::default()
        })
    );
    assert_eq!(
        (0..6).map(|i| host.channel(i).cloned()).collect::<Vec<_>>(),
        before
    );
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    let AudioRequest::ConfigureSe { channels } = backend.requests().last().unwrap() else {
        panic!("SE configuration")
    };
    for (index, channel) in channels[..4].iter().enumerate() {
        let channel = channel.as_ref().unwrap();
        assert_eq!(channel.volume, 16);
        assert_eq!(channel.start, index == 0);
        assert_eq!(channel.looped, index % 2 == 0);
        assert_eq!(host.channel(index).unwrap().flag, index == 0);
    }
    assert!(channels[4..].iter().all(Option::is_none));
}

#[test]
fn disabled_bgm_tracks_the_latest_request_for_reenable() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, AudioSettings, RecordingAudio};
    let mut code = vec![];
    command(&mut code, 0x20a0, &[0x8000_0000, 700, 0]);
    command(&mut code, 0x22d6, &[1, 1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"latest.ogg").unwrap();
    let mut host = AudioHost::new(AudioSettings {
        bgm_enabled: false,
        ..AudioSettings::default()
    });
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    assert!(backend.requests().is_empty());
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    let AudioRequest::PlayBgm {
        primary,
        fade_ms,
        looped,
        ..
    } = &backend.requests()[0]
    else {
        panic!("BGM restart")
    };
    assert_eq!(primary, b"latest.ogg");
    assert_eq!(*fade_ms, 0);
    assert!(*looped);
    assert!(host.bgm().playing);
}

#[test]
fn voice_switch_does_not_stop_current_audio_and_gates_future_playback() {
    use cmvs_rs::audio::{AudioHost, RecordingAudio};
    let mut code = vec![];
    command(&mut code, 0x20a6, &[0, 0x8000_0000, 256, 1]);
    command(&mut code, 0x22d8, &[1, 0]);
    command(&mut code, 0x20a6, &[0, 0x8000_0000, 256, 1]);
    command(&mut code, 0x22d8, &[1, 1]);
    command(&mut code, 0x20a6, &[0, 0x8000_0000, 256, 1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"synthetic.ogg").unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    for expected in [1, 1, 1, 1, 2] {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(backend.requests().len(), expected);
    }
}

#[test]
fn failed_audio_use_changes_keep_configuration_records_and_vm_for_retry() {
    use cmvs_rs::audio::{
        AudioBackend, AudioHost, AudioRequest, RecordingAudio, SE_CHANNELS, SePlayback,
    };
    struct Reject;
    impl AudioBackend for Reject {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn std::error::Error>> {
            unreachable!()
        }
        fn stop_bgm(&mut self) -> Result<(), Box<dyn std::error::Error>> {
            Err("synthetic stop failure".into())
        }
        fn play_bgm(&mut self, _: &AudioRequest) -> Result<(), Box<dyn std::error::Error>> {
            Err("synthetic load failure".into())
        }
        fn configure_se(
            &mut self,
            _: &[Option<SePlayback>; SE_CHANNELS],
        ) -> Result<(), Box<dyn std::error::Error>> {
            Err("synthetic group failure".into())
        }
    }
    for op in [0x22d6, 0x22d7] {
        for enabled in [false, true] {
            let mut code = vec![];
            command(&mut code, 0x20a0, &[0x8000_0000, 0, 1]);
            command(&mut code, 0x20b0, &[0, 0x8000_0000, 64, 0, 1]);
            if enabled {
                command(&mut code, op, &[1, 0]);
            }
            command(&mut code, op, &[1, u32::from(enabled)]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            vm.state_mut().set_string(0, b"synthetic.ogg").unwrap();
            let mut host = AudioHost::default();
            let mut record = RecordingAudio::default();
            for _ in 0..(2 + usize::from(enabled)) {
                pending(&mut vm);
                host.try_handle(&mut vm, &mut record).unwrap();
            }
            let settings = host.settings();
            let bgm = host.bgm().clone();
            let se = host.channel(0).cloned();
            pending(&mut vm);
            let pc = vm.pc();
            assert!(host.try_handle(&mut vm, &mut Reject).is_err());
            assert_eq!(host.settings(), settings);
            assert_eq!(*host.bgm(), bgm);
            assert_eq!(host.channel(0).cloned(), se);
            assert_eq!(vm.pc(), pc);
            assert_eq!(vm.stack().len_bytes(), 8);
            assert!(vm.pending_system().is_some());
            host.try_handle(&mut vm, &mut record).unwrap();
            assert_eq!(vm.stack().len_bytes(), 0);
        }
    }
}

#[test]
fn live_volume_setters_clamp_and_update_only_their_device_family() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, AudioSettings, RecordingAudio};
    for (op, raw, value) in [
        (0x22d9, u32::MAX, 0),
        (0x22da, 300, 256),
        (0x22db, 128, 128),
        (0x22dc, 64, 64),
        (0x22dd, 32, 32),
    ] {
        let mut code = vec![];
        command(&mut code, op, &[1, raw]);
        command(&mut code, op, &[0, 0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::new(AudioSettings::default());
        let mut backend = RecordingAudio::default();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        let AudioRequest::SetVolumes { channels } = backend.requests().last().unwrap() else {
            panic!("volume request")
        };
        for (index, channel) in channels.iter().enumerate() {
            let affected = match op {
                0x22d9 => true,
                0x22da => index < 2,
                0x22db => index >= 9,
                0x22dc => (5..9).contains(&index),
                _ => (2..5).contains(&index),
            };
            assert_eq!(*channel, affected.then_some(value));
        }
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(value));
    }
}

#[test]
fn disabled_bgm_keeps_device_gains_while_accepting_volume_configuration() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, AudioSettings, RecordingAudio};
    for op in [0x22d9, 0x22da] {
        let mut code = vec![];
        command(&mut code, op, &[1, 42]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::new(AudioSettings {
            bgm_enabled: false,
            ..AudioSettings::default()
        });
        let mut backend = RecordingAudio::default();
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        let AudioRequest::SetVolumes { channels } = backend.requests().last().unwrap() else {
            panic!("volume request")
        };
        assert_eq!(&channels[..2], &[None, None]);
        if op == 0x22da {
            assert_eq!(channels, &[None; 15]);
            assert_eq!(host.settings().bgm, 42);
        } else {
            assert_eq!(&channels[2..], &[Some(42); 13]);
            assert_eq!(host.settings().master, 42);
        }
    }
}

#[test]
fn failed_live_gain_update_preserves_configuration_stack_and_pending_command() {
    use cmvs_rs::audio::{AudioBackend, AudioHost, AudioRequest};
    struct Reject;
    impl AudioBackend for Reject {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn std::error::Error>> {
            unreachable!()
        }
        fn set_volumes(&mut self, _: &[Option<u32>; 15]) -> Result<(), Box<dyn std::error::Error>> {
            Err("synthetic failure".into())
        }
    }
    let mut code = vec![];
    command(&mut code, 0x22da, &[1, 42]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    let before = host.settings();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(host.try_handle(&mut vm, &mut Reject).is_err());
    assert_eq!(host.settings(), before);
    assert_eq!(vm.pc(), pc);
    assert_eq!(vm.stack().len_bytes(), 8);
    assert!(vm.pending_system().is_some());
}

#[test]
fn master_adjustment_and_se_adjustment_preserve_the_original_gain_asymmetry() {
    use cmvs_rs::audio::{AudioHost, AudioRequest, RecordingAudio};
    let mut code = vec![];
    command(&mut code, 0x20b0, &[0, 0x8000_0000, 64, 0, 0]);
    command(&mut code, 0x22db, &[1, 128]);
    command(&mut code, 0x22d9, &[1, 128]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"synthetic.ogg").unwrap();
    let mut host = AudioHost::default();
    let mut backend = RecordingAudio::default();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend).unwrap();
    for expected in [32, 64] {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        let AudioRequest::SetVolumes { channels } = backend.requests().last().unwrap() else {
            panic!("volume request")
        };
        assert_eq!(channels[9], Some(expected));
        assert_eq!(channels[10], Some(if expected == 32 { 128 } else { 64 }));
    }
    assert_eq!(host.channel(0).unwrap().volume, 64);
}

#[test]
fn font_name_query_writes_the_bank_and_preserves_other_results() {
    let mut code = vec![];
    for slot in [0, 7, 127, 128] {
        command(&mut code, 0x22de, &[0, 0x8000_0000 | slot]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut settings = SettingsHost::default();
    for slot in [0, 7, 127, 0] {
        vm.state_mut().set_string(slot, b"old").unwrap();
        vm.state_mut().set_system_integer(1, 77).unwrap();
        pending(&mut vm);
        assert!(settings.try_handle(&mut vm).unwrap());
        assert_eq!(
            vm.resolve_string(0x8000_0000 | slot).unwrap(),
            DEFAULT_FONT_FACE
        );
        assert_eq!(vm.state().system_integer(0), Some(1));
        assert_eq!(vm.state().system_integer(1), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn unsupported_font_change_and_destination_leave_the_command_pending() {
    for args in [[1, 0x8000_0000], [0, 0x4000_0000]] {
        let mut code = vec![];
        command(&mut code, 0x22de, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"another font").unwrap();
        vm.state_mut().set_system_integer(0, 99).unwrap();
        pending(&mut vm);
        let pc = vm.pc();
        let mut settings = SettingsHost::default();
        assert!(settings.try_handle(&mut vm).is_err());
        assert_eq!(vm.pc(), pc);
        assert!(vm.pending_system().is_some());
        assert_eq!(vm.stack().len_bytes(), 8);
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert_eq!(settings.font_face(), DEFAULT_FONT_FACE);
    }
}

#[test]
fn setting_the_current_font_does_not_change_results() {
    let mut code = vec![];
    command(&mut code, 0x22de, &[1, 0x8000_0000]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, DEFAULT_FONT_FACE).unwrap();
    vm.state_mut().set_system_integer(0, 42).unwrap();
    pending(&mut vm);
    assert!(SettingsHost::default().try_handle(&mut vm).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(42));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn automatic_advance_fields_round_trip_raw_words_and_normalize_only_the_boolean() {
    for (op, initial, expected) in [
        (0x22e6, 1, 1),
        (0x22e7, 10, u32::MAX),
        (0x22e8, 0, u32::MAX),
        (0x22f8, 0, u32::MAX),
    ] {
        let mut code = vec![];
        command(&mut code, op, &[0, 99]);
        command(&mut code, op, &[1, u32::MAX]);
        command(&mut code, op, &[0, 99]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut settings = SettingsHost::default();
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(initial));
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(initial));
        pending(&mut vm);
        settings.try_handle(&mut vm).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

#[test]
fn configuration_queries_expose_current_constructor_policy_and_setters_stay_barriers() {
    for (op, value) in [(0x22d5, 1), (0x22e9, 1)] {
        let mut code = vec![];
        command(&mut code, op, &[0, 0]);
        command(&mut code, op, &[1, 2]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut settings = SettingsHost::default();
        pending(&mut vm);
        assert!(settings.try_handle(&mut vm).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(value));
        pending(&mut vm);
        let pc = vm.pc();
        assert!(settings.try_handle(&mut vm).is_err());
        assert_eq!(vm.pc(), pc);
        assert_eq!(vm.stack().len_bytes(), 8);
    }
}

#[test]
fn audio_queries_use_playback_settings_without_backend_requests() {
    use cmvs_rs::audio::{AudioHost, AudioSettings, RecordingAudio};
    let values = [0, 1, 0, 140, 150, 160, 170, 180];
    let settings = AudioSettings {
        bgm_enabled: false,
        se_enabled: true,
        voice_enabled: false,
        master: 140,
        bgm: 150,
        se: 160,
        voice: 170,
        movie: 180,
        ..AudioSettings::default()
    };
    let mut code = vec![];
    for op in 0x22d6..=0x22dd {
        command(&mut code, op, &[0, 0]);
    }
    for op in [0x2239, 0x223b, 0x223d] {
        command(&mut code, op, &[]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::new(settings);
    let mut backend = RecordingAudio::default();
    for value in values.into_iter().chain([128, 500, 1000]) {
        pending(&mut vm);
        assert!(host.try_handle(&mut vm, &mut backend).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(value));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(host.settings(), settings);
}

#[test]
fn audio_ini_initializes_query_values_and_preserves_startup_volume_words() {
    use cmvs_rs::audio::{AudioHost, RecordingAudio};
    let ini = b"[SOUND_INFO]\nMASTER_VOLUME=224\nMUSIC_VOLUME=128\nSE_VOLUME=192\nVOICE_VOLUME=256\nVIDEO_VOLUME=224\nMUSIC_USE=0\nSE_USE=-1\nVOICE_USE=0\nVOICE_FADE_VOLOME=75\nVOICE_FADE_TIMER=600\nVOICE_FADE_WAIT=1200\n";
    let mut host = AudioHost::from_ini(ini);
    let settings = host.settings();
    assert_eq!(
        [
            settings.master,
            settings.bgm,
            settings.se,
            settings.voice,
            settings.movie
        ],
        [224, 128, 192, 256, 224]
    );
    assert!(!settings.bgm_enabled && settings.se_enabled && !settings.voice_enabled);
    let mut code = vec![];
    for op in [0x2239, 0x223b, 0x223d] {
        command(&mut code, op, &[]);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut backend = RecordingAudio::default();
    for value in [75, 600, 1200] {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(value));
    }
    assert_eq!(AudioHost::from_ini(&[]).settings().master, 100);
    assert_eq!(
        AudioHost::from_ini(b"[SOUND_INFO]\nMASTER_VOLUME=-1\n")
            .settings()
            .master,
        u32::MAX
    );
}

#[test]
fn substring_counts_complete_pairs_and_does_not_clear_an_empty_copy() {
    for (start, count, expected, characters) in [
        (0, 1, &b"\x82\xa0"[..], 1),
        (0, 3, &b"\x82\xa0A"[..], 2),
        (2, 40, &b"A\x82\xa2"[..], 2),
        (5, 40, &b"old"[..], 0),
        (0, 0, &b"old"[..], 0),
    ] {
        let mut code = vec![];
        command(&mut code, 0x20f3, &[0x8000_0001, 0x8000_0002, start, count]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(1, b"old").unwrap();
        vm.state_mut().set_string(2, b"\x82\xa0A\x82\xa2").unwrap();
        pending(&mut vm);
        assert!(cmvs_rs::text::try_handle(&mut vm).unwrap());
        assert_eq!(vm.resolve_string(0x8000_0001).unwrap(), expected);
        assert_eq!(vm.state().system_integer(0), Some(characters));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
}

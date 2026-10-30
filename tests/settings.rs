use cmvs_rs::{
    machine::Machine,
    settings::{DEFAULT_FONT_FACE, SettingsHost},
};
#[path = "support/bytecode.rs"]
mod bytecode;
use bytecode::{command, pending, script};

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
    for (op, value) in [
        (0x22d5, 1),
        (0x22df, 1),
        (0x22e0, 0),
        (0x22e1, 4),
        (0x22e2, 1),
        (0x22e3, 2),
        (0x22e4, 0),
        (0x22e9, 1),
    ] {
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

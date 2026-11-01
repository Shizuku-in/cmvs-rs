#[path = "support/shared_audio.rs"]
mod shared_audio;
use cmvs_rs::audio::{VoiceActivity, VoiceDucking, VoiceDuckingSettings};
use shared_audio::SharedAudio;

const TALKING: [VoiceActivity; 1] = [VoiceActivity {
    playing: true,
    fading_out: false,
}];
const SILENT: [VoiceActivity; 0] = [];

fn settings() -> VoiceDuckingSettings {
    VoiceDuckingSettings {
        percent: 50,
        fade_ms: 100,
        wait_ms: 200,
        include_fading: false,
    }
}
fn configured() -> VoiceDucking {
    let mut state = VoiceDucking::default();
    state.configure(settings());
    state
}

#[test]
fn voice_ducking_lowers_waits_and_restores_with_integer_gains() {
    let mut state = configured();
    for (now, voice, expected) in [
        (0, true, 256),
        (50, true, 192),
        (100, true, 128),
        (110, false, 128),
        (309, false, 128),
        (310, false, 128),
        (360, false, 192),
        (410, false, 256),
        (411, false, 256),
    ] {
        assert_eq!(
            state.update(now, if voice { &TALKING } else { &SILENT }),
            expected
        );
        assert_eq!(state.gain(), expected);
    }
}

#[test]
fn new_voice_during_restore_lowers_from_current_gain() {
    let mut state = configured();
    for (now, voice) in [(0, true), (100, true), (110, false), (310, false)] {
        state.update(now, if voice { &TALKING } else { &SILENT });
    }
    assert_eq!(state.update(360, &TALKING), 192);
    assert_eq!(state.update(410, &TALKING), 160);
    assert_eq!(state.update(460, &TALKING), 128);
}

#[test]
fn early_voice_end_finishes_lowering_and_new_voice_cancels_wait() {
    let mut state = configured();
    state.update(0, &TALKING);
    assert_eq!(state.update(50, &SILENT), 192);
    assert_eq!(state.update(100, &SILENT), 128);
    assert_eq!(state.update(110, &SILENT), 128);
    assert_eq!(state.update(310, &TALKING), 128);
    assert_eq!(state.update(500, &TALKING), 128);
    assert_eq!(state.update(510, &SILENT), 128);
    assert_eq!(state.update(709, &SILENT), 128);
    assert_eq!(state.update(710, &SILENT), 128);
    assert_eq!(state.update(760, &SILENT), 192);
}

#[test]
fn parameter_changes_wait_for_idle_and_latest_pending_values_win() {
    let mut state = configured();
    state.update(0, &TALKING);
    state.configure(VoiceDuckingSettings {
        percent: 0,
        ..settings()
    });
    state.configure(VoiceDuckingSettings {
        percent: 25,
        fade_ms: 10,
        ..settings()
    });
    assert_eq!(state.update(50, &TALKING), 192);
    assert_eq!(state.update(100, &TALKING), 128);
    state.update(110, &SILENT);
    state.update(310, &SILENT);
    assert_eq!(state.update(410, &SILENT), 256);
    assert_eq!(state.update(411, &TALKING), 256);
    assert_eq!(state.update(416, &TALKING), 160);
    assert_eq!(state.update(421, &TALKING), 64);
}

#[test]
fn fade_flag_counts_fading_voices_instead_of_disabling_ducking() {
    let fading = [VoiceActivity {
        playing: true,
        fading_out: true,
    }];
    let mut state = configured();
    assert_eq!(state.update(0, &fading), 256);
    assert_eq!(state.update(100, &fading), 256);
    // False still counts ordinary voice playback.
    state.update(101, &TALKING);
    assert_eq!(state.update(201, &TALKING), 128);
    let mut state = VoiceDucking::default();
    state.configure(VoiceDuckingSettings {
        include_fading: true,
        ..settings()
    });
    // Detection uses the active flag before pending parameters are committed.
    assert_eq!(state.update(0, &fading), 256);
    assert_eq!(state.update(100, &fading), 256);
    assert_eq!(state.update(200, &fading), 128);
}

#[test]
fn zero_durations_keep_state_transition_boundaries_and_percentage_100_is_unity() {
    let mut state = VoiceDucking::default();
    state.configure(VoiceDuckingSettings {
        percent: 0,
        fade_ms: 0,
        wait_ms: 0,
        include_fading: false,
    });
    assert_eq!(state.update(0, &TALKING), 256);
    assert_eq!(state.update(0, &TALKING), 0);
    assert_eq!(state.update(0, &SILENT), 0);
    assert_eq!(state.update(0, &SILENT), 0);
    assert_eq!(state.update(0, &SILENT), 256);
    state.configure(VoiceDuckingSettings {
        percent: u32::MAX,
        ..settings()
    });
    assert_eq!(state.update(1, &TALKING), 256);
    assert_eq!(state.update(1000, &TALKING), 256);
}

#[test]
fn millisecond_clock_wraps_and_negative_script_durations_remain_unsigned_words() {
    let mut state = configured();
    state.update(u32::MAX - 10, &TALKING);
    assert_eq!(state.update(39, &TALKING), 192);
    let mut state = VoiceDucking::default();
    state.configure(VoiceDuckingSettings {
        fade_ms: u32::MAX,
        ..settings()
    });
    state.update(0, &TALKING);
    assert_eq!(state.update(1000, &TALKING), 256);
}

#[path = "support/bytecode.rs"]
mod bytecode;

#[test]
fn ducking_commands_preserve_other_fields_results_and_exact_clamp_rules() {
    use bytecode::{command, pending, script};
    use cmvs_rs::{
        audio::{AudioHost, AudioRequest, RecordingAudio},
        machine::Machine,
    };
    for (op, raw, expected) in [
        (0x2238, u32::MAX, 0),
        (0x2238, 101, 100),
        (0x223a, u32::MAX, u32::MAX),
        (0x223a, 2001, 2000),
        (0x223c, u32::MAX, u32::MAX),
        (0x223c, 3001, 3000),
        (0x223e, u32::MAX, 1),
        (0x223e, 0, 0),
    ] {
        let mut code = vec![];
        command(&mut code, op, &[raw]);
        command(&mut code, op + 1, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = AudioHost::default();
        let mut backend = RecordingAudio::default();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        pending(&mut vm);
        host.shared_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        let mut settings = VoiceDuckingSettings {
            percent: 100,
            ..VoiceDuckingSettings::default()
        };
        match op {
            0x2238 => settings.percent = expected,
            0x223a => settings.fade_ms = expected,
            0x223c => settings.wait_ms = expected,
            _ => settings.include_fading = expected != 0,
        }
        assert_eq!(host.voice_ducking(), settings);
        assert_eq!(
            backend.requests(),
            &[AudioRequest::ConfigureVoiceDucking { settings }]
        );
        pending(&mut vm);
        host.shared_handle(&mut vm, &mut backend).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
    }
}

#[test]
fn failed_ducking_configuration_preserves_pending_command_and_parameters() {
    use bytecode::{command, pending, script};
    use cmvs_rs::{
        audio::{AudioBackend, AudioHost, AudioRequest},
        machine::Machine,
    };
    struct Reject;
    impl AudioBackend for Reject {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn std::error::Error>> {
            unreachable!()
        }
        fn configure_voice_ducking(
            &mut self,
            _: VoiceDuckingSettings,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Err("synthetic failure".into())
        }
    }
    let mut code = vec![];
    command(&mut code, 0x2238, &[42]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = AudioHost::default();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(host.shared_handle(&mut vm, &mut Reject).is_err());
    assert_eq!(vm.pc(), pc);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert!(vm.pending_system().is_some());
    assert_eq!(host.voice_ducking(), VoiceDuckingSettings::default());
}

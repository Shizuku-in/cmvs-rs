//! cmvs-20070119 variants on synthetic scripts; see docs/games/puriring.
use cmvs_rs::{
    audio::{AudioHost, AudioRequest, RecordingAudio},
    camera::CameraHost,
    dispatch::{self, AudioHosts, Flow, Hosts},
    machine::Machine,
    profile::{Registry, Semantics, VariantId},
};
mod support;
use support::bytecode::{command, pending, script};

const PROFILE: &str = "cmvs-20070119";

fn semantics(command: u16) -> Semantics {
    Registry::builtin()
        .profile(PROFILE)
        .unwrap()
        .semantics(command)
}

#[test]
fn activation_checks_report_success_only_for_mode_one_without_other_effects() {
    for opcode in [0x22c0u16, 0x22c1] {
        assert_eq!(
            semantics(opcode - 0x2000),
            Semantics::Variant(VariantId::ActivationPass)
        );
        // Other builds keep their own semantics for these numbers.
        assert_eq!(
            Registry::builtin()
                .profile("cmvs-20071117")
                .unwrap()
                .semantics(opcode - 0x2000),
            Semantics::Root
        );
        for (mode, expected) in [(0, 0), (1, 1), (2, 0), (u32::MAX, 0)] {
            let mut code = vec![];
            command(&mut code, opcode, &[mode]);
            let program = script(&code);
            let mut vm = Machine::new(&program).unwrap();
            pending(&mut vm);
            vm.state_mut().set_system_integer(0, 77).unwrap();
            vm.state_mut().set_system_integer(1, 88).unwrap();
            let flow = dispatch::run(
                semantics(opcode - 0x2000),
                &mut vm,
                &mut Hosts {
                    math: true,
                    ..Default::default()
                },
            );
            assert_eq!(flow.unwrap(), Some(Flow::Done));
            assert_eq!(vm.stack().len_bytes(), 0);
            assert_eq!(vm.state().system_integer(0), Some(expected));
            assert_eq!(vm.state().system_integer(1), Some(88));
        }
    }
}

#[test]
fn mixer_gain_is_clamped_to_256_and_reaches_the_backend() {
    assert_eq!(semantics(0xa5), Semantics::Variant(VariantId::MixerGain));
    for (raw, gain) in [(0, 0), (128, 128), (256, 256), (257, 256), (u32::MAX, 256)] {
        let mut code = vec![];
        command(&mut code, 0x20a5, &[raw]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 5).unwrap();
        let mut audio = AudioHost::default();
        let mut backend = RecordingAudio::default();
        let flow = dispatch::run(
            semantics(0xa5),
            &mut vm,
            &mut Hosts {
                audio: Some(AudioHosts {
                    audio: &mut audio,
                    backend: &mut backend,
                }),
                ..Default::default()
            },
        );
        assert_eq!(flow.unwrap(), Some(Flow::Done));
        assert_eq!(backend.requests(), &[AudioRequest::SetMixerGain { gain }]);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(5));
    }
}

fn run_camera(camera: &mut CameraHost, opcode: u16, args: &[u32]) -> Machine<'static> {
    let mut code = vec![];
    command(&mut code, opcode, args);
    let program = Box::leak(Box::new(script(&code)));
    let mut vm = Machine::new(program).unwrap();
    pending(&mut vm);
    let flow = dispatch::run(
        semantics(opcode - 0x2000),
        &mut vm,
        &mut Hosts {
            camera: Some(camera),
            ..Default::default()
        },
    );
    assert_eq!(flow.unwrap(), Some(Flow::Done));
    assert_eq!(vm.stack().len_bytes(), 0);
    vm
}

fn view_words(camera: &CameraHost) -> [f32; 7] {
    camera.view(0).unwrap().words.map(f32::from_bits)
}

#[test]
fn view_motion_interpolates_one_group_and_jumps_to_the_target_at_the_end() {
    let mut camera = CameraHost::default();
    let before = view_words(&camera);
    run_camera(
        &mut camera,
        0x2274,
        &[0, 10.0f32.to_bits(), 20.0f32.to_bits(), 100],
    );
    assert!(camera.view_moving());
    camera.advance_frame(33);
    let words = view_words(&camera);
    // Linear offsets are truncated to hundredths: 10 * 33 / 100 = 3.3, 20 * 0.33 = 6.6.
    assert_eq!([words[3], words[4]], [3.3, 6.6]);
    assert_eq!(&words[..3], &before[..3]);
    assert_eq!(&words[5..], &before[5..]);
    let vm = run_camera(&mut camera, 0x2272, &[]);
    assert_eq!(vm.state().system_integer(0), Some(1));
    camera.advance_frame(67);
    assert!(!camera.view_moving());
    assert_eq!(
        [view_words(&camera)[3], view_words(&camera)[4]],
        [10.0, 20.0]
    );
    let vm = run_camera(&mut camera, 0x2272, &[]);
    assert_eq!(vm.state().system_integer(0), Some(0));
}

#[test]
fn view_motion_eases_and_stops_with_or_without_finishing() {
    let mut camera = CameraHost::default();
    let start = view_words(&camera);
    let target = [start[0] + 10.0, start[1], start[2] - 10.0];
    let mut args = vec![1];
    args.extend(target.map(f32::to_bits));
    args.push(200);
    run_camera(&mut camera, 0x2273, &args);
    camera.advance_frame(100);
    let words = view_words(&camera);
    // Mode 1 at half time: sin(45 degrees) * 10, truncated to hundredths.
    assert_eq!(words[0], start[0] + 7.07);
    assert_eq!(words[2], start[2] - 7.07);
    run_camera(&mut camera, 0x2270, &[0]);
    assert!(!camera.view_moving());
    assert_eq!(view_words(&camera)[0], start[0] + 7.07);

    run_camera(&mut camera, 0x2273, &args);
    camera.advance_frame(50);
    run_camera(&mut camera, 0x2270, &[1]);
    assert_eq!(&view_words(&camera)[..3], &target);
}

#[test]
fn separate_image_tables_and_window_scale_motion_are_bound_only_here() {
    let registry = Registry::builtin();
    let profile = registry.profile(PROFILE).unwrap();
    for command in [0x17, 0x18, 0x32] {
        assert_eq!(
            profile.semantics(command),
            Semantics::Variant(VariantId::ImageTables)
        );
    }
    for command in 0x1c0..=0x1c2 {
        assert_eq!(
            profile.semantics(command),
            Semantics::Variant(VariantId::WindowScaleMotion)
        );
    }
    for command in 0x270..=0x274 {
        assert_eq!(
            profile.semantics(command),
            Semantics::Variant(VariantId::ViewMotion)
        );
        assert!(
            registry
                .profiles()
                .iter()
                .filter(|p| p.id != PROFILE)
                .all(|p| p.semantics(command) == Semantics::Root)
        );
    }
    assert!(dispatch::is_wait_for(profile.semantics(0x271), 0x271));
    assert!(!dispatch::is_wait(0x271));
}

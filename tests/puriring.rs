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

#[test]
fn sound_preloading_effect_creators_configure_audio_paths_first() {
    // Walk, circle, magic and sprites preload sounds by name.
    for command in [0x291, 0x292, 0x294, 0x295] {
        assert!(dispatch::loads_sounds_for(semantics(command), command));
        assert!(!dispatch::loads_sounds_for(Semantics::Root, command));
    }
    for command in [0x290, 0x293, 0x296, 0x297, 0x2a0, 0x2a3, 0x2ae, 0x2af] {
        assert!(!dispatch::loads_sounds_for(semantics(command), command));
    }
    assert!(dispatch::loads_sounds_for(semantics(0xa0), 0xa0));
}

#[test]
fn floating_planes_use_command_296_and_consume_two_words_for_an_invalid_slot() {
    use cmvs_rs::{dispatch::ImageHosts, image::ImageSource, scene::SceneHost};
    struct NoImages;
    impl ImageSource for NoImages {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            panic!("invalid slots must not read images");
        }
    }
    assert_eq!(
        semantics(0x296),
        Semantics::Variant(VariantId::LegacyEffects)
    );
    assert_eq!(
        semantics(0x295),
        Semantics::Variant(VariantId::LegacyEffects)
    );
    let mut code = vec![];
    command(&mut code, 0x2296, &[4, 123]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 77).unwrap();
    let mut scene = SceneHost::default();
    let mut source = NoImages;
    let mut audio = AudioHost::default();
    let mut backend = RecordingAudio::default();
    let flow = dispatch::run(
        semantics(0x296),
        &mut vm,
        &mut Hosts {
            images: Some(ImageHosts {
                scene: &mut scene,
                source: &mut source,
                change_images: false,
            }),
            audio: Some(AudioHosts {
                audio: &mut audio,
                backend: &mut backend,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(flow, Some(Flow::Done));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(77));
    assert!(scene.legacy_effects().is_empty());
    assert!(backend.requests().is_empty());
}

#[test]
fn vortex_binding_uses_one_word_and_keeps_failed_creation_pending() {
    use cmvs_rs::{dispatch::ImageHosts, image::ImageSource, random::CrtRandom, scene::SceneHost};
    struct FailingImages(usize);
    impl ImageSource for FailingImages {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            self.0 += 1;
            Err("synthetic image failure".into())
        }
    }
    assert_eq!(
        semantics(0x297),
        Semantics::Variant(VariantId::LegacyEffects)
    );
    assert_eq!(
        Registry::builtin()
            .profile("cmvs-20071117")
            .unwrap()
            .semantics(0x297),
        Semantics::Root
    );
    for slot in [4, u32::MAX, 0] {
        let mut code = vec![];
        command(&mut code, 0x2297, &[slot]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 77).unwrap();
        let before = (vm.pc(), vm.stack().len_bytes());
        let mut scene = SceneHost::default();
        let mut images = FailingImages(0);
        let mut random = CrtRandom::new(0);
        let mut camera = CameraHost::default();
        let old_camera = camera.view(0).unwrap().words;
        let mut audio = AudioHost::default();
        let mut backend = RecordingAudio::default();
        let out = dispatch::run(
            semantics(0x297),
            &mut vm,
            &mut Hosts {
                images: Some(ImageHosts {
                    scene: &mut scene,
                    source: &mut images,
                    change_images: false,
                }),
                audio: Some(AudioHosts {
                    audio: &mut audio,
                    backend: &mut backend,
                }),
                camera: Some(&mut camera),
                random: Some(&mut random),
                ..Default::default()
            },
        );
        assert_eq!(out.is_err(), slot == 0);
        assert_eq!(images.0, usize::from(slot == 0));
        assert_eq!(random, CrtRandom::new(0));
        assert_eq!(camera.view(0).unwrap().words, old_camera);
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert!(scene.legacy_effects().is_empty());
        if slot == 0 {
            assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
        } else {
            assert_eq!(out.unwrap(), Some(Flow::Done));
            assert_eq!(vm.stack().len_bytes(), 0);
        }
    }
}

#[test]
#[ignore = "requires CMVS_GAME_ROOT pointing to read-only PURIRING resources"]
fn external_vortex_creates_switches_renders_and_releases() {
    use cmvs_rs::{
        audio::AudioSettings, dispatch::ImageHosts, effect_objects::LegacyEffect,
        image::ImageArchives, random::CrtRandom, scene::SceneHost, scene_render,
    };
    let root =
        std::path::PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let mut archives = ImageArchives::new(&root).unwrap();
    let paths = [
        "up.cpz",
        "event.cpz",
        "bg.cpz",
        "chip.cpz",
        "stand.cpz",
        "face.cpz",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| ((0x2011, i as u32), format!("data/pack/{name}").into_bytes()))
    .collect();
    let mut images = archives.with_paths(&paths);
    let mut scene = SceneHost::default();
    let profile = Registry::builtin().profile(PROFILE).unwrap();
    let mut camera = CameraHost::for_profile(profile);
    let mut random = CrtRandom::new(0);
    let mut audio = AudioHost::default();
    let mut backend = RecordingAudio::default();
    let mut code = vec![];
    command(&mut code, 0x2297, &[1]);
    command(&mut code, 0x22af, &[1, 10, 123, 456]);
    command(&mut code, 0x22ae, &[1]);
    command(&mut code, 0x22a0, &[1]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 77).unwrap();
    assert_eq!(
        dispatch::run(
            semantics(0x297),
            &mut vm,
            &mut Hosts {
                images: Some(ImageHosts {
                    scene: &mut scene,
                    source: &mut images,
                    change_images: false
                }),
                audio: Some(AudioHosts {
                    audio: &mut audio,
                    backend: &mut backend
                }),
                camera: Some(&mut camera),
                random: Some(&mut random),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(vm.state().system_integer(0), Some(77));
    let first = scene_render::render_scene(&scene, Some(&camera), &[], 1024, 614).unwrap();
    pending(&mut vm);
    assert_eq!(
        dispatch::run(
            semantics(0x2af),
            &mut vm,
            &mut Hosts {
                scene: Some(&mut scene),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    scene
        .advance_legacy_effects(200, &mut camera, AudioSettings::default(), &mut backend)
        .unwrap();
    let second = scene_render::render_scene(&scene, Some(&camera), &[], 1024, 614).unwrap();
    assert_ne!(first.pixels(), second.pixels());
    assert!(
        second
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[..3] != [0, 0, 0])
    );
    let LegacyEffect::Vortex(vortex) = &scene.legacy_effects()[&1] else {
        panic!("wrong type");
    };
    assert_eq!(vortex.scene.meshes.len(), 200);
    assert_eq!(
        vortex.scene.images[vortex.scene.meshes[2].image].name,
        b"__ev-xx004e.pb3"
    );
    assert_eq!(
        vortex.scene.images[vortex.scene.meshes[3].image].name,
        b"__ev-xx004b2.pb3"
    );
    pending(&mut vm);
    dispatch::run(
        semantics(0x2ae),
        &mut vm,
        &mut Hosts {
            scene: Some(&mut scene),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    pending(&mut vm);
    dispatch::run(
        semantics(0x2a0),
        &mut vm,
        &mut Hosts {
            scene: Some(&mut scene),
            audio: Some(AudioHosts {
                audio: &mut audio,
                backend: &mut backend,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(scene.legacy_effects().is_empty());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert!(backend.requests().is_empty());
}

#[test]
fn hundredth_degree_polar_point_preserves_other_results_and_signed_radius() {
    assert_eq!(semantics(0xd9), Semantics::Variant(VariantId::PolarPoint));
    for (angle, radius, expected) in [
        (0i32, -100i32, [-100, 0]),
        (4500, 100, [70, 70]),
        (-4500, 100, [70, -70]),
        (4500, -100, [-70, -70]),
    ] {
        let mut code = vec![];
        command(&mut code, 0x20d9, &[angle as u32, radius as u32]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 77).unwrap();
        vm.state_mut().set_system_integer(3, 88).unwrap();
        assert_eq!(
            dispatch::run(
                semantics(0xd9),
                &mut vm,
                &mut Hosts {
                    math: true,
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.state().system_integer(3), Some(88));
        for (i, value) in expected.into_iter().enumerate() {
            assert_eq!(
                vm.state().system_integer((i + 1) as u32),
                Some(value as u32)
            );
        }
    }
}

#[test]
#[ignore = "requires CMVS_GAME_ROOT and CMVS_PURIRING_SAVE pointing to read-only game resources and a CSV2 save"]
fn external_snapshot_load_restores_scene_and_reencodes() {
    use cmvs_rs::{
        platform::RecordingBackend,
        runner::{Startup, StartupEvent, load_start},
        save::Csv2Save,
        snapshot::{EngineSnapshot, SnapshotLayout},
    };
    let root =
        std::path::PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let input = std::path::PathBuf::from(
        std::env::var_os("CMVS_PURIRING_SAVE").expect("CMVS_PURIRING_SAVE"),
    );
    let bytes = std::fs::read(input).unwrap();
    let save = Csv2Save::decode(&bytes).unwrap();
    let layout = SnapshotLayout::TextV1SceneLegacy;
    let snapshot = EngineSnapshot::decode_with_layout(&save.snapshot, layout).unwrap();
    assert_eq!(snapshot.encode_with_layout(layout).unwrap(), save.snapshot);
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    assert_eq!(game.selection.profile.id, PROFILE);
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    let user = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "puriring-load-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&user).unwrap();
    std::fs::write(user.join("save003.dat"), bytes).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    let mut backend = RecordingBackend::default();
    let mut entered = false;
    for _ in 0..1_000_000 {
        match run.tick(&mut backend).unwrap() {
            StartupEvent::SnapshotCaptured => {
                entered = true;
                break;
            }
            StartupEvent::Yield => run.advance_frame(16).unwrap(),
            StartupEvent::Progress => {}
            event => panic!("unexpected bootstrap event: {event:?}"),
        }
    }
    assert!(entered);
    run.load_slot(3, &mut backend).unwrap();
    run.render_frame().unwrap();
    assert!(run.save_slot(4).unwrap());
    let second = Csv2Save::decode(&std::fs::read(user.join("save004.dat")).unwrap()).unwrap();
    assert_eq!(second.snapshot, save.snapshot);
    // Exercise the restored VM, not just its cached save image. Each decide release
    // is delivered at a message wait; scene clocks advance at real frame boundaries.
    let mut traces = Vec::new();
    for _ in 0..2 {
        run.input_mut().set(0, false, false, false).unwrap();
        run.load_slot(3, &mut backend).unwrap();
        let bindings = cmvs_rs::input::KeyBindings::parse(
            &std::fs::read_to_string(root.join("key.cfg")).unwrap(),
        )
        .unwrap();
        let mut codes = cmvs_rs::input::CodeInput::default();
        let mut taps = std::collections::VecDeque::new();
        run.input_mut().set_pointer(-1, -1);
        let mut trace = Vec::new();
        let mut was_waiting = false;
        for _ in 0..2_000_000 {
            match run.tick(&mut backend).unwrap() {
                StartupEvent::Yield => {
                    run.advance_frame(16).unwrap();
                    let waiting = run
                        .machine()
                        .pending_system()
                        .is_some_and(|(_, c)| c == 0x153)
                        && run.machine().repeat_count() >= 2;
                    if waiting && !was_waiting {
                        let history = run.history().entries().front().map(|h| h.strings.clone());
                        trace.push((
                            run.machine().current_name().to_vec(),
                            run.machine().pc(),
                            history,
                        ));
                    }
                    was_waiting = waiting;
                    if waiting && taps.is_empty() {
                        taps.extend([true, false]);
                    }
                    let mut now = [false; 128];
                    now[cmvs_rs::input::MOUSE_CODES[0]] = taps.pop_front().unwrap_or(false);
                    codes.update(&bindings, &now, run.input_mut());
                    if trace.len() >= 20 {
                        break;
                    }
                }
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                event => panic!("unexpected restored event: {event:?}"),
            }
        }
        assert_eq!(
            trace.len(),
            20,
            "restored execution stopped at {:?}/{:#x}",
            run.machine().current_name(),
            run.machine().pc()
        );
        traces.push(trace);
    }
    assert!(
        traces[0] == traces[1],
        "repeated loads must resume the same 20 message waits and backlog texts"
    );
    drop(run);
    std::fs::remove_dir_all(user).unwrap();
}

#[test]
fn window_full_move_queries_ignore_the_value_and_writes_preserve_system_results() {
    use cmvs_rs::{
        dispatch::PlatformHosts,
        platform::{DirectoryAssets, PlatformHost, RecordingBackend},
    };
    let variant = Semantics::Variant(VariantId::WindowFullMove);
    assert_eq!(semantics(0x2ec), variant);
    assert_eq!(
        Registry::builtin()
            .profile("cmvs-20071117")
            .unwrap()
            .semantics(0x2ec),
        Semantics::Root
    );
    let mut host = PlatformHost::default();
    host.configure_window_full_move(b"[SYSTEM_INFO]\nWINDOW_FULL_MOVE_USE=-1\n");
    assert!(host.window_full_move());
    let mut backend = RecordingBackend::default();
    let mut assets = DirectoryAssets::new(std::path::Path::new(".")).unwrap();
    for (mode, value, expected) in [
        (0, u32::MAX, true),
        (1, 0, false),
        (0, u32::MAX, false),
        (u32::MAX, 2, true),
    ] {
        let mut code = vec![];
        command(&mut code, 0x22ec, &[mode, value]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 77).unwrap();
        vm.state_mut().set_system_integer(1, 88).unwrap();
        assert_eq!(
            dispatch::run(
                variant,
                &mut vm,
                &mut Hosts {
                    platform: Some(PlatformHosts {
                        host: &mut host,
                        backend: &mut backend,
                        assets: &mut assets,
                        cursors: &[]
                    }),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(host.window_full_move(), expected);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(
            vm.state().system_integer(0),
            Some(if mode == 0 { u32::from(expected) } else { 77 })
        );
        assert_eq!(vm.state().system_integer(1), Some(88));
    }
}

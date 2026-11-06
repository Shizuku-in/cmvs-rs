use cmvs_rs::{
    dispatch::{self, Flow, Hosts, ImageHosts},
    image::ImageSource,
    machine::Machine,
    profile::Semantics,
    random::CrtRandom,
    scene::SceneHost,
};
mod support;
use support::bytecode::{command, pending, script};
struct Images;
impl ImageSource for Images {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        Ok(Some(b"NONE".to_vec()))
    }
}

#[test]
fn base_falling_and_fragments_share_slots_rng_query_and_release() {
    let mut code = Vec::new();
    command(
        &mut code,
        0x22a2,
        &[
            0,
            0x8000_0000,
            10f32.to_bits(),
            8f32.to_bits(),
            12000f32.to_bits(),
            6f32.to_bits(),
            2,
            1,
        ],
    );
    command(&mut code, 0x22ae, &[0]);
    command(&mut code, 0x2020, &[29]);
    command(&mut code, 0x22a8, &[0, 29, 20, 1000]);
    command(&mut code, 0x22ae, &[0]);
    command(&mut code, 0x22ae, &[0]);
    command(&mut code, 0x22a0, &[0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"snow.pb3").unwrap();
    let mut scene = SceneHost::default();
    let mut random = CrtRandom::new(0);
    pending(&mut vm);
    let before = (vm.pc(), vm.stack().len_bytes());
    assert_eq!(
        dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
        Some(Flow::Unavailable)
    );
    assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
    assert_eq!(
        dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                images: Some(ImageHosts {
                    scene: &mut scene,
                    source: &mut Images,
                    change_images: false
                }),
                random: Some(&mut random),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(scene.falling_effects()[&0].particles.len(), 2);
    let mut expected = CrtRandom::new(0);
    for _ in 0..8 {
        expected.next_value();
    }
    assert_eq!(random, expected);
    for index in 0..6 {
        pending(&mut vm);
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    scene: Some(&mut scene),
                    random: Some(&mut random),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 0);
        match index {
            0 => assert_eq!(vm.state().system_integer(0), Some(u32::MAX)),
            2 => {
                assert!(scene.falling_effects().is_empty());
                assert!(scene.object(29).is_none());
            }
            3 => {
                assert_eq!(vm.state().system_integer(0), Some(0));
                scene
                    .advance_effect_pools(16, &mut random, &Default::default())
                    .unwrap();
            }
            4 => assert_eq!(vm.state().system_integer(0), Some(u32::MAX)),
            5 => assert!(scene.fragment_effects().is_empty()),
            _ => {}
        }
    }
}

#[test]
fn object_effect_errors_are_transactional_and_reset_keeps_ordinary_slots() {
    let mut setup = Vec::new();
    for (op, args) in [
        (0x2020, vec![29]),
        (0x2040, vec![29]),
        (0x2044, vec![29, u32::MAX, 0, 0, 4, 3]),
        (0x2022, vec![29, 1]),
        (0x2050, vec![29, 1]),
        (0x2328, vec![29, 1, 0, 0, 500, 1, 0, 0, 250, 1000]),
    ] {
        command(&mut setup, op, &args);
    }
    let program = script(&setup);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = SceneHost::default();
    for _ in 0..6 {
        pending(&mut vm);
        assert!(scene.try_handle(&mut vm).unwrap());
    }
    assert_eq!(
        scene
            .object(29)
            .unwrap()
            .special_effect
            .as_ref()
            .unwrap()
            .children
            .len(),
        3
    );
    for args in [
        vec![],
        vec![29],
        vec![29, 1, 0, 0, 500, 0, 0, 0, 250, 1000],
        vec![29, 1, 0, 0, 500, 1, 0, 0, 250, 0],
        vec![29, 16, 0, 0, 0, 1, 1025, 0, 0, 0],
        vec![256, 16, 0, 0, 0, 1, 1, 0, 0, 0],
    ] {
        let mut code = Vec::new();
        command(&mut code, 0x2328, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        let objects = scene.objects().clone();
        let pending_before = vm.pending_system();
        assert!(scene.try_handle(&mut vm).is_err());
        assert_eq!(scene.objects(), &objects);
        assert_eq!(vm.pending_system(), pending_before);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
    let parent = scene.object(29).unwrap().clone();
    let mut code = Vec::new();
    command(&mut code, 0x2329, &[29]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    let actual = scene.object(29).unwrap();
    assert_eq!(actual.special_effect.as_ref().unwrap().words[0], 0);
    assert_eq!(actual.children, parent.children);
    assert_eq!(actual.draw, parent.draw);
    assert_eq!(actual.flags, parent.flags);
    // Destroying the drawing state after RESET is valid: the inactive header
    // may retain parameter words, but must not require a drawing state to tick.
    let mut code = Vec::new();
    command(&mut code, 0x2042, &[29, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    scene.advance_frame(16);
}

#[test]
#[ignore = "requires explicit read-only CMVS_GAME_ROOT; invokes original library functions with synthetic scene/resources"]
fn external_original_effect_library_functions_use_base_dispatch() {
    use cmvs_rs::{
        camera::CameraHost,
        machine::Step,
        profile::Registry,
        script_control::{ArchiveScripts, ScriptSource},
    };
    let root =
        std::path::PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let registry = Registry::builtin();
    let game = registry.identify_game(&root, None).unwrap();
    assert_eq!(game.selection.profile.id, "cmvs-20160201");
    let mut source =
        ArchiveScripts::new(std::fs::File::open(root.join("data/pack/script.cpz")).unwrap())
            .unwrap();
    let original = source.load(b"intproc.ps3").unwrap();
    let labels = original.labels();
    assert_eq!((labels[39], labels[173]), (0x8e02, 0x2e92a));
    for (label, parameter, expected) in [(39, None, 0x2a2), (173, Some(1u32), 0x328)] {
        let mut scene = SceneHost::default();
        let mut setup = Vec::new();
        for (op, args) in [
            (0x2020, vec![29]),
            (0x2040, vec![29]),
            (0x2044, vec![29, u32::MAX, 0, 0, 4, 3]),
        ] {
            command(&mut setup, op, &args);
        }
        let setup = script(&setup);
        let mut setup_vm = Machine::new(&setup).unwrap();
        for _ in 0..3 {
            pending(&mut setup_vm);
            scene.try_handle(&mut setup_vm).unwrap();
        }
        let mut vm = Machine::at(&original, labels[label]).unwrap();
        vm.set_current_name(b"intproc.ps3".to_vec());
        vm.state_mut().set_integer(110, 0).unwrap();
        let mut image = vm.capture_vm().unwrap();
        let sp = if parameter.is_some() { 8 } else { 4 };
        image.stack_pointer = sp;
        image.frame_depth = 1;
        image.frames[1] = sp;
        if let Some(value) = parameter {
            image.stack[..4].copy_from_slice(&value.to_le_bytes());
        }
        image.stack[sp as usize - 4..sp as usize].copy_from_slice(&0u32.to_le_bytes());
        vm.restore_vm(&image).unwrap();
        let mut random = CrtRandom::new(0);
        let mut camera = CameraHost::default();
        let mut hits = Vec::new();
        for _ in 0..2000 {
            if vm.pc() == 0 {
                break;
            }
            if let Step::System { pc, command } = vm.step().unwrap() {
                if command == expected {
                    hits.push(pc);
                }
                let flow = if dispatch::uses_images(command) {
                    dispatch::run(
                        Semantics::Root,
                        &mut vm,
                        &mut Hosts {
                            images: Some(ImageHosts {
                                scene: &mut scene,
                                source: &mut Images,
                                change_images: false,
                            }),
                            random: Some(&mut random),
                            ..Default::default()
                        },
                    )
                } else {
                    dispatch::run(
                        Semantics::Root,
                        &mut vm,
                        &mut Hosts {
                            scene: Some(&mut scene),
                            camera: Some(&mut camera),
                            ..Default::default()
                        },
                    )
                }
                .unwrap();
                assert_eq!(
                    flow,
                    Some(Flow::Done),
                    "label {label} opcode {:04x}",
                    command + 0x2000
                );
            }
        }
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(
            hits,
            if label == 39 {
                vec![0x8ed4]
            } else {
                vec![0x2e9da]
            }
        );
        if label == 39 {
            assert_eq!(scene.falling_effects()[&0].particles.len(), 256);
            assert_eq!(vm.state().integer(408), Some(1));
        } else {
            assert_eq!(
                scene
                    .object(29)
                    .unwrap()
                    .special_effect
                    .as_ref()
                    .unwrap()
                    .children
                    .len(),
                3
            );
            assert_eq!(vm.state().integer(490), Some(1));
        }
        println!("original label{label}: hits={hits:x?}, returned with empty stack");
    }
}

use cmvs_rs::{
    dispatch::{self, Flow, Hosts, ImageHosts},
    image::ImageSource,
    machine::Machine,
    profile::{ResearchId, Semantics},
    random::CrtRandom,
    scene::SceneHost,
};
mod support;
use support::bytecode::{command, pending, script};

#[test]
fn fragment_dispatch_transfers_source_and_preserves_pending_when_random_is_missing() {
    let mut code = vec![];
    command(&mut code, 0x2020, &[29]);
    command(&mut code, 0x22a8, &[0, 29, 20, 1000]);
    command(&mut code, 0x22a0, &[0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut scene = SceneHost::default();
    pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    pending(&mut vm);
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    assert_eq!(
        dispatch::run(
            Semantics::Research(ResearchId::Fragments),
            &mut vm,
            &mut Hosts {
                scene: Some(&mut scene),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Unavailable)
    );
    assert_eq!(
        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
        before
    );
    assert!(scene.object(29).is_some());
    let mut random = CrtRandom::new(0);
    dispatch::run(
        Semantics::Research(ResearchId::Fragments),
        &mut vm,
        &mut Hosts {
            scene: Some(&mut scene),
            random: Some(&mut random),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(scene.object(29).is_none());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert!(scene.fragment_effects()[&0].fragments.is_empty());
    assert_eq!(random, CrtRandom::new(0));
    pending(&mut vm);
    dispatch::run(
        Semantics::Root,
        &mut vm,
        &mut Hosts {
            scene: Some(&mut scene),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(scene.fragment_effects().is_empty());
}

#[derive(Default)]
struct Images {
    reads: usize,
    fail: bool,
}
impl ImageSource for Images {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        assert_eq!(name, b"particle.pb3");
        self.reads += 1;
        if self.fail {
            return Err("synthetic read failure".into());
        }
        // Unknown magic follows the original quiet image-load failure, without skipping particles.
        Ok(Some(b"NONE".to_vec()))
    }
}
fn args(slot: u32, count: u32) -> Vec<u32> {
    vec![
        slot,
        0x80000000,
        0,
        40.0f32.to_bits(),
        20.0f32.to_bits(),
        (-20.0f32).to_bits(),
        2000.0f32.to_bits(),
        count,
    ]
}
fn run(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    source: &mut Images,
    random: &mut CrtRandom,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    dispatch::run(
        Semantics::Research(ResearchId::Particles),
        vm,
        &mut Hosts {
            images: Some(ImageHosts {
                scene,
                source,
                change_images: false,
            }),
            random: Some(random),
            ..Default::default()
        },
    )
}

#[test]
fn research_creation_needs_images_and_shared_random_and_never_uses_root() {
    let mut code = vec![];
    command(&mut code, 0x22a5, &args(0, 2));
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"particle.pb3").unwrap();
    pending(&mut vm);
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    let mut scene = SceneHost::default();
    let mut source = Images::default();
    let mut random = CrtRandom::new(0);
    assert_eq!(
        dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
        None
    );
    for images in [false, true] {
        let mut hosts = Hosts::default();
        if images {
            hosts.images = Some(ImageHosts {
                scene: &mut scene,
                source: &mut source,
                change_images: false,
            });
        } else {
            hosts.random = Some(&mut random);
        }
        assert_eq!(
            dispatch::run(
                Semantics::Research(ResearchId::Particles),
                &mut vm,
                &mut hosts
            )
            .unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
    }
    assert_eq!(
        run(&mut vm, &mut scene, &mut source, &mut random).unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(scene.particle_effects()[&0].particles.len(), 2);
    assert!(scene.objects().is_empty());
    let mut expected = CrtRandom::new(0);
    for _ in 0..16 {
        expected.next_value();
    }
    assert_eq!(random, expected);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(
        run(&mut vm, &mut scene, &mut source, &mut random).unwrap(),
        None
    );
    assert_eq!(source.reads, 1);
}

#[test]
fn replacement_failure_preserves_old_pool_random_and_pending_then_retries_once() {
    let mut code = vec![];
    command(&mut code, 0x22a5, &args(2, 1));
    command(&mut code, 0x22a5, &args(2, 3));
    command(&mut code, 0x22a0, &[2]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"particle.pb3").unwrap();
    let mut scene = SceneHost::default();
    let mut source = Images::default();
    let mut random = CrtRandom::new(7);
    pending(&mut vm);
    run(&mut vm, &mut scene, &mut source, &mut random).unwrap();
    pending(&mut vm);
    let before = (
        format!("{:?}", scene.particle_effects()),
        random.clone(),
        vm.pc(),
        vm.stack().len_bytes(),
    );
    source.fail = true;
    assert!(run(&mut vm, &mut scene, &mut source, &mut random).is_err());
    assert_eq!(
        (
            format!("{:?}", scene.particle_effects()),
            random.clone(),
            vm.pc(),
            vm.stack().len_bytes()
        ),
        before
    );
    assert!(vm.pending_system().is_some());
    source.fail = false;
    run(&mut vm, &mut scene, &mut source, &mut random).unwrap();
    assert_eq!(scene.particle_effects()[&2].particles.len(), 3);
    pending(&mut vm);
    assert_eq!(
        dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                scene: Some(&mut scene),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(Flow::Done)
    );
    assert!(scene.particle_effects().is_empty());
    let before = random.clone();
    scene.advance_particles(500, &mut random);
    assert_eq!(random, before);
}

#[test]
fn invalid_slots_still_resolve_names_but_skip_allocation_and_resource_reads() {
    let mut code = vec![];
    command(&mut code, 0x22a5, &args(u32::MAX, u32::MAX));
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut scene = SceneHost::default();
    let mut source = Images::default();
    let mut random = CrtRandom::new(4);
    assert!(run(&mut vm, &mut scene, &mut source, &mut random).is_err());
    vm.state_mut().set_string(0, b"particle.pb3").unwrap();
    assert_eq!(
        run(&mut vm, &mut scene, &mut source, &mut random).unwrap(),
        Some(Flow::Done)
    );
    assert_eq!(source.reads, 0);
    assert_eq!(random, CrtRandom::new(4));
    assert!(scene.particle_effects().is_empty());
}

#[test]
fn malformed_parameters_and_short_stack_fail_before_random_or_resources() {
    for (index, value) in [(7, 16_385), (6, 0), (3, f32::NAN.to_bits())] {
        let mut arguments = args(0, 1);
        arguments[index] = value;
        let mut code = vec![];
        command(&mut code, 0x22a5, &arguments);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"particle.pb3").unwrap();
        pending(&mut vm);
        let mut scene = SceneHost::default();
        let mut source = Images::default();
        let mut random = CrtRandom::new(4);
        assert!(run(&mut vm, &mut scene, &mut source, &mut random).is_err());
        assert_eq!(vm.stack().len_bytes(), 32);
        assert!(vm.pending_system().is_some());
        assert!(scene.particle_effects().is_empty());
        assert_eq!(source.reads, 0);
        assert_eq!(random, CrtRandom::new(4));
    }
    let mut code = vec![];
    command(&mut code, 0x22a5, &[0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    assert!(
        run(
            &mut vm,
            &mut SceneHost::default(),
            &mut Images::default(),
            &mut CrtRandom::new(0)
        )
        .is_err()
    );
    assert_eq!(vm.stack().len_bytes(), 4);
}

#[test]
fn trace_reports_the_selected_particle_entry_and_requires_an_explicit_random_source() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-particle-trace-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("synthetic.ps3");
    let mut arguments = args(99, 1);
    arguments[1] = 0;
    let mut code = Vec::new();
    command(&mut code, 0x22a5, &arguments);
    command(&mut code, 0x22a0, &[99]);
    command(&mut code, 0x2003, &[]);
    let text = b"particle.pb3\0";
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48u32,
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
    fs::write(&path, &raw).unwrap();
    for seeded in [false, true] {
        let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
        cli.arg(&path)
            .args(["--profile", "cmvs-20171114", "--record-platform"])
            .arg(&root);
        if seeded {
            cli.args(["--rand-seed", "0"]);
        }
        let output = cli.output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(stdout.contains("# particle pools=0"), seeded, "{stdout}");
        assert_eq!(
            stdout.contains("# effect object release 0x22a0:"),
            seeded,
            "{stdout}"
        );
        let stop = if seeded { "0x2003" } else { "0x22a5" };
        assert!(
            stdout.contains(&format!("# STOP pending system={stop};")),
            "{stdout}"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), raw);
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn falling_override_preserves_pending_on_failure_and_shares_effect_slots_and_random() {
    let arguments = [
        0,
        0x80000000,
        10.0f32.to_bits(),
        8.0f32.to_bits(),
        12000.0f32.to_bits(),
        6.0f32.to_bits(),
        2,
        1,
    ];
    let mut code = vec![];
    command(&mut code, 0x22a5, &args(0, 1));
    command(&mut code, 0x22a2, &arguments);
    command(&mut code, 0x22a5, &args(0, 1));
    command(&mut code, 0x22a2, &arguments);
    command(&mut code, 0x22a0, &[0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"particle.pb3").unwrap();
    let mut scene = SceneHost::default();
    let mut source = Images::default();
    let mut random = CrtRandom::new(0);
    pending(&mut vm);
    run(&mut vm, &mut scene, &mut source, &mut random).unwrap();
    for index in 0..2 {
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let original_random = random.clone();
        let semantics = Semantics::Research(ResearchId::FallingBillboards);
        assert_eq!(
            dispatch::run(semantics, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        source.fail = true;
        let execute = |vm: &mut Machine<'_>,
                       scene: &mut SceneHost,
                       random: &mut CrtRandom,
                       source: &mut Images| {
            dispatch::run(
                semantics,
                vm,
                &mut Hosts {
                    images: Some(ImageHosts {
                        scene,
                        source,
                        change_images: false,
                    }),
                    random: Some(random),
                    ..Default::default()
                },
            )
        };
        assert!(execute(&mut vm, &mut scene, &mut random, &mut source).is_err());
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(scene.particle_effects()[&0].particles.len(), 1);
        assert!(scene.falling_effects().is_empty());
        assert_eq!(
            random.clone().next_value(),
            original_random.clone().next_value()
        );
        source.fail = false;
        assert_eq!(
            execute(&mut vm, &mut scene, &mut random, &mut source).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        assert!(scene.particle_effects().is_empty());
        assert_eq!(scene.falling_effects()[&0].particles.len(), 2);
        let mut expected = original_random;
        for _ in 0..8 {
            expected.next_value();
        }
        assert_eq!(random.next_value(), expected.next_value());
        pending(&mut vm);
        if index == 0 {
            run(&mut vm, &mut scene, &mut source, &mut random).unwrap();
            assert!(scene.falling_effects().is_empty());
        } else {
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    scene: Some(&mut scene),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(scene.falling_effects().is_empty());
        }
    }
    for (invalid, accepted) in [
        (
            {
                let mut a = arguments.to_vec();
                a[0] = 99;
                a[2] = f32::NAN.to_bits();
                a[6] = u32::MAX;
                a
            },
            true,
        ),
        (arguments[..7].to_vec(), false),
        (
            {
                let mut a = arguments.to_vec();
                a[2] = 0;
                a
            },
            false,
        ),
        (
            {
                let mut a = arguments.to_vec();
                a[6] = u32::MAX;
                a
            },
            false,
        ),
    ] {
        let mut code = vec![];
        command(&mut code, 0x22a2, &invalid);
        let script = support::bytecode::script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"particle.pb3").unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let reads = source.reads;
        let seed = random.clone().next_value();
        let result = dispatch::run(
            Semantics::Research(ResearchId::FallingBillboards),
            &mut vm,
            &mut Hosts {
                images: Some(ImageHosts {
                    scene: &mut scene,
                    source: &mut source,
                    change_images: false,
                }),
                random: Some(&mut random),
                ..Default::default()
            },
        );
        assert_eq!(result.is_ok(), accepted);
        if !accepted {
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
        }
        assert!(scene.falling_effects().is_empty());
        assert_eq!(source.reads, reads);
        assert_eq!(random.clone().next_value(), seed);
    }
}

fn run_collision(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    source: &mut Images,
    random: &mut CrtRandom,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    let command = vm.pending_system().unwrap().1;
    let mut hosts = Hosts {
        random: Some(random),
        ..Default::default()
    };
    if dispatch::uses_images(command) {
        hosts.images = Some(ImageHosts {
            scene,
            source,
            change_images: false,
        });
    } else {
        hosts.scene = Some(scene);
    }
    dispatch::run(
        Semantics::Research(ResearchId::CollisionParticles),
        vm,
        &mut hosts,
    )
}

#[test]
fn collision_mesh_commands_bound_vertices_and_particle_creation_retries_atomically() {
    let mut code = vec![];
    command(&mut code, 0x2020, &[20]);
    command(&mut code, 0x2330, &[20]);
    for index in 0..130 {
        command(
            &mut code,
            0x2332,
            &[
                20,
                (index as f32).to_bits(),
                2.0f32.to_bits(),
                3.0f32.to_bits(),
            ],
        );
    }
    command(&mut code, 0x2331, &[20]);
    command(&mut code, 0x2332, &[20, 0, 0, 0]);
    command(
        &mut code,
        0x22a6,
        &[0, 0x80000000, 0, 15.0f32.to_bits(), 5000.0f32.to_bits(), 2],
    );
    command(&mut code, 0x22a0, &[0]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"particle.pb3").unwrap();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    let mut scene = SceneHost::default();
    let mut source = Images::default();
    let mut random = CrtRandom::new(0);
    pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    for index in 0..133 {
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(
                Semantics::Research(ResearchId::CollisionParticles),
                &mut vm,
                &mut Hosts::default()
            )
            .unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(
            run_collision(&mut vm, &mut scene, &mut source, &mut random).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
        if index == 130 {
            assert_eq!(
                scene
                    .object(20)
                    .unwrap()
                    .collision_vertices
                    .as_ref()
                    .unwrap()
                    .len(),
                128
            );
        }
    }
    assert!(scene.object(20).unwrap().collision_vertices.is_none());
    pending(&mut vm);
    let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
    source.fail = true;
    assert!(run_collision(&mut vm, &mut scene, &mut source, &mut random).is_err());
    assert_eq!(
        (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
        before
    );
    assert!(scene.collision_effects().is_empty());
    assert_eq!(random.clone().next_value(), CrtRandom::new(0).next_value());
    source.fail = false;
    run_collision(&mut vm, &mut scene, &mut source, &mut random).unwrap();
    assert_eq!(scene.collision_effects()[&0].particles.len(), 2);
    assert!(scene.particle_effects().is_empty() && scene.falling_effects().is_empty());
    let mut expected = CrtRandom::new(0);
    for _ in 0..10 {
        expected.next_value();
    }
    assert_eq!(random.next_value(), expected.next_value());
    pending(&mut vm);
    dispatch::run(
        Semantics::Root,
        &mut vm,
        &mut Hosts {
            scene: Some(&mut scene),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(scene.collision_effects().is_empty());
}

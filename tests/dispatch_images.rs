use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts, ImageHosts},
    image::ImageSource,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
use std::error::Error;
mod support;
use support::bytecode::{command, pending, script};

const NAME: u32 = 0x80000000;

fn cases() -> Vec<(u16, Vec<u32>)> {
    vec![
        (0x2030, vec![3, NAME]),
        (0x2035, vec![3, NAME, 0, 0]),
        (0x2038, vec![3, 7, NAME]),
        (0x203b, vec![3, NAME, 2, 0x808080, 0]),
        (0x203f, vec![]),
        (0x222a, vec![NAME, 3, 20, 10, 1]),
        (0x222d, vec![]),
        (0x2234, vec![NAME, 3, 20, 10, 1]),
        (0x2237, vec![]),
        (0x22c4, vec![3, NAME, 20, 0]),
    ]
}

/// Synthetic 2x1 PB3 type 1, three solid color planes.
fn pixels() -> Vec<u8> {
    fn put(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut bytes = vec![0; 52];
    bytes[..4].copy_from_slice(b"PB3B");
    put(&mut bytes, 8, 1);
    put(&mut bytes, 24, 1);
    bytes[28] = 1;
    bytes[30] = 2;
    bytes[32] = 1;
    bytes[34] = 24;
    put(&mut bytes, 44, 52);
    put(&mut bytes, 48, 52 + 19 * 3);
    for _ in 0..3 {
        bytes.extend(15u32.to_le_bytes());
    }
    for _ in 0..3 {
        for value in [1u32, 2, 0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend([0xc0, 255, 255]);
    }
    for _ in 0..3 {
        bytes.extend(0u32.to_le_bytes());
    }
    let len = bytes.len() as u32;
    put(&mut bytes, 4, len);
    let seed = bytes[bytes.len() - 47..bytes.len() - 3].to_vec();
    let key = [bytes[bytes.len() - 3], bytes[bytes.len() - 2]];
    for i in 0..44 {
        bytes[8 + i] = bytes[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
    bytes
}

#[derive(Default)]
struct Images {
    reads: usize,
    arms: usize,
    fail: bool,
}
impl ImageSource for Images {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        assert_eq!(name, b"a.pb3");
        self.reads += 1;
        if self.fail {
            return Err(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read rejected").into(),
            );
        }
        Ok(Some(pixels()))
    }
    fn cache_next_read(&mut self) -> Result<(), Box<dyn Error>> {
        self.arms += 1;
        Ok(())
    }
}

fn run(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    source: &mut dyn ImageSource,
    change_images: bool,
) -> Result<Option<Flow>, dispatch::DispatchError> {
    dispatch::run(
        Semantics::Root,
        vm,
        &mut Hosts {
            images: Some(ImageHosts {
                scene,
                source,
                change_images,
            }),
            ..Default::default()
        },
    )
}

fn setup(scene: &mut SceneHost, images: &mut Images) {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![3]),
        (0x2022, vec![3, 7]),
        (0x2040, vec![3]),
        (0x2030, vec![3, NAME]),
    ] {
        command(&mut code, op, &args);
    }
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"a.pb3").unwrap();
    for _ in 0..4 {
        pending(&mut vm);
        if dispatch::uses_images(vm.pending_system().unwrap().1) {
            assert_eq!(run(&mut vm, scene, images, true).unwrap(), Some(Flow::Done));
        } else {
            assert!(scene.try_handle(&mut vm).unwrap());
        }
    }
}

#[test]
fn all_image_entries_preserve_missing_capabilities_and_complete_once() {
    let expected: std::collections::BTreeSet<_> =
        cases().iter().map(|(op, _)| op - 0x2000).collect();
    let actual = dispatch::ROOT
        .iter()
        .filter(|e| dispatch::uses_images(e.command))
        .map(|e| e.command)
        .collect();
    assert_eq!(expected, actual);
    assert_eq!(expected.len(), 10);
    for (opcode, args) in cases() {
        let mut scene = SceneHost::default();
        let mut images = Images::default();
        setup(&mut scene, &mut images);
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"a.pb3").unwrap();
        vm.state_mut().set_system_integer(0, 99).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let objects = scene.objects().clone();
        let reads = images.reads;
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        if opcode == 0x22c4 {
            assert_eq!(
                run(&mut vm, &mut scene, &mut images, false).unwrap(),
                Some(Flow::Unavailable)
            );
        }
        assert_eq!(
            dispatch::run(
                Semantics::Research(cmvs_rs::profile::ResearchId::FontCells),
                &mut vm,
                &mut Hosts {
                    images: Some(ImageHosts {
                        scene: &mut scene,
                        source: &mut images,
                        change_images: true
                    }),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(scene.objects(), &objects);
        assert_eq!(images.reads, reads);
        assert_eq!(
            run(&mut vm, &mut scene, &mut images, true).unwrap(),
            Some(Flow::Done),
            "{opcode:04x}"
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            (before.0 + 2, None, 0)
        );
        let reads = images.reads;
        assert_eq!(run(&mut vm, &mut scene, &mut images, true).unwrap(), None);
        assert_eq!(images.reads, reads);
        assert_eq!(images.arms, usize::from(opcode == 0x203f));
        assert_eq!(
            vm.state().system_integer(0),
            Some(if matches!(opcode, 0x203f | 0x222d | 0x2237) {
                99
            } else {
                1
            })
        );
    }
}

#[test]
fn read_errors_keep_typed_source_scene_and_pending_command_for_retry() {
    for (opcode, args) in cases()
        .into_iter()
        .filter(|(op, _)| !matches!(op, 0x203f | 0x222d | 0x2237))
    {
        let mut scene = SceneHost::default();
        let mut images = Images::default();
        setup(&mut scene, &mut images);
        images.fail = true;
        let objects = scene.objects().clone();
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        vm.state_mut().set_string(0, b"a.pb3").unwrap();
        vm.state_mut().set_system_integer(0, 99).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let error = run(&mut vm, &mut scene, &mut images, true).unwrap_err();
        assert_eq!((error.pc, error.command), (before.0, opcode - 0x2000));
        assert_eq!(
            error.to_string(),
            format!("PC=0x{:x} 0x{opcode:04x}: read rejected", before.0)
        );
        let cause = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(cause.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(99));
        assert_eq!(scene.objects(), &objects);
        assert!(scene.mask().is_none() && scene.second_mask().is_none());
        images.fail = false;
        assert_eq!(
            run(&mut vm, &mut scene, &mut images, true).unwrap(),
            Some(Flow::Done)
        );
    }
}

#[test]
fn short_arguments_fail_before_resource_access() {
    for entry in dispatch::ROOT.iter().filter(|e| {
        matches!(
            e.capability,
            Capability::SceneImages | Capability::MaskImages | Capability::ImageChange
        )
    }) {
        if matches!(entry.command, 0x3f | 0x22d | 0x237) {
            continue;
        }
        let mut code = vec![];
        command(&mut code, entry.command + 0x2000, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = vm.pending_system();
        let mut images = Images::default();
        let error = run(&mut vm, &mut SceneHost::default(), &mut images, true).unwrap_err();
        assert_eq!(
            error.as_decode().unwrap().reason,
            if entry.capability == Capability::MaskImages {
                "mask name"
            } else {
                "object slot"
            }
        );
        assert_eq!(vm.pending_system(), before);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!((images.reads, images.arms), (0, 0));
    }
}

#[test]
fn trace_keeps_image_capabilities_and_leaves_replacement_unavailable() {
    use std::{fs, process::Command};
    let root =
        std::env::temp_dir().join(format!("cmvs-cli-dispatch-images-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("raw")).unwrap();
    fs::write(root.join("raw/a.pb3"), pixels()).unwrap();
    let path = root.join("images.ps3");
    for (opcode, args) in cases() {
        for record in [false, true] {
            let mut code = vec![];
            if record {
                for (op, args) in [
                    (0x2011, vec![0, 0]),
                    (0x2020, vec![3]),
                    (0x2022, vec![3, 7]),
                    (0x2040, vec![3]),
                    (0x2030, vec![3, 5]),
                ] {
                    command(&mut code, op, &args);
                }
            }
            let args: Vec<_> = args
                .iter()
                .map(|value| if *value == NAME { 5 } else { *value })
                .collect();
            command(&mut code, opcode, &args);
            command(&mut code, 0x2003, &[]);
            let text = b"raw\\\0a.pb3\0";
            let mut raw = b"PS2A".to_vec();
            for word in [
                48u32,
                0,
                0,
                0,
                code.len() as u32,
                0,
                text.len() as u32,
                0,
                0,
                (code.len() + text.len()) as u32,
                0,
            ] {
                raw.extend(word.to_le_bytes());
            }
            raw.extend(
                code.iter()
                    .chain(text)
                    .map(|b| b.rotate_left(1).wrapping_add(124)),
            );
            fs::write(&path, &raw).unwrap();
            let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
            cli.arg(&path)
                .args(["--profile", "cmvs-20160201", "--bootstrap"]);
            if record {
                cli.arg("--record-platform").arg(&root);
            }
            let output = cli.output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stop = if record && opcode != 0x22c4 {
                0x2003
            } else {
                opcode
            };
            assert!(
                stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                "{stdout}"
            );
            assert_eq!(fs::read(&path).unwrap(), raw);
        }
    }
    fs::remove_file(path).unwrap();
    fs::remove_file(root.join("raw/a.pb3")).unwrap();
    fs::remove_dir(root.join("raw")).unwrap();
    fs::remove_dir(root).unwrap();
}

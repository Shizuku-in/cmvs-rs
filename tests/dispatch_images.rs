use cmvs_rs::{
    dispatch::{self, Flow, Hosts, ImageHosts},
    image::ImageSource,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
use std::error::Error;
mod support;
use support::bytecode::{command, pending, script};

const NAME: u32 = 0x80000000;

fn load_commands() -> Vec<(u16, Vec<u32>)> {
    vec![
        (0x2030, vec![3, NAME]),
        (0x2035, vec![3, NAME, 0, 0]),
        (0x2038, vec![3, 7, NAME]),
        (0x203b, vec![3, NAME, 2, 0x808080, 0]),
        (0x222a, vec![NAME, 3, 20, 10, 1]),
        (0x2234, vec![NAME, 3, 20, 10, 1]),
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
fn read_errors_keep_typed_source_scene_and_pending_command_for_retry() {
    for (opcode, args) in load_commands() {
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

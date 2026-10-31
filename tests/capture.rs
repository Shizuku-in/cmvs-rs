use cmvs_rs::{
    capture::ScreenCapture,
    machine::{Machine, Step},
    ps3::Script,
    raster::RgbaImage,
    scene::SceneHost,
    scene_render,
};

fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        0,
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
/// Pushes `args` so the first one ends up at SP-4, then issues `op`.
fn call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
/// Runs to the next system command and offers it to the scene, then the capture.
fn next(
    vm: &mut Machine<'_>,
    scene: &mut SceneHost,
    capture: &mut ScreenCapture,
) -> Result<bool, cmvs_rs::bytecode::DecodeError> {
    while !matches!(vm.step()?, Step::System { .. }) {}
    Ok(scene.try_handle(vm)? || capture.try_handle(vm, scene)?)
}
/// A 2x1 frame whose alpha bytes differ from 255.
fn frame() -> RgbaImage {
    RgbaImage::new(2, 1, vec![10, 20, 30, 0, 40, 50, 60, 128]).unwrap()
}

#[test]
fn immediate_read_uses_the_device_frame_and_preserves_held_capture_and_object_state() {
    use cmvs_rs::{platform::RecordingBackend, scene::ObjectImage};
    let mut code = vec![];
    call(&mut code, 0x2020, &[3]);
    call(&mut code, 0x2040, &[3]);
    call(&mut code, 0x203c, &[3, u32::MAX]);
    call(&mut code, 0x2304, &[]);
    call(&mut code, 0x203e, &[3, 0x8000_0000]);
    call(&mut code, 0x203e, &[3, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
    capture.take(frame()).unwrap();
    for _ in 0..3 {
        assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    }
    capture
        .take(RgbaImage::new(1, 1, vec![99, 88, 77, 255]).unwrap())
        .unwrap();
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    let held = capture.image().unwrap().clone();
    let mut backend = RecordingBackend::default();
    for (index, readback) in [frame(), RgbaImage::new(1, 1, vec![9, 8, 7, 128]).unwrap()]
        .into_iter()
        .enumerate()
    {
        let before = scene.objects().clone();
        let mut expected = scene.object(3).unwrap().clone();
        let mut pixels = readback.pixels().to_vec();
        for p in pixels.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        expected.image = Some(ObjectImage {
            filter: None,
            name: vec![],
            alpha: false,
            surface: false,
            pixels: RgbaImage::new(readback.width(), readback.height(), pixels).unwrap(),
        });
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let pc = vm.pc();
        vm.state_mut().set_system_integer(0, 12).unwrap();
        vm.state_mut().set_system_integer(1, 34).unwrap();
        if index == 0 {
            assert!(
                capture
                    .try_immediate(&mut vm, &mut scene, &mut backend)
                    .is_err()
            );
            assert_eq!(scene.objects(), &before);
            assert_eq!(vm.pc(), pc);
            assert_eq!(vm.stack().len_bytes(), 8);
            assert_eq!(vm.state().system_integer(0), Some(12));
        }
        backend.present_frame(readback.clone());
        assert!(
            capture
                .try_immediate(&mut vm, &mut scene, &mut backend)
                .unwrap()
        );
        assert_eq!(scene.object(3), Some(&expected), "read {index}");
        assert_eq!(capture.image(), Some(&held));
        assert!(capture.requested());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(1));
        assert_eq!(vm.state().system_integer(1), Some(34));
        // The device read is non-consuming and does not alter the published frame.
        use cmvs_rs::platform::PlatformBackend;
        assert_eq!(backend.read_device_frame().unwrap(), readback);
    }
}

#[test]
fn immediate_invalid_targets_and_short_stack_do_not_read_or_consume() {
    use cmvs_rs::platform::{PlatformBackend, PlatformRequest};
    struct NoRead;
    impl PlatformBackend for NoRead {
        fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn std::error::Error>> {
            panic!("invalid target must not read device");
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn std::error::Error>> {
            panic!("not a platform request");
        }
    }
    for args in [
        vec![3, 0],
        vec![4, u32::MAX],
        vec![256, u32::MAX],
        vec![u32::MAX, u32::MAX],
        vec![3],
    ] {
        let mut code = vec![];
        call(&mut code, 0x2020, &[3]);
        call(&mut code, 0x203e, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
        assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
        capture.take(frame()).unwrap();
        let before = scene.objects().clone();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        vm.state_mut().set_system_integer(0, 12).unwrap();
        assert!(
            capture
                .try_immediate(&mut vm, &mut scene, &mut NoRead)
                .is_err()
        );
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x3e)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(vm.state().system_integer(0), Some(12));
        assert!(capture.image().is_some());
    }
}

#[test]
fn capture_is_requested_taken_with_opaque_alpha_queried_given_and_reset() {
    let mut code = vec![];
    call(&mut code, 0x2020, &[3]);
    call(&mut code, 0x2306, &[]);
    call(&mut code, 0x2304, &[]);
    call(&mut code, 0x2306, &[]);
    call(&mut code, 0x203c, &[3, u32::MAX]);
    call(&mut code, 0x2306, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
    for i in 0..4 {
        vm.state_mut().set_system_integer(i, 100 + i).unwrap();
    }
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert!(capture.requested() && capture.image().is_none());
    capture.take(frame()).unwrap();
    assert!(!capture.requested());
    let opaque = [10, 20, 30, 255, 40, 50, 60, 255];
    assert_eq!(capture.image().unwrap().pixels(), opaque);
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    let image = scene.object(3).unwrap().image.as_ref().unwrap();
    assert_eq!((image.name.as_slice(), image.alpha), (&[][..], false));
    assert_eq!(image.pixels.pixels(), opaque);
    assert!(capture.image().is_none() && !capture.requested());
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(0));
    // Only result 0 changes; the queries and requests pop nothing.
    let results: Vec<_> = (1..4).map(|i| vm.state().system_integer(i)).collect();
    assert_eq!(results, [Some(101), Some(102), Some(103)]);
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
}

#[test]
fn release_clears_a_request_and_a_held_capture() {
    let mut code = vec![];
    call(&mut code, 0x2304, &[]);
    call(&mut code, 0x2305, &[]);
    call(&mut code, 0x2305, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    capture.take(frame()).unwrap();
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert!(capture.image().is_none() && !capture.requested());
    assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
}

#[test]
fn giving_a_capture_needs_an_object_itself_and_a_capture() {
    for (args, captured) in [
        (vec![3, 0], true),          // A child is never a target.
        (vec![4, u32::MAX], true),   // Missing object.
        (vec![256, u32::MAX], true), // Slot outside the table.
        (vec![3, u32::MAX], false),  // Nothing captured.
        (vec![3], true),             // Missing argument.
    ] {
        let mut code = vec![];
        call(&mut code, 0x2020, &[3]);
        call(&mut code, 0x2022, &[3, 0]);
        call(&mut code, 0x203c, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
        for _ in 0..2 {
            assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
        }
        if captured {
            capture.take(frame()).unwrap();
        }
        vm.state_mut().set_system_integer(0, 7).unwrap();
        let before = scene.objects().clone();
        assert!(next(&mut vm, &mut scene, &mut capture).is_err(), "{args:?}");
        assert_eq!(scene.objects(), &before);
        assert_eq!(capture.image().is_some(), captured);
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x3c)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
        assert_eq!(vm.state().system_integer(0), Some(7));
    }
}

#[test]
fn logical_size_targets_objects_and_children_and_pops_sixteen() {
    let mut code = vec![];
    call(&mut code, 0x2020, &[5]);
    call(&mut code, 0x2022, &[5, 767]);
    call(&mut code, 0x202a, &[5, u32::MAX, 1280, 720]);
    call(&mut code, 0x202a, &[5, 767, 640, u32::MAX]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
    vm.state_mut().set_system_integer(0, 9).unwrap();
    for _ in 0..4 {
        assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
    }
    let object = scene.object(5).unwrap();
    assert_eq!(object.logical_size, [1280, 720]);
    assert_eq!(object.children[&767].logical_size, [640, u32::MAX]);
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
    assert_eq!(vm.state().system_integer(0), Some(9));
    for args in [
        vec![6, u32::MAX, 1, 1],   // Missing object.
        vec![256, u32::MAX, 1, 1], // Slot outside the table.
        vec![5, 0, 1, 1],          // Missing child.
        vec![5, 768, 1, 1],        // Child outside the table.
        vec![5, u32::MAX, 1],      // Missing argument.
    ] {
        let mut code = vec![];
        call(&mut code, 0x2020, &[5]);
        call(&mut code, 0x202a, &args);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut scene = SceneHost::default();
        assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
        let before = scene.objects().clone();
        assert!(next(&mut vm, &mut scene, &mut capture).is_err(), "{args:?}");
        assert_eq!(scene.objects(), &before);
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x2a)));
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
}

#[test]
fn only_unit_logical_screen_scaling_is_rendered() {
    for (width, child, drawn) in [
        (0, false, true),
        (1280, false, true),
        (640, false, false),
        (1280, true, true),
        (640, true, false),
    ] {
        let mut code = vec![];
        call(&mut code, 0x2020, &[1]);
        call(&mut code, 0x2040, &[1]);
        call(&mut code, 0x2043, &[1, u32::MAX, 1, 1, 0x00ff00]);
        if child {
            call(&mut code, 0x2022, &[1, 0]);
            call(&mut code, 0x2050, &[1, 0]);
            call(&mut code, 0x2043, &[1, 0, 1, 1, 0x00ff00]);
        }
        let target = if child { 0 } else { u32::MAX };
        call(&mut code, 0x202a, &[1, target, width, 720]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let (mut scene, mut capture) = (SceneHost::default(), ScreenCapture::default());
        while vm.pc() < code.len() {
            assert!(next(&mut vm, &mut scene, &mut capture).unwrap());
        }
        let frame = scene_render::render(&scene, 2, 1);
        assert_eq!(frame.is_ok(), drawn, "{width} child={child}");
        if let Ok(frame) = frame {
            assert_eq!(&frame.pixels()[..4], &[0, 255, 0, 255]);
        }
    }
}

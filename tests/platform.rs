use cmvs_rs::{
    cursor::CursorBitmap,
    machine::{Machine, Step},
    platform::{
        DirectoryAssets, PlatformAssets, PlatformBackend, PlatformHost, PlatformRequest,
        RecordingBackend, ThumbnailConfig,
    },
    ps3::Script,
};
use std::{collections::BTreeMap, error::Error, fs, process::Command};
#[path = "support/bytecode.rs"]
mod bytecode;

#[test]
fn window_size_reset_yields_only_when_it_changes_and_preserves_results() {
    use cmvs_rs::platform::PlatformEvent;
    let mut code = vec![];
    bytecode::command(&mut code, 0x216e, &[]);
    bytecode::command(&mut code, 0x216e, &[]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    for expected in [PlatformEvent::Yield, PlatformEvent::Configured] {
        bytecode::pending(&mut vm);
        vm.state_mut().set_system_integer(0, 77).unwrap();
        assert_eq!(
            host.try_handle(&mut vm, &mut backend, &mut Assets::default(), &[])
                .unwrap(),
            Some(expected)
        );
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(backend.window_size().unwrap(), [1280, 720]);
    }
    assert_eq!(
        backend.requests(),
        &[PlatformRequest::WindowSize([1280, 720])]
    );
}

#[test]
fn fixed_window_reset_is_a_noop_and_fullscreen_future_size_stays_an_explicit_barrier() {
    struct Fixed;
    impl PlatformBackend for Fixed {
        fn resizable(&self) -> Result<bool, Box<dyn Error>> {
            Ok(false)
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            panic!("fixed window must not resize")
        }
    }
    let mut code = vec![];
    bytecode::command(&mut code, 0x216e, &[]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    bytecode::pending(&mut vm);
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    backend.apply(&PlatformRequest::Fullscreen(true)).unwrap();
    let before = vm.pending_system();
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut Assets::default(), &[])
            .is_err()
    );
    assert_eq!(vm.pending_system(), before);
    assert_eq!(
        host.try_handle(&mut vm, &mut Fixed, &mut Assets::default(), &[])
            .unwrap(),
        Some(cmvs_rs::platform::PlatformEvent::Configured)
    );
}

#[test]
fn aspect_mode_normalizes_invalid_words_preserves_results_and_skips_equal_requests() {
    let mut code = vec![];
    for raw in [1, 0, 2, 3, 3, u32::MAX, 4] {
        bytecode::command(&mut code, 0x22fd, &[raw]);
        bytecode::command(&mut code, 0x22fc, &[]);
    }
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    let mut assets = Assets::default();
    for value in [1, 0, 2, 3, 3, 1, 1] {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        bytecode::pending(&mut vm);
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        bytecode::pending(&mut vm);
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(value));
    }
    assert_eq!(
        backend.requests(),
        &[0, 2, 3, 1, 1].map(PlatformRequest::AspectMode)
    );
}

#[test]
fn render_mode_clamps_words_falls_back_without_linear_support_and_skips_equal_requests() {
    let mut code = vec![];
    for raw in [1, 1, 0, 3, 2, u32::MAX, 4, 1] {
        bytecode::command(&mut code, 0x216d, &[raw]);
        bytecode::command(&mut code, 0x216c, &[]);
    }
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    let mut assets = Assets::default();
    // Shader modes 2/3 need PS2/PS3; the recording backend reports major 0.
    for value in [1, 1, 0, 0, 0, 0, 0, 1] {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        bytecode::pending(&mut vm);
        assert_eq!(
            host.try_handle(&mut vm, &mut backend, &mut assets, &[])
                .unwrap(),
            Some(cmvs_rs::platform::PlatformEvent::Configured)
        );
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        bytecode::pending(&mut vm);
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(value));
    }
    assert_eq!(
        backend.requests(),
        &[1, 0, 1].map(PlatformRequest::RenderMode)
    );
    assert_eq!(assets.calls, 0);

    struct PointOnly(u32);
    impl PlatformBackend for PointOnly {
        fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
            Ok(false)
        }
        fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
            Ok(0)
        }
        fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
            Ok(self.0)
        }
        fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            assert_eq!(request, &PlatformRequest::RenderMode(0));
            self.0 = 0;
            Ok(())
        }
    }
    let mut code = vec![];
    bytecode::command(&mut code, 0x216d, &[1]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    bytecode::pending(&mut vm);
    let mut device = PointOnly(1);
    host.try_handle(&mut vm, &mut device, &mut assets, &[])
        .unwrap();
    assert_eq!(device.0, 0);
}

#[test]
fn shader_render_modes_on_capable_devices_and_backend_failures_stay_barriers() {
    struct Device(u8, bool);
    impl PlatformBackend for Device {
        fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
            Ok(true)
        }
        fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
            Ok(self.0)
        }
        fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
            Ok(0)
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            if self.1 {
                Ok(())
            } else {
                Err("device reset failed".into())
            }
        }
    }
    for (raw, device) in [
        (2, Device(2, true)),
        (3, Device(3, true)),
        (1, Device(0, false)),
    ] {
        let mut code = vec![];
        bytecode::command(&mut code, 0x216d, &[raw]);
        let script = bytecode::script(&code);
        let mut vm = Machine::new(&script).unwrap();
        bytecode::pending(&mut vm);
        let before = vm.pending_system();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        let mut device = device;
        assert!(
            PlatformHost::default()
                .try_handle(&mut vm, &mut device, &mut Assets::default(), &[])
                .is_err()
        );
        assert_eq!(vm.pending_system(), before);
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
    // PS2 alone does not satisfy the PS3 mode, which therefore falls back to point.
    let mut code = vec![];
    bytecode::command(&mut code, 0x216d, &[3]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    bytecode::pending(&mut vm);
    PlatformHost::default()
        .try_handle(&mut vm, &mut Device(2, false), &mut Assets::default(), &[])
        .unwrap();
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn fullscreen_changes_yield_preserve_results_and_query_the_new_backend_mode() {
    use cmvs_rs::platform::PlatformEvent;
    let mut code = vec![];
    for raw in [1, 0, u32::MAX, u32::MAX] {
        bytecode::command(&mut code, 0x2164, &[raw]);
        bytecode::command(&mut code, 0x2163, &[]);
    }
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    let mut assets = Assets::default();
    for value in [true, false, true, true] {
        vm.state_mut().set_system_integer(0, 77).unwrap();
        bytecode::pending(&mut vm);
        assert_eq!(
            host.try_handle(&mut vm, &mut backend, &mut assets, &[])
                .unwrap(),
            Some(PlatformEvent::Yield)
        );
        assert_eq!(vm.state().system_integer(0), Some(77));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(
            backend.requests().last(),
            Some(&PlatformRequest::Fullscreen(value))
        );
        bytecode::pending(&mut vm);
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(u32::from(value)));
    }
    assert_eq!(assets.calls, 0);
}

#[test]
fn failed_display_changes_retain_pending_command_stack_and_results() {
    struct Reject;
    impl PlatformBackend for Reject {
        fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
            Ok(1)
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            Err("synthetic display failure".into())
        }
    }
    for op in [0x2164, 0x22fd] {
        let mut code = vec![];
        bytecode::command(&mut code, op, &[2]);
        let script = bytecode::script(&code);
        let mut vm = Machine::new(&script).unwrap();
        bytecode::pending(&mut vm);
        let before = vm.pending_system();
        vm.state_mut().set_system_integer(0, 77).unwrap();
        assert!(
            PlatformHost::default()
                .try_handle(&mut vm, &mut Reject, &mut Assets::default(), &[])
                .is_err()
        );
        assert_eq!(vm.pending_system(), before);
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.state().system_integer(0), Some(77));
    }
}

#[test]
fn fullscreen_query_uses_backend_state_and_needs_no_arguments_or_assets() {
    struct Display(bool);
    impl PlatformBackend for Display {
        fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
            Ok(self.0)
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            panic!("query must not apply requests")
        }
    }
    for fullscreen in [false, true] {
        let mut code = Vec::new();
        bytecode::command(&mut code, 0x2163, &[]);
        let script = bytecode::script(&code);
        let mut vm = Machine::new(&script).unwrap();
        bytecode::pending(&mut vm);
        let mut host = PlatformHost::default();
        let mut assets = Assets::default();
        host.try_handle(&mut vm, &mut Display(fullscreen), &mut assets, &[])
            .unwrap();
        assert_eq!(vm.state().system_integer(0), Some(u32::from(fullscreen)));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(assets.calls, 0);
    }
}

#[test]
fn graphics_and_resizing_queries_report_explicit_backend_capabilities() {
    struct Device(u8, bool);
    impl PlatformBackend for Device {
        fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error>> {
            Ok(true)
        }
        fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
            Ok(2)
        }
        fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
            Ok(self.0)
        }
        fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
            Ok(2)
        }
        fn resizable(&self) -> Result<bool, Box<dyn Error>> {
            Ok(self.1)
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
            panic!("query must not apply requests")
        }
    }
    for (major, flags) in [(0, 3), (1, 3), (2, 7), (3, 15), (255, 15)] {
        let mut code = Vec::new();
        for op in [0x216b, 0x216c, 0x216f, 0x22c8, 0x22fc] {
            bytecode::command(&mut code, op, &[]);
        }
        let script = bytecode::script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut host = PlatformHost::default();
        let mut assets = Assets::default();
        for expected in [flags, 2, 1, 1, 2] {
            bytecode::pending(&mut vm);
            host.try_handle(&mut vm, &mut Device(major, true), &mut assets, &[])
                .unwrap();
            assert_eq!(vm.state().system_integer(0), Some(expected));
            assert_eq!(vm.stack().len_bytes(), 0);
        }
        assert_eq!(assets.calls, 0);
    }
}

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn cur() -> Vec<u8> {
    let mut b = vec![0; 94];
    b[2] = 2;
    b[4] = 1;
    b[6] = 2;
    b[7] = 2;
    b[8] = 4;
    b[10] = 1;
    put(&mut b, 14, 72);
    put(&mut b, 18, 22);
    put(&mut b, 22, 40);
    put(&mut b, 26, 2);
    put(&mut b, 30, 4);
    b[34] = 1;
    b[36] = 8;
    put(&mut b, 54, 4);
    b[66..70].copy_from_slice(&[30, 20, 10, 0]);
    b[70..74].copy_from_slice(&[255, 255, 255, 0]);
    b[78..86].copy_from_slice(&[2, 3, 0, 0, 0, 1, 0, 0]);
    b[86] = 0xc0;
    b
}
fn ico() -> Vec<u8> {
    let mut b = vec![0; 23];
    b[2] = 1;
    b[4] = 1;
    b[6] = 1;
    b[7] = 1;
    put(&mut b, 14, 1);
    put(&mut b, 18, 22);
    b
}

#[test]
fn cursor_bottom_up_palette_and_mask_preserve_all_compositing_cases() {
    let c = CursorBitmap::decode(&cur()).unwrap();
    assert_eq!((c.width(), c.height(), c.hotspot()), (2, 2, (1, 0)));
    assert_eq!(
        c.xor_rgb(),
        [[0, 0, 0], [10, 20, 30], [255, 255, 255], [0, 0, 0]]
    );
    assert_eq!(c.and_mask(), [false, false, true, true]);
    assert_eq!(
        c.composite(&[[12, 34, 56]; 4]).unwrap(),
        [[0, 0, 0], [10, 20, 30], [243, 221, 199], [12, 34, 56]]
    );
    assert!(c.composite(&[]).is_err());
}

#[test]
fn cursor_truncation_invalid_ranges_and_unsupported_variants_fail() {
    let raw = cur();
    for end in 0..raw.len() {
        assert!(CursorBitmap::decode(&raw[..end]).is_err());
    }
    for (at, v) in [
        (2, 1),
        (4, 2),
        (10, 2),
        (14, u32::MAX),
        (18, u32::MAX),
        (22, 108),
        (26, 3),
        (30, u32::MAX),
        (36, 32),
        (38, 1),
        (54, 257),
    ] {
        let mut b = raw.clone();
        put(&mut b, at, v);
        assert!(CursorBitmap::decode(&b).is_err(), "at={at}");
    }
    let mut b = raw.clone();
    b[78] = 4;
    assert!(CursorBitmap::decode(&b).is_err());
    let mut b = raw;
    b.push(0);
    assert!(CursorBitmap::decode(&b).is_err());
}

#[derive(Default)]
struct Assets {
    files: BTreeMap<Vec<u8>, Vec<u8>>,
    calls: usize,
    fail: bool,
}
impl PlatformAssets for Assets {
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        self.calls += 1;
        if self.fail {
            return Err("synthetic read failure".into());
        }
        self.files
            .get(name)
            .cloned()
            .ok_or_else(|| "synthetic missing asset".into())
    }
}
fn script(commands: &[(u16, u32)], text: &[u8]) -> Script {
    let mut code = vec![];
    for (op, arg) in commands {
        code.extend(0x442u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
        code.extend(op.to_le_bytes());
    }
    code.extend(0x201eu16.to_le_bytes());
    let mut raw = b"PS2A".to_vec();
    for v in [
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
        raw.extend(v.to_le_bytes());
    }
    code.extend(text);
    raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn title_and_icon_requests_preserve_byte_encoding_and_registers() {
    let script = script(&[(0x2160, 0), (0x208f, 3)], b"\x81x\0icon.ico\0");
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().result = Some(99);
    vm.state_mut().set_system_integer(0, 4).unwrap();
    let mut assets = Assets::default();
    assets.files.insert(b"icon.ico".to_vec(), ico());
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap()
            .is_none()
    );
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap();
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    assert_eq!(host.title(), Some(b"\x81x".as_slice()));
    assert_eq!(host.icon(), Some(b"icon.ico".as_slice()));
    assert_eq!(backend.requests().len(), 2);
    assert_eq!(assets.calls, 1);
    assert_eq!(vm.state().result, Some(99));
    assert_eq!(vm.state().system_integer(0), Some(4));
    pending(&mut vm);
    let pc = vm.pc();
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(vm.pc(), pc);
}

struct RejectBackend;
impl PlatformBackend for RejectBackend {
    fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        Err("synthetic backend unavailable".into())
    }
}
#[test]
fn backend_failure_preserves_vm_and_host_for_retry() {
    let script = script(&[(0x2160, 0)], b"title\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut host = PlatformHost::default();
    let mut assets = Assets::default();
    let before = vm.pending_system();
    assert!(
        host.try_handle(&mut vm, &mut RejectBackend, &mut assets, &[])
            .is_err()
    );
    assert_eq!(vm.pending_system(), before);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(host.title(), None);
    let mut backend = RecordingBackend::default();
    host.try_handle(&mut vm, &mut backend, &mut assets, &[])
        .unwrap();
    assert_eq!(host.title(), Some(b"title".as_slice()));
}

#[test]
fn scene_title_composes_window_text_and_keeps_base_title() {
    let mut assets = Assets::default();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    let orphan = script(&[(0x2161, 0)], b"scene\0");
    let mut vm = Machine::new(&orphan).unwrap();
    pending(&mut vm);
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .is_err()
    );
    assert_eq!((vm.stack().len_bytes(), backend.requests().len()), (4, 0));

    let mut text = b"Game\0- main -\0".to_vec();
    let long = text.len() as u32;
    text.extend([b'x'; 122]);
    text.push(0);
    let script = script(&[(0x2160, 0), (0x2161, 5), (0x2161, long)], &text);
    let mut vm = Machine::new(&script).unwrap();
    assert_eq!(host.scene_title(), b"");
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend, &mut assets, &[])
        .unwrap();
    pending(&mut vm);
    assert!(
        host.try_handle(&mut vm, &mut RejectBackend, &mut assets, &[])
            .is_err()
    );
    assert_eq!(host.scene_title(), b"");
    host.try_handle(&mut vm, &mut backend, &mut assets, &[])
        .unwrap();
    assert_eq!(host.title(), Some(b"Game".as_slice()));
    assert_eq!(host.scene_title(), b"- main -");
    assert_eq!(
        backend.requests().last(),
        Some(&PlatformRequest::Title(b"Game\x81\x40- main -".to_vec()))
    );
    pending(&mut vm);
    let before = vm.pending_system();
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .is_err()
    );
    assert_eq!(vm.pending_system(), before);
    assert_eq!(host.scene_title(), b"- main -");
    assert_eq!(backend.requests().len(), 2);
}

#[test]
fn cursor_selection_reads_once_default_restores_and_invalid_id_stops() {
    let script = script(&[(0x2099, 1), (0x2099, 1), (0x2099, 0), (0x2099, 2)], b"");
    let mut vm = Machine::new(&script).unwrap();
    let mut assets = Assets::default();
    assets.files.insert(b"mouse.cur".to_vec(), cur());
    let mut backend = RecordingBackend::default();
    let mut host = PlatformHost::default();
    let names = [b"mouse.cur".to_vec()];
    for expected in [1, 1, 0] {
        pending(&mut vm);
        host.try_handle(&mut vm, &mut backend, &mut assets, &names)
            .unwrap();
        assert_eq!(host.cursor(), expected);
    }
    assert_eq!(assets.calls, 1);
    assert_eq!(backend.requests().len(), 2);
    assert!(matches!(
        &backend.requests()[1],
        PlatformRequest::Cursor {
            id: 0,
            bitmap: None
        }
    ));
    pending(&mut vm);
    let before = vm.pending_system();
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &names)
            .is_err()
    );
    assert_eq!(vm.pending_system(), before);
    assert_eq!(host.cursor(), 0);
}

#[test]
fn cursor_io_decode_and_backend_errors_do_not_commit_selection() {
    let script = script(&[(0x2099, 1)], b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut assets = Assets::default();
    let mut backend = RecordingBackend::default();
    let mut host = PlatformHost::default();
    let names = [b"mouse.cur".to_vec()];
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &names)
            .is_err()
    );
    assets.files.insert(b"mouse.cur".to_vec(), vec![0; 22]);
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &names)
            .is_err()
    );
    assets.files.insert(b"mouse.cur".to_vec(), cur());
    assert!(
        host.try_handle(&mut vm, &mut RejectBackend, &mut assets, &names)
            .is_err()
    );
    assert_eq!(host.cursor(), 0);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert!(backend.requests().is_empty());
    host.try_handle(&mut vm, &mut backend, &mut assets, &names)
        .unwrap();
    assert_eq!(host.cursor(), 1);
}

#[test]
fn title_limit_and_icon_directory_errors_do_not_invoke_backend() {
    for len in [255, 256] {
        let mut text = vec![b'x'; len];
        text.push(0);
        let script = script(&[(0x2160, 0)], &text);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut host = PlatformHost::default();
        let mut assets = Assets::default();
        let mut backend = RecordingBackend::default();
        let result = host.try_handle(&mut vm, &mut backend, &mut assets, &[]);
        assert_eq!(result.is_ok(), len == 255);
        assert_eq!(backend.requests().len(), usize::from(len == 255));
    }
    for corrupt in [vec![0; 6], {
        let mut b = ico();
        put(&mut b, 18, u32::MAX);
        b
    }] {
        let script = script(&[(0x208f, 0)], b"icon.ico\0");
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut assets = Assets::default();
        assets.files.insert(b"icon.ico".to_vec(), corrupt);
        let mut backend = RecordingBackend::default();
        assert!(
            PlatformHost::default()
                .try_handle(&mut vm, &mut backend, &mut assets, &[])
                .is_err()
        );
        assert!(backend.requests().is_empty());
        assert_eq!(vm.stack().len_bytes(), 4);
    }
}

#[test]
fn thumbnail_argument_order_is_retained_and_gesture_commands_are_not_platform() {
    let short = script(&[(0x22b0, 192), (0x2350, 0)], b"");
    let mut vm = Machine::new(&short).unwrap();
    let mut host = PlatformHost::default();
    let mut backend = RecordingBackend::default();
    let mut assets = Assets::default();
    // 0x22b0 lacks the second argument: must fail without replacing configuration.
    pending(&mut vm);
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .is_err()
    );
    assert_eq!(host.thumbnail(), None);
    let mut code = vec![];
    for value in [108u32, 192] {
        code.extend(0x442u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(0x22b0u16.to_le_bytes());
    let mut raw = b"PS2A".to_vec();
    for v in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        0,
        0,
        0,
        code.len() as u32,
        0,
    ] {
        raw.extend(v.to_le_bytes());
    }
    raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    let image = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&image).unwrap();
    pending(&mut vm);
    host.try_handle(&mut vm, &mut backend, &mut assets, &[])
        .unwrap();
    assert_eq!(
        host.thumbnail(),
        Some(ThumbnailConfig {
            width: 192,
            height: 108,
            enabled: true
        })
    );
    assert_eq!(vm.stack().len_bytes(), 0);
    assert!(backend.requests().is_empty());
    let gesture = script(&[(0x2350, 1)], b"");
    let mut vm = Machine::new(&gesture).unwrap();
    pending(&mut vm);
    assert!(
        host.try_handle(&mut vm, &mut backend, &mut assets, &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(vm.pending_system(), Some((8, 0x350)));
}

#[test]
fn recording_backend_has_a_bounded_history() {
    let mut backend = RecordingBackend::default();
    for _ in 0..4096 {
        backend.apply(&PlatformRequest::Title(vec![])).unwrap();
    }
    assert!(backend.apply(&PlatformRequest::Title(vec![])).is_err());
    assert_eq!(backend.requests().len(), 4096);
    let mut backend = RecordingBackend::default();
    let request = PlatformRequest::Icon {
        name: vec![],
        bytes: std::sync::Arc::new(vec![0; 4 * 1024 * 1024]),
    };
    for _ in 0..16 {
        backend.apply(&request).unwrap();
    }
    assert!(backend.apply(&PlatformRequest::Title(vec![1])).is_err());
    assert_eq!(backend.requests().len(), 16);
}

#[test]
fn asset_paths_and_cursor_cli_are_bounded_and_read_only() {
    let root = std::env::temp_dir().join(format!("cmvs-platform-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("icons")).unwrap();
    let path = root.join("icons/mouse.cur");
    let raw = cur();
    fs::write(&path, &raw).unwrap();
    let mut assets = DirectoryAssets::new(&root).unwrap();
    assert_eq!(assets.read(b"icons\\mouse.cur").unwrap(), raw);
    for name in [
        b"../escape".as_slice(),
        b"icons/../mouse.cur",
        b"/icons/mouse.cur",
        b"C:\\mouse.cur",
        b"icons//mouse.cur",
        b"\xff",
    ] {
        assert!(assets.read(name).is_err());
    }
    let output = root.join("planes.bin");
    let exe = env!("CARGO_BIN_EXE_cmvs-cursor");
    let run = |target: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--planes")
            .arg(target)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(fs::read(&output).unwrap().len(), 16);
    assert!(!run(&output).status.success());
    fs::remove_file(output).unwrap();
    fs::remove_file(path).unwrap();
    fs::remove_dir(root.join("icons")).unwrap();
    fs::remove_dir(root).unwrap();
}

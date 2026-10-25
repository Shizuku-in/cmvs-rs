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

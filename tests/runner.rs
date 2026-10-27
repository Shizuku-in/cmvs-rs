use cmvs_rs::{
    platform::{PlatformBackend, PlatformRequest, RecordingBackend},
    ps3::Script,
    runner::{SnapshotPolicy, Startup, StartupEvent, load_start},
    snapshot::SaveImage,
};
use std::{error::Error, fs, path::PathBuf};

struct Root(PathBuf);
impl Root {
    fn new(name: &str, code: &[u8], text: &[u8]) -> Self {
        let root = std::env::temp_dir().join(format!("cmvs-startup-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::create_dir(root.join("data/pack")).unwrap();
        fs::create_dir(root.join("save")).unwrap();
        let mut raw = b"PS2A".to_vec();
        for n in [
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
            raw.extend(n.to_le_bytes());
        }
        raw.extend(
            code.iter()
                .chain(text)
                .map(|b| b.rotate_left(1).wrapping_add(124)),
        );
        fs::write(root.join("data/pack/start.ps3"), raw).unwrap();
        let bytes: Vec<_> = include_str!("fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        fs::write(root.join("data/pack/script.cpz"), bytes).unwrap();
        Self(root)
    }
    fn script(&self) -> Script {
        load_start(&self.0).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        for name in ["data/pack/start.ps3", "data/pack/script.cpz"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        for name in ["data/pack", "data", "save"] {
            let _ = fs::remove_dir(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
fn string_call(op: u16) -> Vec<u8> {
    let mut b = 0x201u16.to_le_bytes().to_vec();
    b.extend(0x120u16.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(0x20fu16.to_le_bytes());
    b.extend(0x430u16.to_le_bytes());
    b.extend(op.to_le_bytes());
    b
}

#[test]
fn startup_main_snapshot_barrier_is_sticky_and_does_not_execute_new_main() {
    let root = Root::new("barrier", &string_call(0x2080), b"fixture.ps3\0");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::MainPreparedNeedsSnapshot
    );
    assert_eq!(run.machine().current_name(), b"fixture.ps3");
    let before = (
        run.steps(),
        run.machine().pc(),
        run.machine().stack().len_bytes(),
    );
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::MainPreparedNeedsSnapshot
    );
    assert_eq!(
        (
            run.steps(),
            run.machine().pc(),
            run.machine().stack().len_bytes()
        ),
        before
    );
}
#[test]
fn explicit_policy_enters_main_without_building_a_save_image() {
    let root = Root::new("no-snapshot", &string_call(0x2080), b"fixture.ps3\0");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    run.set_snapshot_policy(SnapshotPolicy::ContinueWithoutImage);
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(*run.save_image(), SaveImage::Absent);
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::MainEnteredWithoutSnapshot
    );
    assert_eq!(
        *run.save_image(),
        SaveImage::Unavailable {
            script: b"fixture.ps3".to_vec(),
            pc: 0
        }
    );
    assert_eq!(run.machine().state().bit(0), Some(false));
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(
        (
            run.steps(),
            run.machine().pc(),
            run.machine().stack().len_bytes()
        ),
        (4, 8, 4)
    );
}
#[test]
fn startup_records_clear_color_without_drawing() {
    let mut code = 0x440u16.to_le_bytes().to_vec();
    code.extend(4u16.to_le_bytes());
    code.extend(0x00ff_8000u32.to_le_bytes());
    code.extend(0x22cbu16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("clear", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.scene().clear().rgba(), [0xff, 0x80, 0, 255]);
    assert!(backend.requests().is_empty());
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
}
struct Reject;
impl PlatformBackend for Reject {
    fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        Err("synthetic backend failure".into())
    }
}
#[test]
fn startup_backend_error_can_be_retried_without_losing_pending_request() {
    let mut code = string_call(0x2160);
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("retry", &code, b"title\0");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    run.tick(&mut Reject).unwrap();
    run.tick(&mut Reject).unwrap();
    assert!(run.tick(&mut Reject).is_err());
    let pending = run.machine().pending_system();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert!(pending.is_some());
    assert!(run.machine().pending_system().is_none());
    assert_eq!(backend.requests().len(), 1);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
}
#[test]
fn missing_system_file_is_a_verified_no_change_return() {
    let mut code = 0x2128u16.to_le_bytes().to_vec();
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("missing-system", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.machine().state().integer(2048), None);
    assert_eq!(run.machine().pc(), 2);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
}

#[test]
fn startup_fade_wait_needs_explicit_frames_and_resumes_after_removal() {
    let mut code = vec![];
    for value in [9u32, 100, 0, 1] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(0x2220u16.to_le_bytes());
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0u32.to_le_bytes());
    code.extend(0x2224u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("fade-wait", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..6 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    let pc = run.machine().pc();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.scene().fade().current().unwrap().elapsed, 0);
    run.advance_frame(100);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(run.machine().pc(), pc + 2);
    assert!(run.scene().fade().current().is_none());
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
}

#[test]
fn message_recording_is_opt_in_and_resumes_each_yield_with_explicit_frame_time() {
    let mut code = vec![];
    for (op, words) in [
        (0x215du16, vec![720u32, 1280, 100, 100, 0, 0, 7, 0]),
        (0x2152, vec![0, u32::MAX, 0]),
    ] {
        for word in words {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(word.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    }
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("message-record", &code, b"A\0");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        if let StartupEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x2152);
            break;
        }
    }
    assert!(run.messages().text(7).unwrap().glyphs.is_empty());
    run.set_record_text(true);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 1);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 2);
    assert_eq!(run.messages().text(7).unwrap().glyphs.len(), 1);
    run.advance_frame(400);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 0);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.timers().values()[0], 400);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
    assert_eq!(run.timers().values()[0], 400);
}

#[test]
fn timers_advance_at_run_entry_and_ignore_budget_slices_and_reused_deltas() {
    let mut code = vec![];
    for (op, slot) in [(0x200bu16, 0u32), (0x200c, 0), (0x200c, 1)] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(slot.to_le_bytes());
        code.extend(op.to_le_bytes());
    }
    code.extend(0x2000u16.to_le_bytes());
    for slot in [0u32, 1] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(slot.to_le_bytes());
        code.extend(0x200cu16.to_le_bytes());
    }
    code.extend(0x2000u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("timer-boundary", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.advance_frame(100);
    for _ in 0..4 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    run.advance_frame(7); // Host budget slice, not a VM yield.
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().system_integer(0), Some(100));
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    run.advance_frame(16);
    for expected in [16, 116] {
        for _ in 0..2 {
            run.tick(&mut backend).unwrap();
        }
        assert_eq!(run.machine().state().system_integer(0), Some(expected));
    }
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    // No new frame delta: retrying the host must not reuse the previous 16ms.
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
    assert_eq!(&run.timers().values()[..2], &[16, 116]);
}

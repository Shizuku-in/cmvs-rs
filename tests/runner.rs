#[path = "support/bytecode.rs"]
#[allow(dead_code)]
mod commands;
use cmvs_rs::{
    digest::{sha256, to_hex},
    platform::{PlatformBackend, PlatformRequest, RecordingBackend},
    profile::{Game, Registry},
    ps3::Script,
    runner::{SnapshotPolicy, Startup, StartupEvent, load_start},
    snapshot::SaveImage,
};
use std::{error::Error, fs, path::PathBuf, sync::OnceLock};

/// A synthetic build whose 2048 cases all count as base semantics, so these tests exercise
/// the runner rather than any real profile table.
fn synthetic_profile() -> String {
    let mut text = format!(
        "profile synthetic-base\nfamily cmvs\nbase -\nbinary x64 file=cmvs64.exe sha256={} \
         timestamp=0x00000000 linker=10.0 image-base=0x140000000 table=0x1000 default=0x2000 \
         cases=2048\n",
        "0".repeat(64)
    );
    for command in 0..2048 {
        text.push_str(&format!("0x{:04x} base 0x3000\n", 0x2000 + command));
    }
    text
}
fn base_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::parse(&[&synthetic_profile()], "").unwrap())
}

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
    /// Synthetic roots match no game record, so they name the synthetic profile explicitly.
    fn game(&self) -> Game<'static> {
        base_registry()
            .identify_game(&self.0, Some("synthetic-base"))
            .unwrap()
    }
    /// A registry whose record matches this root's files and verifies skip-auto-resume.
    fn recorded_registry(&self, game_id: u32) -> Registry {
        let hash = |name: &str| to_hex(&sha256(&fs::read(self.0.join(name)).unwrap()));
        let games = format!(
            "game synthetic\nprofile synthetic-base\nstart data/pack/start.ps3 sha256={}\n\
             archive data/pack/script.cpz sha256={}\ngame-id 0x{game_id:08x}\n\
             policy skip-auto-resume integer=2198\nend\n",
            hash("data/pack/start.ps3"),
            hash("data/pack/script.cpz")
        );
        Registry::parse(&[&synthetic_profile()], &games).unwrap()
    }
    fn script(&self) -> Script {
        load_start(&self.0, &self.game().files).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        for name in [
            "data/pack/start.ps3",
            "data/pack/script.cpz",
            "save/system.dat",
            "cmvs.cfg",
        ] {
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
fn slot_exchange_and_move_preserve_raw_files_and_use_only_the_user_directory() {
    use std::collections::BTreeMap;
    for op in [0x228cu16, 0x228e] {
        for (first, second) in [(0, 1), (999, 998), (3, 3)] {
            for presence in 0..4 {
                let mut code = vec![];
                commands::command(&mut code, op, &[first, second]);
                let name = format!("slot-edit-{op}-{first}-{second}-{presence}");
                let root = Root::new(&name, &code, b"");
                let user = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(&name);
                let script = root.script();
                let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
                run.set_user_data_directory(&user).unwrap();
                let (a, b) = (
                    format!("save{first:03}.dat"),
                    format!("save{second:03}.dat"),
                );
                let mut expected = BTreeMap::from([
                    ("savetemp.dat".to_string(), b"old temp".to_vec()),
                    ("savedel.dat".to_string(), b"old deleted".to_vec()),
                ]);
                if presence & 1 != 0 {
                    expected.insert(a.clone(), b"first raw payload".to_vec());
                }
                if presence & 2 != 0 {
                    expected.insert(b.clone(), b"second raw payload".to_vec());
                }
                for (name, bytes) in &expected {
                    fs::write(user.join(name), bytes).unwrap();
                }
                // Same names exist in the synthetic game directory and must remain untouched.
                fs::write(root.0.join("save").join(&a), b"game file").unwrap();
                if op == 0x228c {
                    expected.remove("savetemp.dat");
                    if first != second {
                        let old_a = expected.remove(&a);
                        let old_b = expected.remove(&b);
                        if let Some(bytes) = old_a {
                            expected.insert(b.clone(), bytes);
                        }
                        if let Some(bytes) = old_b {
                            expected.insert(a.clone(), bytes);
                        }
                    }
                } else {
                    expected.remove("savedel.dat");
                    if expected.contains_key(&b) {
                        if let Some(old) = expected.remove(&a) {
                            expected.insert("savedel.dat".to_string(), old);
                        }
                        if let Some(bytes) = expected.remove(&b) {
                            expected.insert(a.clone(), bytes);
                        }
                    }
                }
                let mut backend = RecordingBackend::default();
                for _ in 0..3 {
                    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
                }
                assert_eq!(run.machine().pc(), code.len());
                assert_eq!(run.machine().stack().len_bytes(), 0);
                for name in [&a, &b, "savetemp.dat", "savedel.dat"] {
                    assert_eq!(
                        fs::read(user.join(name)).ok().as_ref(),
                        expected.get(name),
                        "{op:x} {first}/{second} presence={presence} {name}"
                    );
                }
                assert_eq!(
                    fs::read(root.0.join("save").join(&a)).unwrap(),
                    b"game file"
                );
                drop(run);
                fs::remove_file(root.0.join("save").join(&a)).unwrap();
                fs::remove_dir_all(user).unwrap();
            }
        }
    }
}

#[test]
fn slot_edits_ignore_out_of_range_words_and_require_a_user_directory_for_valid_slots() {
    for op in [0x228c, 0x228e] {
        for args in [[1000, 0], [0, u32::MAX], [0, 1]] {
            let mut code = vec![];
            commands::command(&mut code, op, &args);
            let root = Root::new(&format!("edit-invalid-{op}-{}", args[0]), &code, b"");
            let script = root.script();
            let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
            let mut backend = RecordingBackend::default();
            for _ in 0..2 {
                run.tick(&mut backend).unwrap();
            }
            if args == [0, 1] {
                assert!(run.tick(&mut backend).is_err());
                assert!(run.machine().pending_system().is_some());
                assert_eq!(run.machine().stack().len_bytes(), 8);
            } else {
                assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
                assert_eq!(run.machine().stack().len_bytes(), 0);
            }
        }
    }
}

#[test]
fn slot_exchange_refreshes_existing_menu_labels_and_latest_selection() {
    let mut code = vec![];
    commands::command(&mut code, 0x22b3, &[2]);
    commands::command(&mut code, 0x228c, &[0, 1]);
    let root = Root::new("slot-edit-menu", &code, b"");
    let user = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("slot-edit-menu");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    let labels: [&[u8]; 2] = [b"2020-01-01 00:00:00 first", b"2021-01-01 00:00:00 second"];
    for (slot, label) in labels.iter().enumerate() {
        let mut raw = vec![0; 512];
        raw[..4].copy_from_slice(b"CSV2");
        raw[16..16 + label.len()].copy_from_slice(label);
        fs::write(user.join(format!("save{slot:03}.dat")), raw).unwrap();
    }
    let mut backend = RecordingBackend::default();
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.save_menu().menu().unwrap().latest, Some(1));
    for _ in 0..3 {
        run.tick(&mut backend).unwrap();
    }
    let menu = run.save_menu().menu().unwrap();
    assert_eq!(menu.latest, Some(0));
    let cmvs_rs::save_menu::SlotStatus::Recognized(header) = &menu.slots[0] else {
        panic!("header");
    };
    assert_eq!(header.label, labels[1]);
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

#[test]
#[cfg(windows)]
fn failed_slot_swap_resumes_after_the_successful_rename_without_swapping_twice() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut code = vec![];
    commands::command(&mut code, 0x228c, &[0, 1]);
    let root = Root::new("slot-edit-retry", &code, b"");
    let user = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("slot-edit-retry");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    fs::write(user.join("save000.dat"), b"first").unwrap();
    fs::write(user.join("save001.dat"), b"second").unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(user.join("save001.dat"))
        .unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
    }
    for _ in 0..2 {
        assert!(run.tick(&mut backend).is_err());
        assert!(!user.join("save000.dat").exists());
        assert_eq!(fs::read(user.join("savetemp.dat")).unwrap(), b"first");
    }
    assert!(run.save_slot(0).is_err());
    assert!(run.load_slot(0, &mut backend).is_err());
    drop(locked);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(fs::read(user.join("save000.dat")).unwrap(), b"second");
    assert_eq!(fs::read(user.join("save001.dat")).unwrap(), b"first");
    assert!(!user.join("savetemp.dat").exists());
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

#[test]
fn user_configuration_round_trips_live_settings_and_preserves_the_game_file() {
    use cmvs_rs::settings::DisplayPreferences;
    let mut code = vec![];
    for (op, value) in [
        (0x22e1, 7),
        (0x22e0, 1),
        (0x22d5, 0),
        (0x22d9, 123),
        (0x22d0, 2),
        (0x22d3, 1),
        (0x22e7, 8),
        (0x22e9, 4),
    ] {
        commands::command(&mut code, op, &[1, value]);
    }
    code.extend(0x2000u16.to_le_bytes());
    let root = Root::new("preferences", &code, b"");
    let original = b"[MESSAGE_INFO]\nTEXT_WAIT=2\nTEXT_BOLD=0\n[FUTURE]\nUNKNOWN=\x82\xa0\n";
    fs::write(root.0.join("cmvs.cfg"), original).unwrap();
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("preferences-{}", std::process::id()));
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(!run.save_preferences(DisplayPreferences::default()).unwrap());
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    let mut yielded = false;
    for _ in 0..100 {
        if run.tick(&mut backend).unwrap() == StartupEvent::Yield {
            yielded = true;
            break;
        }
    }
    assert!(yielded);
    let settings = run.settings().clone();
    let audio = run.audio().settings();
    let duck = run.audio().voice_ducking();
    let flags = run.message_flags();
    let confirmations = run.save_menu().confirmations();
    run.save_preferences(DisplayPreferences {
        fullscreen: true,
        aspect_mode: 2,
        aspect_scaling: true,
        render_mode: 0,
    })
    .unwrap();
    let first = fs::read(directory.join("cmvs.cfg")).unwrap();
    run.save_preferences(DisplayPreferences {
        fullscreen: false,
        aspect_mode: 3,
        aspect_scaling: false,
        render_mode: 0,
    })
    .unwrap();
    assert_eq!(fs::read(directory.join("cmvs.cfg.bak")).unwrap(), first);
    drop(run);
    let mut resumed = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    resumed.set_user_data_directory(&directory).unwrap();
    assert_eq!(resumed.settings(), &settings);
    assert_eq!(resumed.audio().settings(), audio);
    assert_eq!(resumed.audio().voice_ducking(), duck);
    assert_eq!(resumed.message_flags().mode_1548(), flags.mode_1548());
    assert_eq!(resumed.save_menu().confirmations(), confirmations);
    assert_eq!(
        resumed.display_preferences(),
        DisplayPreferences {
            fullscreen: false,
            aspect_mode: 3,
            aspect_scaling: false,
            render_mode: 0
        }
    );
    assert_eq!(fs::read(root.0.join("cmvs.cfg")).unwrap(), original);
    assert!(
        fs::read(directory.join("cmvs.cfg"))
            .unwrap()
            .windows(b"UNKNOWN=\x82\xa0\n".len())
            .any(|w| w == b"UNKNOWN=\x82\xa0\n")
    );
    // Point mode 0 is stored as the original's swapped word 1.
    assert!(
        fs::read(directory.join("cmvs.cfg"))
            .unwrap()
            .windows(b"ASPECT_RATIO_FILTER=1\r\n".len())
            .any(|w| w == b"ASPECT_RATIO_FILTER=1\r\n")
    );
    drop(resumed);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn immediate_readback_retries_until_a_draw_and_ignores_unflushed_scene_changes() {
    use cmvs_rs::raster::RgbaImage;
    let mut code = vec![];
    commands::command(&mut code, 0x2020, &[3]);
    commands::command(&mut code, 0x22cb, &[0xabcdef]);
    commands::command(&mut code, 0x203e, &[3, u32::MAX]);
    commands::command(&mut code, 0x22cb, &[0x123456]);
    commands::command(&mut code, 0x203e, &[3, u32::MAX]);
    let root = Root::new("immediate-readback", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    while run.machine().pc() < code.len() - 2 {
        if run.tick(&mut backend).is_err() {
            assert_eq!(run.machine().pending_system().unwrap().1, 0x3e);
            assert_eq!(run.machine().stack().len_bytes(), 8);
            assert!(run.scene().object(3).unwrap().image.is_none());
            // Completed device pixels can differ from current, not-yet-drawn VM state.
            backend.present_frame(RgbaImage::new(1, 1, vec![11, 22, 33, 0]).unwrap());
            assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
        }
    }
    assert_eq!(run.scene().clear().color, 0x123456);
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert_eq!(
        run.scene()
            .object(3)
            .unwrap()
            .image
            .as_ref()
            .unwrap()
            .pixels
            .pixels(),
        &[11, 22, 33, 255]
    );
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.machine().state().system_integer(0), Some(1));
}

#[test]
fn quit_is_terminal_and_does_not_execute_the_following_command() {
    let root = Root::new("quit", &[1, 0x20, 0xff, 0x27], b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Quit);
    assert_eq!(run.machine().pc(), 2);
    assert!(run.machine().pending_system().is_none());
    let steps = run.steps();
    run.set_focused(false);
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Quit);
    }
    assert_eq!(run.steps(), steps);
}

#[test]
fn reset_missing_main_keeps_the_command_pending_and_old_state_intact() {
    let mut code = vec![];
    commands::command(&mut code, 0x22cb, &[0x123456]);
    commands::command(&mut code, 0x22eb, &[1, 1]);
    commands::command(&mut code, 0x2003, &[]);
    let root = Root::new("reset-missing", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    while run.machine().pc() != code.len() - 2 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    for _ in 0..2 {
        assert!(run.tick(&mut backend).is_err());
        assert_eq!(run.machine().pending_system(), Some((code.len() - 2, 3)));
        assert_eq!(run.scene().clear().color, 0x123456);
        assert!(run.message_flags().skip());
    }
}

#[test]
fn inactive_policy_freezes_vm_and_scene_but_keeps_audio_updates_without_time_catchup() {
    use cmvs_rs::audio::{AudioBackend, AudioRequest};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Audio(Arc<AtomicUsize>);
    impl AudioBackend for Audio {
        fn configure_voice_ducking(
            &mut self,
            _: cmvs_rs::audio::VoiceDuckingSettings,
        ) -> Result<(), Box<dyn Error>> {
            Ok(())
        }
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            unreachable!()
        }
        fn update(&mut self) -> Result<(), Box<dyn Error>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    for active_draw in [false, true] {
        let mut code = vec![];
        commands::command(&mut code, 0x2220, &[0, 0, 1000, 9]);
        for _ in 0..10 {
            code.extend(0x2000u16.to_le_bytes());
        }
        let root = Root::new(
            if active_draw {
                "background-run"
            } else {
                "background-pause"
            },
            &code,
            b"",
        );
        fs::write(
            root.0.join("cmvs.cfg"),
            format!("[SYSTEM_INFO]\nACTIVE_DRAW={}\n", u8::from(active_draw)),
        )
        .unwrap();
        let script = root.script();
        let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        let updates = Arc::new(AtomicUsize::new(0));
        run.set_audio_backend(Box::new(Audio(updates.clone())))
            .unwrap();
        let mut backend = RecordingBackend::default();
        for _ in 0..30 {
            if run.tick(&mut backend).unwrap() == StartupEvent::Yield {
                break;
            }
        }
        run.advance_decoded_frame(16).unwrap();
        let steps = run.steps();
        let clock = run.timers().clock();
        let fade = run.scene().fade().current().unwrap().elapsed;
        run.set_focused(false);
        assert_eq!(run.frame_running(), active_draw);
        for _ in 0..5 {
            assert_eq!(
                run.tick(&mut backend).unwrap(),
                if active_draw {
                    StartupEvent::Yield
                } else {
                    StartupEvent::Inactive
                }
            );
            run.advance_decoded_frame(10).unwrap();
        }
        if active_draw {
            assert_eq!(run.steps(), steps + 5);
        } else {
            assert_eq!(run.steps(), steps);
            assert_eq!(run.timers().clock(), clock);
            assert_eq!(run.scene().fade().current().unwrap().elapsed, fade);
            // The recording-only frame entry must respect the same pause policy.
            run.advance_frame(60_000).unwrap();
            assert_eq!(run.scene().fade().current().unwrap().elapsed, fade);
        }
        let clock = run.timers().clock();
        run.set_focused(true);
        run.advance_decoded_frame(16).unwrap();
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
        assert_eq!(run.timers().clock(), clock + 16);
        assert_eq!(updates.load(Ordering::SeqCst), 7);
    }
}

#[test]
fn fullscreen_request_yields_once_and_query_resumes_on_the_next_run() {
    let mut code = vec![];
    commands::command(&mut code, 0x2164, &[1]);
    commands::command(&mut code, 0x2163, &[]);
    let root = Root::new("fullscreen-yield", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    let mut yielded = false;
    for _ in 0..20 {
        if run.tick(&mut backend).unwrap() == StartupEvent::Yield {
            yielded = true;
            break;
        }
    }
    assert!(yielded);
    assert_eq!(backend.requests(), &[PlatformRequest::Fullscreen(true)]);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    run.advance_frame(16).unwrap();
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
        if run.machine().state().system_integer(0) == Some(1) {
            return;
        }
    }
    panic!("fullscreen query did not run after the yield");
}

#[test]
fn audio_backend_receives_startup_ducking_and_failed_replacement_keeps_old_device() {
    use cmvs_rs::audio::{AudioBackend, AudioRequest, VoiceDuckingSettings};
    use std::sync::{Arc, Mutex};
    struct Probe {
        settings: Arc<Mutex<Vec<VoiceDuckingSettings>>>,
        updates: Arc<Mutex<usize>>,
        reject: bool,
    }
    impl AudioBackend for Probe {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
            unreachable!()
        }
        fn configure_voice_ducking(
            &mut self,
            settings: VoiceDuckingSettings,
        ) -> Result<(), Box<dyn Error>> {
            if self.reject {
                return Err("synthetic configuration failure".into());
            }
            self.settings.lock().unwrap().push(settings);
            Ok(())
        }
        fn update(&mut self) -> Result<(), Box<dyn Error>> {
            *self.updates.lock().unwrap() += 1;
            Ok(())
        }
    }
    let root = Root::new("ducking-backend", &0x20a1u16.to_le_bytes(), b"");
    fs::write(root.0.join("cmvs.cfg"), b"[SOUND_INFO]\nVOICE_FADE_VOLOME=42\nVOICE_FADE_TIMER=123\nVOICE_FADE_WAIT=321\nVOICE_FADE_USE=1\n").unwrap();
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let settings = Arc::new(Mutex::new(vec![]));
    let updates = Arc::new(Mutex::new(0));
    run.set_audio_backend(Box::new(Probe {
        settings: settings.clone(),
        updates: updates.clone(),
        reject: false,
    }))
    .unwrap();
    assert_eq!(
        *settings.lock().unwrap(),
        vec![VoiceDuckingSettings {
            percent: 42,
            fade_ms: 123,
            wait_ms: 321,
            include_fading: true
        }]
    );
    assert!(
        run.set_audio_backend(Box::new(Probe {
            settings: settings.clone(),
            updates: updates.clone(),
            reject: true
        }))
        .is_err()
    );
    run.advance_decoded_frame(16).unwrap();
    assert_eq!(*updates.lock().unwrap(), 1);
    assert_eq!(settings.lock().unwrap().len(), 1);
}

#[test]
fn startup_main_snapshot_barrier_is_sticky_and_does_not_execute_new_main() {
    let root = Root::new("barrier", &string_call(0x2080), b"fixture.ps3\0");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_snapshot_policy(SnapshotPolicy::Barrier);
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
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
/// 0x0400 to `target`, as raw code bytes.
fn jump(target: u32) -> Vec<u8> {
    let mut code = 0x400u16.to_le_bytes().to_vec();
    code.extend(target.to_le_bytes());
    code
}

#[test]
fn yield_watchdog_stops_a_loop_that_never_yields_without_stepping() {
    let root = Root::new("watchdog", &jump(0), b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.set_yield_watchdog(3);
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    for _ in 0..2 {
        let error = run.tick(&mut backend).unwrap_err().to_string();
        assert!(error.contains("watchdog"), "{error}");
        assert_eq!((run.steps(), run.machine().pc()), (3, 0));
    }
    run.set_yield_watchdog(5);
    for _ in 0..2 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    assert!(run.tick(&mut backend).is_err());
    assert_eq!(run.steps(), 5);
}

#[test]
fn yields_reset_the_watchdog_and_total_steps_are_unbounded() {
    // 0x2000 yields, then 0x0400 jumps back to it.
    let mut code = 0x2000u16.to_le_bytes().to_vec();
    code.extend(jump(0));
    let root = Root::new("yielding", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.set_yield_watchdog(2);
    for _ in 0..1000 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    assert_eq!(run.steps(), 2000);
    // One step after the last yield already reaches a watchdog of 1.
    run.set_yield_watchdog(1);
    assert!(run.tick(&mut backend).is_err());
    assert_eq!(run.steps(), 2000);
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..6 {
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    }
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    let pc = run.machine().pc();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.scene().fade().current().unwrap().elapsed, 0);
    run.advance_frame(100).unwrap();
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
fn message_wait_resumes_without_advancing_time_on_release_completion() {
    let mut code = vec![];
    // Startup READ_SKIP now follows the INI default, so establish unread ID 0.
    system_call(&mut code, 0x2091, &[0, 1, 0]);
    for (op, words) in [
        (0x215du16, vec![720u32, 1280, 100, 100, 0, 0, 7, 0]),
        (0x2153, vec![u32::MAX, 0, 0]),
    ] {
        for word in words {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(word.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    }
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("message-wait", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        if run.tick(&mut backend).unwrap() == StartupEvent::Yield {
            break;
        }
    }
    let pc = run.machine().pc();
    assert_eq!(run.machine().pending_system(), Some((pc, 0x153)));
    run.advance_frame(16).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.timers().values()[0], 16);
    run.input_mut().set(0, false, false, true).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
    assert!(!run.input().released(0));
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.timers().values()[0], 16);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
    assert_eq!(run.timers().values()[0], 16);
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
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
    run.advance_frame(400).unwrap();
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
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.advance_frame(100).unwrap();
    for _ in 0..4 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    run.advance_frame(7).unwrap(); // Host budget slice, not a VM yield.
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().system_integer(0), Some(100));
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    run.advance_frame(16).unwrap();
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

#[test]
fn stopwatch_uses_frame_time_once_per_vm_run() {
    let mut code = vec![];
    for op in [0x200eu16, 0x200f, 0x2000, 0x200f, 0x200f, 0x2000, 0x200f] {
        code.extend(op.to_le_bytes());
    }
    let root = Root::new("stopwatch-boundary", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.advance_frame(100).unwrap();
    run.tick(&mut backend).unwrap(); // Mark at 100ms.
    run.advance_frame(7).unwrap(); // A budget slice does not advance the VM clock.
    run.tick(&mut backend).unwrap();
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    run.advance_frame(16).unwrap();
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
        assert_eq!(run.machine().state().system_integer(0), Some(16));
    }
    assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Yield);
    run.tick(&mut backend).unwrap(); // No new delta: the previous 16ms is not reused.
    assert_eq!(run.machine().state().system_integer(0), Some(16));
    assert_eq!(run.timers().clock(), 116);
}

// Synthetic CSS1, literal-only LZSS; no game bytes or external assets.
fn auto_resume_save(game_id: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 44864];
    payload[32512 + 4 * (2198 - 2048)..32516 + 4 * (2198 - 2048)]
        .copy_from_slice(&1i32.to_le_bytes());
    payload[32512 + 4 * (2199 - 2048)..32516 + 4 * (2199 - 2048)]
        .copy_from_slice(&77i32.to_le_bytes());
    payload[10] = 4; // Persistent bit 2130 remains set.
    let mut packed = Vec::new();
    for chunk in payload.chunks(8) {
        packed.push(((1u16 << chunk.len()) - 1) as u8);
        packed.extend_from_slice(chunk);
    }
    let mut raw = vec![0u8; 360];
    raw[..4].copy_from_slice(b"CSS1");
    for (offset, value) in [
        (4, 104),
        (68, game_id),
        (92, 44864),
        (96, packed.len() as u32),
        (100, 360 + packed.len() as u32),
    ] {
        raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    raw[76] = 1;
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, byte) in packed.iter().enumerate() {
        raw.push((byte.wrapping_add(64) ^ key[i % key.len()]).rotate_left(2));
    }
    let checksum = raw[360..]
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    raw.extend(checksum.to_le_bytes());
    raw
}

#[test]
fn system_save_export_is_opt_in_external_and_becomes_the_read_source() {
    use cmvs_rs::system_save::Css1Save;
    let mut code = vec![];
    let mut call = |op: u16, args: &[u32]| {
        for value in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    };
    call(0x201d, &[0x12345678]);
    call(0x2128, &[]);
    call(0x2092, &[2048, 1, 42]);
    call(0x2129, &[]);
    call(0x2092, &[2048, 1, 99]);
    call(0x2128, &[]);
    call(0x27ff, &[]);
    let root = Root::new("system-export", &code, b"");
    let original = auto_resume_save(0x12345678);
    fs::write(root.0.join("save/system.dat"), &original).unwrap();
    let output = root.0.with_extension("css1");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(
        run.set_system_save_output(&root.0.join("save/new.dat"))
            .is_err()
    );
    let mut backend = RecordingBackend::default();
    loop {
        if let StartupEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x2129);
            break;
        }
    }
    assert!(!output.exists());
    assert!(run.set_system_save_output(&output).is_err()); // Too late to configure.
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_system_save_output(&output).unwrap();
    loop {
        if let StartupEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x27ff);
            break;
        }
    }
    assert_eq!(run.machine().state().integer(2048), Some(42));
    let saved = Css1Save::decode(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        saved
            .applied(run.machine().state(), Some(0x12345678))
            .unwrap()
            .integer(2048),
        Some(42)
    );
    assert_eq!(fs::read(root.0.join("save/system.dat")).unwrap(), original);
    fs::remove_file(output).unwrap();
}

#[test]
fn user_directory_saves_twice_restarts_and_never_falls_back_from_corrupt_primary() {
    use cmvs_rs::{evaluate::State, system_save::Css1Save};
    let mut code = vec![];
    let mut call = |op: u16, args: &[u32]| {
        for value in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    };
    call(0x201d, &[0x12345678]);
    call(0x2128, &[]);
    call(0x2092, &[2048, 1, 42]);
    call(0x2129, &[]);
    call(0x2092, &[2048, 1, 99]);
    call(0x2129, &[]);
    call(0x2092, &[2048, 1, 17]);
    call(0x2128, &[]);
    call(0x27ff, &[]);
    let root = Root::new("user-directory", &code, b"");
    let original = auto_resume_save(0x12345678);
    fs::write(root.0.join("save/system.dat"), &original).unwrap();
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("cmvs-runner-profile-{}", std::process::id()));
    assert!(!directory.exists());
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(
        run.set_user_data_directory(&root.0.join("save/profile"))
            .is_err()
    );
    run.set_user_data_directory(&directory).unwrap();
    assert!(
        run.set_system_save_output(&directory.join("other.dat"))
            .is_err()
    );
    let mut backend = RecordingBackend::default();
    loop {
        if let StartupEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x27ff);
            break;
        }
    }
    assert_eq!(run.machine().state().integer(2048), Some(99));
    let stored = Css1Save::decode(&fs::read(directory.join("system.dat")).unwrap()).unwrap();
    let backup_raw = fs::read(directory.join("system.bak")).unwrap();
    let backup = Css1Save::decode(&backup_raw).unwrap();
    assert_eq!(
        stored
            .applied(&State::default(), Some(0x12345678))
            .unwrap()
            .integer(2048),
        Some(99)
    );
    assert_eq!(
        backup
            .applied(&State::default(), Some(0x12345678))
            .unwrap()
            .integer(2048),
        Some(42)
    );
    drop(run);
    let mut restarted = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    restarted.set_user_data_directory(&directory).unwrap();
    while restarted.machine().state().integer(2048).is_none() {
        restarted.tick(&mut backend).unwrap();
    }
    assert_eq!(restarted.machine().state().integer(2048), Some(99));
    drop(restarted);
    fs::write(directory.join("system.dat"), b"corrupt profile").unwrap();
    let mut corrupt = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    corrupt.set_user_data_directory(&directory).unwrap();
    let mut failed = false;
    for _ in 0..20 {
        if corrupt.tick(&mut backend).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert_eq!(corrupt.machine().state().integer(2048), None);
    assert_eq!(fs::read(root.0.join("save/system.dat")).unwrap(), original);
    assert_eq!(fs::read(directory.join("system.bak")).unwrap(), backup_raw);
    drop(corrupt);
    for name in ["system.dat", "system.bak", ".cmvs-system.lock"] {
        fs::remove_file(directory.join(name)).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}
fn auto_resume_code(game_id: u32, reads: usize) -> Vec<u8> {
    let mut code = 0x442u16.to_le_bytes().to_vec();
    code.extend(4u16.to_le_bytes());
    code.extend(game_id.to_le_bytes());
    code.extend(0x201du16.to_le_bytes());
    for _ in 0..reads {
        code.extend(0x2128u16.to_le_bytes());
    }
    code.extend(0x27ffu16.to_le_bytes());
    code
}

#[test]
fn auto_resume_policy_is_explicit_preserves_other_settings_and_never_writes_saves() {
    for skip in [false, true] {
        let code = auto_resume_code(0x1780_0016, 1);
        let root = Root::new(&format!("auto-resume-{skip}"), &code, b"");
        let raw = auto_resume_save(0x1780_0016);
        let path = root.0.join("save/system.dat");
        fs::write(&path, &raw).unwrap();
        let script = root.script();
        let registry = root.recorded_registry(0x1780_0016);
        let game = registry.identify_game(&root.0, None).unwrap();
        let mut run = Startup::new(&script, &root.0, &game, 0).unwrap();
        if skip {
            run.set_skip_auto_resume(true).unwrap();
        }
        let mut backend = RecordingBackend::default();
        for _ in 0..3 {
            assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
        }
        assert_eq!(
            run.machine().state().integer(2198),
            Some(if skip { 0 } else { 1 })
        );
        assert_eq!(run.machine().state().integer(2199), Some(77));
        assert_eq!(run.machine().state().bit(2130), Some(true));
        assert_eq!(run.auto_resume_skipped(), skip);
        assert!(matches!(
            run.tick(&mut backend).unwrap(),
            StartupEvent::Unhandled {
                command: 0x27ff,
                ..
            }
        ));
        assert_eq!(fs::read(path).unwrap(), raw);
        assert!(run.set_skip_auto_resume(false).is_err());
    }
}

#[test]
fn auto_resume_policy_only_overrides_the_first_system_save_read() {
    let code = auto_resume_code(0x1780_0016, 2);
    let root = Root::new("auto-resume-once", &code, b"");
    fs::write(
        root.0.join("save/system.dat"),
        auto_resume_save(0x1780_0016),
    )
    .unwrap();
    let script = root.script();
    let registry = root.recorded_registry(0x1780_0016);
    let game = registry.identify_game(&root.0, None).unwrap();
    let mut run = Startup::new(&script, &root.0, &game, 0).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..3 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().integer(2198), Some(0));
    run.tick(&mut backend).unwrap();
    assert_eq!(run.machine().state().integer(2198), Some(1));
}

#[test]
fn auto_resume_policy_rejects_other_games_without_applying_system_state() {
    let code = auto_resume_code(0x1234_5678, 1);
    let root = Root::new("auto-resume-wrong-profile", &code, b"");
    let raw = auto_resume_save(0x1234_5678);
    fs::write(root.0.join("save/system.dat"), &raw).unwrap();
    let script = root.script();
    // Without a game record the policy cannot be enabled at all.
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(run.set_skip_auto_resume(true).is_err());
    // A record verified for another game ID stops at 0x201d, before any system state.
    let registry = root.recorded_registry(0x1780_0016);
    let game = registry.identify_game(&root.0, None).unwrap();
    let mut run = Startup::new(&script, &root.0, &game, 0).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    let mut backend = RecordingBackend::default();
    let error = loop {
        match run.tick(&mut backend) {
            Ok(_) => {}
            Err(error) => break error.to_string(),
        }
    };
    assert!(error.contains("declares game ID 0x12345678"), "{error}");
    assert_eq!(run.machine().pending_system().map(|(_, c)| c), Some(0x1d));
    assert_eq!(run.machine().state().integer(2198), None);
    assert!(!run.auto_resume_skipped());
    assert_eq!(fs::read(root.0.join("save/system.dat")).unwrap(), raw);
}

#[test]
fn auto_resume_policy_keeps_missing_and_invalid_system_saves_distinct_and_retryable() {
    let code = auto_resume_code(0x1780_0016, 2);
    let root = Root::new("auto-resume-missing", &code, b"");
    let script = root.script();
    let registry = root.recorded_registry(0x1780_0016);
    let game = registry.identify_game(&root.0, None).unwrap();
    let mut run = Startup::new(&script, &root.0, &game, 0).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..3 {
        run.tick(&mut backend).unwrap();
    }
    assert_eq!(run.machine().state().integer(2198), None);
    assert!(!run.auto_resume_skipped());
    // Missing first read consumes the startup policy; a later load is not overridden.
    fs::write(
        root.0.join("save/system.dat"),
        auto_resume_save(0x1780_0016),
    )
    .unwrap();
    run.tick(&mut backend).unwrap();
    assert_eq!(run.machine().state().integer(2198), Some(1));

    let root = Root::new("auto-resume-invalid", &code, b"");
    let mut raw = auto_resume_save(0x1780_0016);
    raw[360] ^= 1;
    fs::write(root.0.join("save/system.dat"), &raw).unwrap();
    let script = root.script();
    let registry = root.recorded_registry(0x1780_0016);
    let game = registry.identify_game(&root.0, None).unwrap();
    let mut run = Startup::new(&script, &root.0, &game, 0).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
    }
    assert!(run.tick(&mut backend).is_err());
    assert_eq!(run.machine().pending_system(), Some((10, 0x128)));
    assert_eq!(run.machine().state().integer(2198), None);
    assert!(!run.auto_resume_skipped());
    fs::write(
        root.0.join("save/system.dat"),
        auto_resume_save(0x1780_0016),
    )
    .unwrap();
    run.tick(&mut backend).unwrap();
    assert_eq!(run.machine().state().integer(2198), Some(0));
    assert!(run.auto_resume_skipped());
}

/// Pushes `args` so that `args[0]` lies at SP-4, then issues `op`.
fn system_call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for value in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}

#[test]
fn read_skip_cancels_script_skip_at_unread_output_and_keeps_flags_on_unknown_bits() {
    for (name, read) in [
        ("read-skip-unread", Some(false)),
        ("read-skip-read", Some(true)),
        ("read-skip-unknown", None),
    ] {
        let mut code = vec![];
        if let Some(read) = read {
            system_call(&mut code, 0x2091, &[123, 1, u32::from(read)]);
        }
        system_call(&mut code, 0x215d, &[0, 0, 0, 0, 100, 100, 1280, 720]);
        system_call(&mut code, 0x22eb, &[1, 1]);
        system_call(&mut code, 0x2152, &[0, 123, 0]);
        code.extend(0x27ffu16.to_le_bytes());
        let root = Root::new(name, &code, b"A\0");
        let script = root.script();
        let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        run.set_record_text(true);
        let mut backend = RecordingBackend::default();
        let mut reached = false;
        for _ in 0..100 {
            match run.tick(&mut backend) {
                Ok(StartupEvent::Progress) => {}
                Ok(StartupEvent::Yield) => {
                    assert_eq!(run.message_flags().skip(), read.unwrap());
                    assert_eq!(
                        run.messages().text(0).unwrap().glyphs[0].active,
                        read == Some(false)
                    );
                    reached = true;
                    break;
                }
                Err(e) => {
                    assert_eq!(read, None);
                    assert!(e.to_string().contains("read-skip bit"));
                    assert!(run.message_flags().skip());
                    assert!(run.messages().text(0).unwrap().glyphs.is_empty());
                    let pc = run.machine().pc();
                    assert!(run.tick(&mut backend).is_err());
                    assert_eq!(run.machine().pc(), pc);
                    assert!(run.message_flags().skip());
                    reached = true;
                    break;
                }
                event => panic!("{event:?}"),
            }
        }
        assert!(reached);
    }
}

#[test]
fn scenario_snapshot_failure_keeps_the_command_vm_and_image_unchanged() {
    // An oversized glyph stays outside the supported reconstruction range.
    let mut code = vec![];
    for (op, words) in [
        (0x215du16, vec![720u32, 1280, 100, 100, 0, 0, 7, 0]),
        (0x2143, vec![129, 0]),
        (0x2152, vec![0, u32::MAX, 0]),
    ] {
        for word in words {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(word.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    }
    let snapshot_pc = code.len();
    code.extend(0x22b2u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("snapshot-failure", &code, b"A ");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_record_text(true);
    let mut backend = RecordingBackend::default();
    let error = loop {
        match run.tick(&mut backend) {
            Ok(StartupEvent::Progress) => {}
            Ok(StartupEvent::Yield) => run.advance_frame(400).unwrap(),
            Ok(event) => panic!("unexpected {event:?}"),
            Err(error) => break error,
        }
    };
    assert!(error.to_string().contains("size"), "{error}");
    for _ in 0..2 {
        assert_eq!(run.machine().pending_system(), Some((snapshot_pc, 0x2b2)));
        assert_eq!(run.machine().pc(), snapshot_pc);
        assert_eq!(run.machine().state().bit(0), None);
        assert_eq!(*run.save_image(), SaveImage::Absent);
        assert!(run.tick(&mut backend).is_err());
    }
}

#[test]
fn scenario_snapshot_writes_a_slot_that_loads_back_with_backups() {
    use cmvs_rs::{save::Csv2Save, snapshot::EngineSnapshot};
    // A load sets the window title from the base title 0x2160 gives.
    let mut code = string_call(0x2160);
    system_call(&mut code, 0x201d, &[0x12345678]);
    system_call(&mut code, 0x2128, &[]);
    system_call(&mut code, 0x2092, &[5, 1, 42]);
    commands::command(&mut code, 0x2020, &[20]);
    commands::command(&mut code, 0x22cb, &[0x112233]);
    commands::command(&mut code, 0x2304, &[]);
    commands::command(&mut code, 0x2000, &[]);
    commands::command(&mut code, 0x203c, &[20, u32::MAX]);
    commands::command(&mut code, 0x215d, &[0, 7, 0, 0, 100, 100, 1280, 720]);
    commands::command(&mut code, 0x2152, &[0, u32::MAX, 0]);
    let view_words = [1.25f32, -2.0, 3.0, 17.5, -30.0, 45.0, 60.0].map(f32::to_bits);
    commands::command(&mut code, 0x2260, &view_words[..3]);
    for (op, word) in (0x2261..=0x2264).zip(&view_words[3..]) {
        commands::command(&mut code, op, &[*word]);
    }
    let snapshot_pc = code.len();
    code.extend(0x22b2u16.to_le_bytes());
    commands::command(&mut code, 0x2260, &[0; 3]);
    system_call(&mut code, 0x2092, &[5, 1, 7]);
    system_call(&mut code, 0x22b6, &[3]);
    let end = code.len();
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("snapshot-slots", &code, b"Synthetic ");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cmvs-runner-slots-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    run.set_record_text(true);
    let mut backend = RecordingBackend::default();
    let run_to_end = |run: &mut Startup<'_>, backend: &mut RecordingBackend| {
        let mut captured = 0;
        loop {
            match run.tick(backend).unwrap() {
                StartupEvent::Progress => {}
                StartupEvent::Yield => run.advance_frame(400).unwrap(),
                StartupEvent::SnapshotCaptured => captured += 1,
                StartupEvent::Unhandled { pc, command } => {
                    assert_eq!((pc, command), (end, 0x27ff));
                    return captured;
                }
                event => panic!("unexpected {event:?}"),
            }
        }
    };
    assert_eq!(run_to_end(&mut run, &mut backend), 1);
    assert_eq!(run.machine().state().integer(5), Some(7));
    assert_eq!(run.machine().state().bit(0), Some(false));
    let slot = directory.join("save003.dat");
    let first = fs::read(&slot).unwrap();
    let save = Csv2Save::decode(&first).unwrap();
    assert_eq!(&first[16..36], b"2000-01-01 00:00:00 ");
    let snapshot = EngineSnapshot::decode(&save.snapshot).unwrap();
    assert_eq!(
        (snapshot.active_slot, snapshot.pc, snapshot.repeat_count),
        (0, snapshot_pc as u32 + 2, 0)
    );
    assert_eq!(snapshot.variables.integers()[5], 42);
    assert_eq!(snapshot.variables.packed_bits()[0] & 1, 1);
    assert_eq!(snapshot.texts[7].as_ref().unwrap().glyphs.len(), 9);
    let recorded_view: Vec<_> = snapshot.cameras[2][6..]
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    assert_eq!(recorded_view, view_words);
    let live = run.scene().object(20).unwrap().image.as_ref().unwrap();
    assert_eq!(&live.pixels.pixels()[..4], &[0x11, 0x22, 0x33, 255]);
    assert!(!live.alpha && !live.surface && live.name.is_empty());
    let saved_image = snapshot.objects[&20].image.as_ref().unwrap();
    assert!(saved_image.name.is_empty() && saved_image.surface.is_none());
    assert!(directory.join("system.dat").exists());
    assert!(!directory.join("save998.dat").exists());
    assert!(!directory.join("save999.dat").exists());

    // Loading slot 3 first keeps the held image as slot 998, then resumes after 0x22b2
    // with the captured locals, bit 0 included.
    run.load_slot(3, &mut backend).unwrap();
    assert_eq!(
        backend.requests().last(),
        Some(&PlatformRequest::Title(b"Synthetic\x81\x40".to_vec()))
    );
    assert!(directory.join("save998.dat").exists());
    assert_eq!(run.machine().pc(), snapshot_pc + 2);
    assert_eq!(run.machine().state().integer(5), Some(42));
    assert_eq!(run.machine().state().bit(0), Some(true));
    assert_eq!(run.camera().view(0).unwrap().words, view_words);
    let restored = run.scene().object(20).unwrap().image.as_ref().unwrap();
    assert!(restored.alpha && !restored.surface && restored.name.is_empty());
    assert!(
        restored
            .pixels
            .pixels()
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255])
    );
    let glyphs = &run.messages().text(7).unwrap().glyphs;
    assert_eq!(glyphs.len(), 9);
    assert!(
        glyphs
            .iter()
            .all(|g| g.shown && !g.active && g.opacity == 255)
    );
    // Saving over slot 3 again moves the previous version to slot 999.
    assert_eq!(run_to_end(&mut run, &mut backend), 0);
    assert_eq!(fs::read(directory.join("save999.dat")).unwrap(), first);
    assert_eq!(fs::read(&slot).unwrap(), first);
    assert!(run.load_slot(4, &mut backend).is_err());
    assert!(run.save_slot(1000).is_err());
    // 0x6a0 is an allocation descriptor, not saved overlay pixels. The native
    // loader ignores its dimensions; the software backend allocates no D3D target.
    let mut with_targets = snapshot.clone();
    let mut target = 0u16.to_le_bytes().to_vec();
    target.extend(2048u32.to_le_bytes());
    target.extend(1024u32.to_le_bytes());
    for index in 0..2 {
        with_targets
            .raw_objects
            .insert((0x6a0, index), target.clone());
    }
    let header: &[u8; 600] = first[..600].try_into().unwrap();
    let changed = Csv2Save::encode(
        header,
        &with_targets.encode().unwrap(),
        &save.auxiliary,
        &save.script_image,
    )
    .unwrap();
    fs::write(directory.join("save004.dat"), changed).unwrap();
    run.load_slot(4, &mut backend).unwrap();
    assert_eq!(run.messages().text(7).unwrap().glyphs.len(), 9);
    // Unknown target versions fail before installing any VM/visual state.
    with_targets.raw_objects.get_mut(&(0x6a0, 1)).unwrap()[0] = 1;
    let changed = Csv2Save::encode(
        header,
        &with_targets.encode().unwrap(),
        &save.auxiliary,
        &save.script_image,
    )
    .unwrap();
    fs::write(directory.join("save004.dat"), changed).unwrap();
    let before = run.inspect_snapshot().unwrap().encode().unwrap();
    assert!(run.load_slot(4, &mut backend).is_err());
    assert_eq!(run.inspect_snapshot().unwrap().encode().unwrap(), before);
    drop(run);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn gameplay_saves_need_an_independent_user_directory() {
    let mut code = vec![];
    code.extend(0x22b2u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("snapshot-no-directory", &code, b"");
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    // Nothing to write before the first snapshot, as in the original.
    assert!(!run.save_slot(0).unwrap());
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        StartupEvent::SnapshotCaptured
    );
    let error = run.save_slot(0).unwrap_err();
    assert!(error.to_string().contains("--user-data-dir"), "{error}");
    assert!(!root.0.join("save/save000.dat").exists());
}

/// 0x2089 on, callback 6 registered at a body that sets integer 42 and returns; a yield
/// ends the first run. `block` adds 0x208c(1) before the yield.
fn dispatch_code(block: bool) -> (Vec<u8>, usize) {
    let mut code = vec![];
    system_call(&mut code, 0x2089, &[1]);
    let target = 10 + 42 + if block { 10 } else { 0 } + 2 + 26 + 2;
    system_call(&mut code, 0x2088, &[6, target as u32, 0, 0, 0]);
    if block {
        system_call(&mut code, 0x208c, &[1]);
    }
    code.extend(0x2000u16.to_le_bytes());
    system_call(&mut code, 0x2092, &[100, 1, 5]);
    let end = code.len();
    code.extend(0x27ffu16.to_le_bytes());
    assert_eq!(code.len(), target);
    system_call(&mut code, 0x2092, &[42, 1, 7]);
    code.extend(0x0414u16.to_le_bytes());
    code.extend(0u16.to_le_bytes());
    (code, end)
}

#[test]
fn automatic_dispatch_runs_due_callbacks_at_the_next_run_entry_and_returns() {
    for block in [false, true] {
        let (code, end) = dispatch_code(block);
        let root = Root::new(&format!("dispatch-{block}"), &code, b"");
        let script = root.script();
        let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        let mut backend = RecordingBackend::default();
        let mut yields = 0;
        loop {
            match run.tick(&mut backend).unwrap() {
                StartupEvent::Progress => {}
                StartupEvent::Yield => {
                    yields += 1;
                    // The callback has not run inside the run that registered it.
                    assert_eq!(run.machine().state().integer(42), None);
                }
                StartupEvent::Unhandled { pc, command } => {
                    assert_eq!((pc, command), (end, 0x27ff));
                    break;
                }
                event => panic!("unexpected {event:?}"),
            }
        }
        assert_eq!(yields, 1);
        // 0x208c(1) suppresses the dispatch; otherwise the callback ran first and
        // returned through its frame to the instruction after the yield.
        let expected = (!block).then_some(7);
        assert_eq!(run.machine().state().integer(42), expected);
        assert_eq!(run.machine().state().integer(100), Some(5));
        assert_eq!(run.machine().stack().len_bytes(), 0);
        assert_eq!(run.machine().current_slot(), 0);
    }
}

#[test]
fn snapshots_inside_a_dispatched_callback_load_back_and_return_through_the_frame() {
    // Callback 6 disables itself, takes the scenario snapshot and returns.
    let build = |callback: u32| {
        let mut code = string_call(0x2160);
        system_call(&mut code, 0x201d, &[0x12345678]);
        system_call(&mut code, 0x2128, &[]);
        system_call(&mut code, 0x2089, &[1]);
        system_call(&mut code, 0x2088, &[6, callback, 0, 0, 0]);
        code.extend(0x2000u16.to_le_bytes());
        let end = code.len();
        code.extend(0x27ffu16.to_le_bytes());
        let start = code.len();
        system_call(&mut code, 0x208b, &[6]);
        code.extend(0x22b2u16.to_le_bytes());
        code.extend(0x0414u16.to_le_bytes());
        code.extend(0u16.to_le_bytes());
        (code, start, end)
    };
    let (_, start, _) = build(0);
    let (code, start_again, end) = build(start as u32);
    assert_eq!(start, start_again);
    let root = Root::new("callback-snapshot", &code, b"Synthetic ");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cmvs-runner-callback-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    let run_to_end = |run: &mut Startup<'_>, backend: &mut RecordingBackend| {
        let mut captured = 0;
        loop {
            match run.tick(backend).unwrap() {
                StartupEvent::Progress | StartupEvent::Yield => {}
                StartupEvent::SnapshotCaptured => {
                    captured += 1;
                    // Captured inside the callback, above its return frame.
                    assert_eq!(run.machine().stack().len_bytes(), 12);
                }
                StartupEvent::Unhandled { pc, command } => {
                    assert_eq!((pc, command), (end, 0x27ff));
                    return captured;
                }
                event => panic!("unexpected {event:?}"),
            }
        }
    };
    assert_eq!(run_to_end(&mut run, &mut backend), 1);
    assert!(run.save_slot(0).unwrap());
    run.load_slot(0, &mut backend).unwrap();
    // The restored stack keeps the dispatch frames; the callback returns through them.
    assert_eq!(run_to_end(&mut run, &mut backend), 0);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    drop(run);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn an_enabled_save_callback_stops_the_slot_write_but_not_the_snapshot() {
    let mut code = vec![];
    system_call(&mut code, 0x201d, &[0x12345678]);
    system_call(&mut code, 0x2128, &[]);
    system_call(&mut code, 0x2088, &[30, 0, 0, 0, 0]);
    code.extend(0x22b2u16.to_le_bytes());
    let write = code.len() + 10 - 2;
    system_call(&mut code, 0x22b6, &[0]);
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("save-callback", &code, b"");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cmvs-runner-save-callback-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    let error = loop {
        match run.tick(&mut backend) {
            Ok(StartupEvent::Progress | StartupEvent::SnapshotCaptured) => {}
            Ok(event) => panic!("unexpected {event:?}"),
            Err(error) => break error,
        }
    };
    assert!(error.to_string().contains("callback 30"), "{error}");
    assert_eq!(run.machine().pending_system(), Some((write, 0x2b6)));
    assert!(!directory.join("save000.dat").exists());
    drop(run);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn button_groups_are_saved_and_come_back_empty_on_object_zero() {
    use cmvs_rs::snapshot::EngineSnapshot;
    let mut code = string_call(0x2160);
    system_call(&mut code, 0x201d, &[0x12345678]);
    system_call(&mut code, 0x2128, &[]);
    system_call(&mut code, 0x2020, &[10]);
    system_call(&mut code, 0x2210, &[2, 10]);
    system_call(&mut code, 0x2212, &[2, 7]);
    system_call(&mut code, 0x2213, &[2, 7, 1, 2, 3, 4]);
    code.extend(0x22b2u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("button-records", &code, b"Synthetic\0");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cmvs-runner-buttons-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        match run.tick(&mut backend).unwrap() {
            StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
            StartupEvent::Unhandled {
                command: 0x27ff, ..
            } => break,
            event => panic!("unexpected {event:?}"),
        }
    }
    let SaveImage::Captured(held) = run.save_image() else {
        panic!("no snapshot");
    };
    let snapshot = EngineSnapshot::decode(&held.snapshot).unwrap();
    let record = &snapshot.raw_objects[&(0x8c0, 2)];
    // Header (object 0, version 2, scale), +8, count, then one 104-byte node.
    assert_eq!(record.len(), 20 + 104);
    assert_eq!(record[..4], [0, 0, 2, 0]);
    assert_eq!(record[16..20], 1u32.to_le_bytes());
    assert_eq!(record[24..28], 7u32.to_le_bytes());
    assert!(run.save_slot(0).unwrap());
    run.load_slot(0, &mut backend).unwrap();
    // The original reader rebuilds the group empty, bound to the record's object 0.
    let group = run.buttons().group(2).unwrap();
    assert!(group.buttons.is_empty());
    assert_eq!(group.object, 0);
    drop(run);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn font_dialog_sets_the_global_face_refreshes_default_texts_and_persists() {
    use cmvs_rs::settings::{DEFAULT_FONT_FACE, DisplayPreferences};
    let mut code = vec![];
    commands::command(&mut code, 0x2100, &[1, 0, 0]);
    for _ in 0..3 {
        commands::command(&mut code, 0x213d, &[]);
    }
    let root = Root::new("font-dialog", &code, b"\0");
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("font-dialog-{}", std::process::id()));
    let script = root.script();
    let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    backend.answer_font_dialog(Some(b"Face A".to_vec()));
    backend.answer_font_dialog(None);
    while backend.font_dialogs().len() < 2 {
        assert_ne!(run.tick(&mut backend).unwrap(), StartupEvent::Inactive);
    }
    assert_eq!(backend.font_dialogs(), [DEFAULT_FONT_FACE, b"Face A"]);
    assert_eq!(run.settings().font_face(), b"Face A");
    assert_eq!(run.messages().default_face().as_ref(), b"Face A");
    assert_eq!(run.messages().text(1).unwrap().face.as_ref(), b"Face A");
    // Without an answer the third dialog fails and its command stays pending.
    let mut failed = false;
    for _ in 0..20 {
        if run.tick(&mut backend).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(run.machine().pending_system().is_some());
    assert_eq!(run.settings().font_face(), b"Face A");
    assert!(run.save_preferences(DisplayPreferences::default()).unwrap());
    let saved = fs::read(directory.join("cmvs.cfg")).unwrap();
    assert!(saved.windows(13).any(|w| w == b"FONT=Face A\r\n"));
    drop(run);
    let mut resumed = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
    resumed.set_user_data_directory(&directory).unwrap();
    assert_eq!(resumed.settings().font_face(), b"Face A");
    drop(resumed);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn chosen_faces_are_validated_before_the_command_completes() {
    let mut code = vec![];
    commands::command(&mut code, 0x213d, &[]);
    let root = Root::new("font-dialog-invalid", &code, b"");
    let script = root.script();
    for face in [Vec::new(), vec![b'x'; 32], b"a\0b".to_vec()] {
        let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        let mut backend = RecordingBackend::default();
        backend.answer_font_dialog(Some(face));
        let mut failed = false;
        for _ in 0..20 {
            if run.tick(&mut backend).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(run.machine().pending_system().is_some());
        assert_eq!(
            run.settings().font_face(),
            cmvs_rs::settings::DEFAULT_FONT_FACE
        );
    }
}

#[test]
fn first_system_save_without_template_writes_reloads_and_survives_restart() {
    use cmvs_rs::{settings::DisplayPreferences, system_save::Css1Save};
    for explicit_write in [false, true] {
        let mut code = vec![];
        commands::command(&mut code, 0x201d, &[0x12345678]);
        commands::command(&mut code, 0x2091, &[2048, 260096, 0]);
        commands::command(&mut code, 0x2092, &[2048, 2048, 0]);
        commands::command(&mut code, 0x2093, &[1024, 1024, 0]);
        commands::command(&mut code, 0x2094, &[64, 64, 0]);
        commands::command(&mut code, 0x2128, &[]);
        let after_load = code.len();
        commands::command(&mut code, 0x2092, &[2400, 1, 96]);
        if explicit_write {
            commands::command(&mut code, 0x2129, &[]);
        }
        commands::command(&mut code, 0x27ff, &[]);
        let root = Root::new(&format!("first-css1-{explicit_write}"), &code, b"\0");
        // First launch of an untouched game may have no save directory at all.
        fs::remove_dir(root.0.join("save")).unwrap();
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "first-css1-{explicit_write}-{}",
            std::process::id()
        ));
        let script = root.script();
        let mut run = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        run.set_user_data_directory(&directory).unwrap();
        // An early close can write INI, but must not manufacture a CSS1.
        run.save_preferences(DisplayPreferences::default()).unwrap();
        assert!(!directory.join("system.dat").exists());
        let mut backend = RecordingBackend::default();
        loop {
            if let StartupEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
                assert_eq!(command, 0x27ff);
                break;
            }
        }
        assert_eq!(directory.join("system.dat").exists(), explicit_write);
        run.save_preferences(DisplayPreferences::default()).unwrap();
        let raw = fs::read(directory.join("system.dat")).unwrap();
        assert_eq!(&raw[8..15], b"cmvs-rs");
        let saved = Css1Save::decode(&raw).unwrap();
        assert_eq!(saved.layout().game_id, 0x12345678);
        assert_eq!(
            saved
                .applied(run.machine().state(), Some(0x12345678))
                .unwrap()
                .integer(2400),
            Some(96)
        );
        drop(run);
        let mut resumed = Startup::new(&script, &root.0, &root.game(), 0).unwrap();
        resumed.set_user_data_directory(&directory).unwrap();
        while resumed.machine().pc() != after_load {
            resumed.tick(&mut backend).unwrap();
        }
        // Check before the script writes 96 again: this value came from CSS1.
        assert_eq!(resumed.machine().state().integer(2400), Some(96));
        assert!(!root.0.join("save/system.dat").exists());
        assert!(!root.0.join("save").exists());
        drop(resumed);
        fs::remove_dir_all(directory).unwrap();
    }
}

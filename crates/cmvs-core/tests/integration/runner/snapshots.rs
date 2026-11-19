use super::*;

#[test]
fn restoring_a_slot_discards_the_movie_overlay_and_its_pending_keyboard() {
    use cmvs_core::host::{
        input::{CodeInput, KeyBindings},
        platform::{TextDialogRequest, TextDialogResult},
    };
    #[derive(Default)]
    struct Keyboard {
        pending: bool,
    }
    impl PlatformBackend for Keyboard {
        fn poll_text_dialog(
            &mut self,
            _: &TextDialogRequest,
        ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
            self.pending = true;
            Ok(TextDialogResult::Pending)
        }
        fn cancel_text_dialog(&mut self) {
            self.pending = false;
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
    }
    let mut code = string_call(0x2160);
    commands::command(&mut code, 0x2128, &[]);
    commands::command(&mut code, 0x22b2, &[]);
    commands::command(&mut code, 0x2015, &[7]);
    commands::command(&mut code, 0x20c0, &[0]);
    commands::command(&mut code, 0x213c, &[7, 7, 7, 32, 0x8000_0003, 0]);
    commands::command(&mut code, 0x2000, &[]);
    let root = Root::new("restore-comment", &code, b"op.cmv\0\0");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0)).unwrap();
    fs::write(root.0.join("op.cmv"), crate::support::cmv9::cmv6(3, false)).unwrap();
    let game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20071117"))
        .unwrap();
    let mut run = Runtime::new(root.script(), &root.0, &game, 0).unwrap();
    let user = crate::support::temp_dir::TempPath::new("restore-comment-save");
    run.set_user_data_directory(&user).unwrap();
    run.enable_in_game_dialogs(None);
    run.set_decode_cmv_frames(true);
    let mut backend = Keyboard::default();
    while run.tick(&mut backend).unwrap() == RuntimeEvent::Progress {}
    assert!(run.save_slot(3).unwrap());
    while run.tick(&mut backend).unwrap() == RuntimeEvent::Progress {}
    assert!(run.movies().player_image().is_some());
    assert!(run.movie_frame().is_none());
    let [w, h] = run.screen_size();
    let accent = (((h - 234) / 2 * w + (w - 400) / 2) * 4) as usize;
    let frame = run.prepare_scene_frame().unwrap().render_cpu().unwrap();
    assert_eq!(&frame.pixels()[accent..accent + 4], &[127, 187, 255, 255]);
    // The gray movie is still visible above the panel, under the dimming layer.
    assert!(frame.pixels()[w as usize * 2] > 30);
    let mut keys = [false; 128];
    keys[22] = true;
    CodeInput::default().update(&KeyBindings::default(), &keys, run.input_mut());
    run.tick(&mut backend).unwrap();
    run.tick(&mut backend).unwrap();
    assert!(backend.pending);
    assert!(run.load_slot(4, &mut backend).is_err());
    assert!(run.keyboard_comment_rect().is_some() && backend.pending);
    run.load_slot(3, &mut backend).unwrap();
    assert!(run.keyboard_comment_rect().is_none() && !backend.pending);
    assert!(run.movies().player_image().is_none());
    assert_ne!(
        &run.render_frame().unwrap().pixels()[accent..accent + 4],
        &[127, 187, 255, 255]
    );
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
                let user = crate::support::temp_dir::TempPath::in_target_tmp(&name);
                let script = root.script();
                let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
                    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
            let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
            let mut backend = RecordingBackend::default();
            for _ in 0..2 {
                run.tick(&mut backend).unwrap();
            }
            if args == [0, 1] {
                assert!(run.tick(&mut backend).is_err());
                assert!(run.machine().pending_system().is_some());
                assert_eq!(run.machine().stack().len_bytes(), 8);
            } else {
                assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
    let user = crate::support::temp_dir::TempPath::in_target_tmp("slot-edit-menu");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
    let cmvs_core::runtime::save_menu::SlotStatus::Recognized(header) = &menu.slots[0] else {
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
    let user = crate::support::temp_dir::TempPath::in_target_tmp("slot-edit-retry");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(fs::read(user.join("save000.dat")).unwrap(), b"second");
    assert_eq!(fs::read(user.join("save001.dat")).unwrap(), b"first");
    assert!(!user.join("savetemp.dat").exists());
    drop(run);
    fs::remove_dir_all(user).unwrap();
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
    let root = Root::new("snapshot-failure", &code, b"A\0");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_record_text(true);
    let mut backend = RecordingBackend::default();
    let error = loop {
        match run.tick(&mut backend) {
            Ok(RuntimeEvent::Progress) => {}
            Ok(RuntimeEvent::Yield) => run.advance_frame(400).unwrap(),
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
    snapshot_slots_roundtrip(None);
}

#[test]
fn slot_metadata_updates_only_two_header_words_in_the_user_directory() {
    for (case, slot, signature, length, expected) in [
        ("v2", 3, b"CSV2", 700, true),
        ("v9", 3, b"CSV9", 600, true),
        ("bad", 3, b"BAD2", 700, false),
        ("missing", 3, b"CSV2", 0, false),
        ("range", 1000, b"CSV2", 700, false),
    ] {
        let mut code = vec![];
        commands::command(&mut code, 0x22be, &[slot, 0xdeadbeef, 0x87654321]);
        let end = code.len();
        commands::command(&mut code, 0x2000, &[]);
        let root = Root::new(&format!("slot-metadata-{case}"), &code, b"");
        let source = root.0.join("save/save003.dat");
        fs::write(&source, b"original source").unwrap();
        let script = root.script();
        let game = root.game();
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        let user = root.0.with_extension("user");
        run.set_user_data_directory(&user).unwrap();
        let mut bytes: Vec<_> = (0..length).map(|n| n as u8).collect();
        if length > 0 {
            bytes[..4].copy_from_slice(signature);
            fs::write(user.join("save003.dat"), &bytes).unwrap();
        }
        let mut backend = RecordingBackend::default();
        while run.machine().pc() != end {
            run.tick(&mut backend).unwrap();
        }
        assert_eq!(
            run.machine().state().system_integer(0),
            Some(u32::from(expected))
        );
        if expected {
            bytes[532..536].copy_from_slice(&0xdeadbeefu32.to_le_bytes());
            bytes[536..540].copy_from_slice(&0x87654321u32.to_le_bytes());
        }
        if length > 0 {
            assert_eq!(fs::read(user.join("save003.dat")).unwrap(), bytes);
        } else {
            assert!(!user.join("save003.dat").exists());
        }
        assert_eq!(fs::read(source).unwrap(), b"original source");
        assert!(!user.join("save999.dat").exists());
        drop(run);
        fs::remove_dir_all(user).unwrap();
    }
}

#[test]
fn slot_metadata_failures_preserve_file_and_pending_command_for_retry() {
    let mut code = vec![];
    commands::command(&mut code, 0x22be, &[3, 1, 2]);
    let root = Root::new("slot-metadata-retry", &code, b"");
    let script = root.script();
    let game = root.game();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..3 {
        run.tick(&mut backend).unwrap();
    }
    assert!(
        run.tick(&mut backend)
            .unwrap_err()
            .to_string()
            .contains("--user-data-dir")
    );
    drop(run);
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let user = root.0.with_extension("user");
    run.set_user_data_directory(&user).unwrap();
    let mut bytes = vec![0; 700];
    bytes[..4].copy_from_slice(b"CSV2");
    fs::write(user.join("save003.dat"), b"CSV2short").unwrap();
    for _ in 0..3 {
        run.tick(&mut backend).unwrap();
    }
    let pc = run.machine().pc();
    assert!(
        run.tick(&mut backend)
            .unwrap_err()
            .to_string()
            .contains("600-byte")
    );
    assert_eq!(fs::read(user.join("save003.dat")).unwrap(), b"CSV2short");
    fs::write(user.join("save003.dat"), &bytes).unwrap();
    fs::create_dir(user.join(".save003.dat.tmp")).unwrap();
    assert!(run.tick(&mut backend).is_err());
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.machine().stack().len_bytes(), 12);
    assert_eq!(run.machine().pending_system(), Some((pc, 0x2be)));
    assert_eq!(fs::read(user.join("save003.dat")).unwrap(), bytes);
    fs::remove_dir(user.join(".save003.dat.tmp")).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    bytes[532..536].copy_from_slice(&1u32.to_le_bytes());
    bytes[536..540].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(fs::read(user.join("save003.dat")).unwrap(), bytes);
    assert_eq!(run.machine().state().system_integer(0), Some(1));
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

#[test]
fn ps2_canvas_queries_pointer_and_v2_saves_keep_four_by_three_coordinates() {
    use cmvs_core::{
        format::save::Csv2Save, format::snapshot::EngineSnapshot, format::snapshot::SnapshotLayout,
    };
    let mut code = string_call(0x2160);
    commands::command(&mut code, 0x201d, &[0x12345678]);
    commands::command(&mut code, 0x2128, &[]);
    commands::command(&mut code, 0x2126, &[]);
    commands::command(&mut code, 0x2140, &[0, 0, 0, 0, 1024, 768]);
    commands::command(&mut code, 0x215f, &[0, 1000, 740, 24, 28]);
    let snapshot_pc = code.len();
    commands::command(&mut code, 0x22b2, &[]);
    commands::command(&mut code, 0x2000, &[]);
    let root = Root::new("ps2-canvas-save", &code, b"Synthetic\0");
    fs::rename(
        root.0.join("data/pack/start.ps3"),
        root.0.join("data/pack/start.ps2"),
    )
    .unwrap();
    fs::write(
        root.0.join("cmvs.cfg"),
        b"[WINDOW_RESOLVING]\nBASE=1024,768\n",
    )
    .unwrap();
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20110525"))
        .unwrap();
    let script = load_start(&root.0, &game.files).unwrap();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let user = root.0.with_extension("user");
    run.set_user_data_directory(&user).unwrap();
    run.input_mut().set_pointer(1010, 755);
    let mut backend = RecordingBackend::default();
    backend.set_screen_size(cmvs_core::render::viewport::ScreenSize::new([1024, 768]).unwrap());
    while run.machine().pc() != snapshot_pc {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    assert_eq!(run.machine().state().system_integer(0), Some(1));
    assert_eq!(run.machine().state().system_integer(1), Some(1024));
    assert_eq!(run.machine().state().system_integer(2), Some(768));
    assert_eq!(run.messages().text(0).unwrap().logical_size, [1024, 768]);
    let frame = run.render_frame().unwrap();
    assert_eq!([frame.width(), frame.height()], [1024, 768]);
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::SnapshotCaptured
    );
    assert!(run.save_slot(3).unwrap());
    let bytes = fs::read(user.join("save003.dat")).unwrap();
    let save = Csv2Save::decode(&bytes).unwrap();
    let snapshot =
        EngineSnapshot::decode_with_layout(&save.snapshot, SnapshotLayout::TextV2).unwrap();
    assert_eq!(
        snapshot.texts[0].as_ref().unwrap().logical_size,
        [1024, 768]
    );
    run.load_slot(3, &mut backend).unwrap();
    assert_eq!(run.machine().current_name(), b"start.ps2");
    assert_eq!(run.scene().screen_size(), [1024, 768]);
    assert_eq!(run.messages().screen_size(), [1024, 768]);
    assert_eq!(run.machine().pc(), snapshot_pc + 2);
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

#[test]
fn old_snapshot_roundtrip_uses_build_layout_and_zero_css1_identity() {
    use cmvs_core::{
        format::save::Csv2Save, format::snapshot::EngineSnapshot, format::snapshot::SnapshotLayout,
        format::system_save::Css1Save,
    };
    for (profile, layout) in [
        ("cmvs-20070119", SnapshotLayout::TextV1SceneLegacy),
        ("cmvs-20071117", SnapshotLayout::TextV1SceneV1),
        ("cmvs-20080719", SnapshotLayout::TextV1SceneV2),
    ] {
        let mut code = string_call(0x2160);
        commands::command(&mut code, 0x2128, &[]);
        commands::command(&mut code, 0x2140, &[0, 0, 0, 0, 800, 600]);
        commands::command(&mut code, 0x2020, &[20]);
        commands::command(&mut code, 0x2040, &[20]);
        commands::command(&mut code, 0x2073, &[20, u32::MAX, 2f32.to_bits()]);
        if profile == "cmvs-20080719" {
            commands::command(&mut code, 0x2225, &[0x102, 24, 80, u32::MAX]);
        }
        let pc = code.len();
        commands::command(&mut code, 0x22b2, &[]);
        commands::command(&mut code, 0x2073, &[20, u32::MAX, 9f32.to_bits()]);
        let end = code.len();
        commands::command(&mut code, 0x2000, &[]);
        let root = Root::new(profile, &code, b"Synthetic\0");
        fs::rename(
            root.0.join("data/pack/start.ps3"),
            root.0.join("data/pack/start.ps2"),
        )
        .unwrap();
        fs::write(root.0.join("save/system.dat"), auto_resume_save(0)).unwrap();
        let game = Registry::builtin()
            .identify_game(&root.0, Some(profile))
            .unwrap();
        let script = load_start(&root.0, &game.files).unwrap();
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        let user = root.0.with_extension("user");
        run.set_user_data_directory(&user).unwrap();
        assert_eq!(run.screen_size(), [800, 600]);
        let frame = run.render_frame().unwrap();
        assert_eq!([frame.width(), frame.height()], [800, 600]);

        let mut backend = RecordingBackend::default();
        backend.set_screen_size(cmvs_core::render::viewport::ScreenSize::new([800, 600]).unwrap());
        while run.machine().pc() != pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        if profile == "cmvs-20080719" {
            run.advance_frame(100).unwrap();
            assert_eq!(run.scene().quake().unwrap().offset, [12, 0]);
        }
        assert_eq!(
            run.tick(&mut backend).unwrap(),
            RuntimeEvent::SnapshotCaptured
        );
        assert!(run.save_slot(3).unwrap());
        let bytes = fs::read(user.join("save003.dat")).unwrap();
        let save = Csv2Save::decode(&bytes).unwrap();
        let parsed = EngineSnapshot::decode_with_layout(&save.snapshot, layout).unwrap();
        assert!(parsed.record_6e0.is_empty());
        assert_eq!(parsed.texts[0].as_ref().unwrap().logical_size, [0, 0]);
        while run.machine().pc() != end {
            run.tick(&mut backend).unwrap();
        }
        run.load_slot(3, &mut backend).unwrap();
        assert_eq!(run.machine().pc(), pc + 2);
        assert_eq!(run.machine().current_name(), b"start.ps2");
        assert_eq!(
            run.scene()
                .object(20)
                .unwrap()
                .draw
                .as_ref()
                .unwrap()
                .world_position_bits()[1],
            2f32.to_bits()
        );
        assert_eq!(
            run.messages().text(0).unwrap().logical_size,
            run.messages().screen_size().map(|v| v as i32)
        );
        assert_eq!(run.scene().screen_size(), [800, 600]);
        assert!(run.save_slot(4).unwrap());
        let second = Csv2Save::decode(&fs::read(user.join("save004.dat")).unwrap()).unwrap();
        assert_eq!(second.snapshot, save.snapshot);
        assert_eq!(second.script_image, save.script_image);
        assert!(user.join("save998.dat").is_file());
        assert_eq!(
            Css1Save::decode(&fs::read(user.join("system.dat")).unwrap())
                .unwrap()
                .layout()
                .game_id,
            0
        );
        if profile == "cmvs-20080719" {
            assert_eq!(
                (
                    run.scene().quake().unwrap().steps,
                    run.scene().quake().unwrap().elapsed
                ),
                (1, 20)
            );
            run.advance_frame(20).unwrap();
            assert_eq!(run.scene().quake().unwrap().offset, [24, 0]);
        }
        drop(run);
        fs::remove_dir_all(user).unwrap();
    }
}

#[test]
fn aoitori_v3_snapshot_restores_text_scene_locals_and_backup_slots() {
    snapshot_slots_roundtrip(Some("cmvs-20171114"));
}

#[test]
fn load_before_first_audio_command_configures_resources_before_replaying_bgm() {
    use cmvs_core::{
        host::audio::AudioBackend, host::audio::AudioRequest, host::audio::VoiceDuckingSettings,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Audio {
        paths_ready: bool,
        configured: bool,
        played: Arc<AtomicBool>,
    }
    impl AudioBackend for Audio {
        fn configure_voice_ducking(
            &mut self,
            _: VoiceDuckingSettings,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn configure_paths(
            &mut self,
            paths: &std::collections::BTreeMap<(u16, u32), Vec<u8>>,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            assert_eq!(
                paths.get(&(0x2013, 0)).map(Vec::as_slice),
                Some(&b"Synthetic"[..])
            );
            self.paths_ready = true;
            self.configured = false;
            Ok(())
        }
        fn configure_archive_parameters(
            &mut self,
            _: &std::collections::BTreeMap<(u16, u32), cmvs_core::format::cpz::Cpz7Parameters>,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            assert!(self.paths_ready);
            self.configured = true;
            Ok(())
        }
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
            Ok(true)
        }
        fn stop(&mut self, _: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn stop_voice(&mut self, _: u8) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn stop_channel15(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn stop_bgm(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn play_bgm(&mut self, request: &AudioRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
            assert!(
                self.configured,
                "restored audio needs resource paths and archive parameters"
            );
            assert!(
                matches!(request, AudioRequest::PlayBgm { primary, .. } if primary == b"Synthetic")
            );
            self.played.store(true, Ordering::Relaxed);
            Ok(())
        }
    }
    let mut code = vec![];
    commands::command(&mut code, 0x201d, &[0x12345678]);
    commands::command(&mut code, 0x2013, &[0, 0]);
    commands::command(&mut code, 0x2128, &[]);
    commands::command(&mut code, 0x2160, &[0]);
    let audio_pc = code.len();
    commands::command(&mut code, 0x20a0, &[0, 0, 1]);
    commands::command(&mut code, 0x22b2, &[]);
    commands::command(&mut code, 0x2000, &[]);
    let root = Root::new("resume-audio-paths", &code, b"Synthetic\0");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let script = root.script();
    let game = root.game();
    let user = root.0.with_extension("user");
    let mut backend = RecordingBackend::default();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    while run.tick(&mut backend).unwrap() != RuntimeEvent::SnapshotCaptured {}
    assert!(run.save_slot(3).unwrap());
    drop(run);
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    // Register resources, then load without executing any playback command.
    while run.machine().pc() != audio_pc {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    let played = Arc::new(AtomicBool::new(false));
    run.set_audio_backend(Box::new(Audio {
        paths_ready: false,
        configured: false,
        played: played.clone(),
    }))
    .unwrap();
    run.load_slot(3, &mut backend).unwrap();
    assert!(played.load(Ordering::Relaxed));
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

#[test]
fn kunado_v4_snapshot_restores_text_scene_particles_locals_and_backup_slots() {
    snapshot_slots_roundtrip(Some("cmvs-20211006"));
    snapshot_slots_roundtrip(Some("cmvs-20221105"));
}

#[test]
fn kazokubu_v2_snapshot_restores_text_scene_particles_locals_and_backup_slots() {
    snapshot_slots_roundtrip(Some("cmvs-20120309"));
}

fn snapshot_slots_roundtrip(profile: Option<&str>) {
    use cmvs_core::{
        format::save::Csv2Save, format::snapshot::EngineSnapshot, format::snapshot::SnapshotLayout,
    };
    let layout = if profile == Some("cmvs-20120309") {
        SnapshotLayout::TextV2
    } else if matches!(profile, Some("cmvs-20211006" | "cmvs-20221105")) {
        SnapshotLayout::TextV3SceneV4
    } else {
        SnapshotLayout::TextV3
    };
    let collision = profile == Some("cmvs-20171114");
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
    let collision_points =
        [[-1.0f32, 2.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 1.0]].map(|p| p.map(f32::to_bits));
    if collision {
        commands::command(&mut code, 0x2330, &[20]);
        for point in collision_points {
            commands::command(&mut code, 0x2332, &[20, point[0], point[1], point[2]]);
        }
    }
    if profile.is_some() {
        commands::command(&mut code, 0x2011, &[0, 10]);
        commands::command(
            &mut code,
            0x22a5,
            &[
                0,
                0,
                0,
                40.0f32.to_bits(),
                20.0f32.to_bits(),
                0,
                2000.0f32.to_bits(),
                2,
            ],
        );
    }
    if matches!(profile, Some("cmvs-20211006" | "cmvs-20221105")) {
        commands::command(&mut code, 0x2022, &[20, 1023]);
        commands::command(&mut code, 0x2178, &[0, 1023]);
    }
    if profile == Some("cmvs-20120309") {
        commands::command(&mut code, 0x2022, &[20, 511]);
        commands::command(&mut code, 0x2178, &[0, 511]);
    }
    let snapshot_pc = code.len();
    code.extend(0x22b2u16.to_le_bytes());
    if collision {
        commands::command(&mut code, 0x2331, &[20]);
    }
    commands::command(&mut code, 0x2260, &[0; 3]);
    system_call(&mut code, 0x2092, &[5, 1, 7]);
    system_call(&mut code, 0x22b6, &[3]);
    let end = code.len();
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new(
        &format!("snapshot-slots-{}", profile.unwrap_or("synthetic")),
        &code,
        b"Synthetic\0data\\pack\\\0",
    );
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    if profile.is_some() {
        fs::write(root.0.join("data/pack/Synthetic"), b"NONE").unwrap();
    }
    let directory = crate::support::temp_dir::TempPath::in_target_tmp(&format!(
        "runner-slots-{}",
        profile.unwrap_or("synthetic")
    ));
    let script = root.script();
    let game = match profile {
        Some(id) => Registry::builtin()
            .identify_game(&root.0, Some(id))
            .unwrap(),
        None => root.game(),
    };
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    run.set_record_text(true);
    let mut backend = RecordingBackend::default();
    let run_to_end = |run: &mut Runtime, backend: &mut RecordingBackend| {
        let mut captured = 0;
        loop {
            if run.machine().pc() == end {
                return captured;
            }
            match run.tick(backend).unwrap() {
                RuntimeEvent::Progress => {}
                RuntimeEvent::Yield => run.advance_frame(400).unwrap(),
                RuntimeEvent::SnapshotCaptured => captured += 1,
                RuntimeEvent::Unhandled { pc, command } => {
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
    let snapshot = EngineSnapshot::decode_with_layout(&save.snapshot, layout).unwrap();
    if profile == Some("cmvs-20120309") {
        for children in [
            &snapshot.objects[&20].children,
            &snapshot.windows[0].as_ref().unwrap().scene.children,
        ] {
            assert!(children.contains_key(&511));
            assert!(!children.contains_key(&512));
        }
    }
    assert_eq!(
        (snapshot.active_slot, snapshot.pc, snapshot.repeat_count),
        (0, snapshot_pc as u32 + 2, 0)
    );
    if collision {
        assert_eq!(
            snapshot.objects[&20].collision_vertices.as_deref(),
            Some(collision_points.as_slice())
        );
        assert_eq!(run.scene().object(20).unwrap().collision_vertices, None);
    }
    if profile.is_some() {
        let effect =
            cmvs_core::format::snapshot::SavedEffect::decode(&snapshot.raw_objects[&(0x800, 0)])
                .unwrap();
        assert_eq!(effect.image_name(), b"Synthetic");
        assert!(
            matches!(effect, cmvs_core::format::snapshot::SavedEffect::Particles { words, .. } if words[5] == 2)
        );
    }
    if matches!(profile, Some("cmvs-20211006" | "cmvs-20221105")) {
        assert!(snapshot.objects[&20].children.contains_key(&1023));
        assert!(
            snapshot.windows[0]
                .as_ref()
                .unwrap()
                .scene
                .children
                .contains_key(&1023)
        );
    }
    assert_eq!(snapshot.variables.integers()[5], 42);
    assert_eq!(snapshot.variables.packed_bits()[0] & 1, 1);
    assert_eq!(snapshot.texts[7].as_ref().unwrap().glyphs.len(), 9);
    let recorded_view: Vec<_> = snapshot.cameras[2][6..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
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

    // A malformed audio record must be rejected before switching to the saved VM and scene.
    let header: &[u8; 600] = first[..600].try_into().unwrap();
    let mut bad_audio = snapshot.clone();
    bad_audio.sound_channels[..32].fill(b'x');
    let bytes = Csv2Save::encode(
        header,
        &bad_audio.encode_with_layout(layout).unwrap(),
        &save.auxiliary,
        &save.script_image,
    )
    .unwrap();
    fs::write(directory.join("save005.dat"), bytes).unwrap();
    let error = run.load_slot(5, &mut backend).unwrap_err();
    assert_eq!(error.to_string(), "SE name lacks its terminator");
    assert_eq!(run.machine().pc(), end);
    assert_eq!(run.machine().state().integer(5), Some(7));
    let live = run.scene().object(20).unwrap().image.as_ref().unwrap();
    assert_eq!(&live.pixels.pixels()[..4], &[0x11, 0x22, 0x33, 255]);
    assert!(!directory.join("save998.dat").exists());

    // Callback 29 must fit the restored stack and target the restored script.
    let before = run.inspect_snapshot().unwrap();
    let requests = backend.requests().len();
    let mut callback = snapshot.clone();
    let target_offset = 4 + 28 * 29 + 8;
    for (target, sp, expected) in [
        (
            snapshot_pc + 1,
            0,
            "callback target is not an instruction boundary",
        ),
        (end, snapshot.stack.len() - 8, "parameter stack limit"),
    ] {
        callback.callbacks[target_offset..target_offset + 4]
            .copy_from_slice(&(target as u32).to_le_bytes());
        callback.stack_pointer = sp as u32;
        let bytes = Csv2Save::encode(
            header,
            &callback.encode_with_layout(layout).unwrap(),
            &save.auxiliary,
            &save.script_image,
        )
        .unwrap();
        fs::write(directory.join("save005.dat"), bytes).unwrap();
        let error = run.load_slot(5, &mut backend).unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert_eq!(run.inspect_snapshot().unwrap(), before);
        assert_eq!(backend.requests().len(), requests);
        assert!(!directory.join("save998.dat").exists());
    }

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
    if collision {
        assert_eq!(
            run.scene()
                .object(20)
                .unwrap()
                .collision_vertices
                .as_deref(),
            Some(collision_points.as_slice())
        );
    }
    if profile.is_some() {
        assert_eq!(run.scene().particle_effects()[&0].particles.len(), 2);
        assert_eq!(
            run.inspect_snapshot().unwrap().raw_objects[&(0x800, 0)],
            snapshot.raw_objects[&(0x800, 0)]
        );
    }

    if matches!(profile, Some("cmvs-20211006" | "cmvs-20221105")) {
        assert!(run.scene().object(20).unwrap().children.contains_key(&1023));
        assert!(
            run.inspect_snapshot().unwrap().windows[0]
                .as_ref()
                .unwrap()
                .scene
                .children
                .contains_key(&1023)
        );
    }
    let restored = run.scene().object(20).unwrap().image.as_ref().unwrap();
    assert!(restored.alpha && !restored.surface && restored.name.is_empty());
    assert!(
        restored
            .pixels
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255])
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
    if profile.is_none() {
        let oversized = directory.join("save006.dat");
        let size = cmvs_core::MAX_PAYLOAD as u64 + 1;
        fs::File::create(&oversized).unwrap().set_len(size).unwrap();
        let system = fs::read(directory.join("system.dat")).unwrap();
        let error = run.save_slot(6).unwrap_err();
        assert_eq!(error.to_string(), "save exceeds research limit");
        assert_eq!(fs::metadata(&oversized).unwrap().len(), size);
        assert_eq!(fs::read(directory.join("save999.dat")).unwrap(), first);
        assert_eq!(fs::read(directory.join("system.dat")).unwrap(), system);
    }
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
    let changed = Csv2Save::encode(
        header,
        &with_targets.encode_with_layout(layout).unwrap(),
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
        &with_targets.encode_with_layout(layout).unwrap(),
        &save.auxiliary,
        &save.script_image,
    )
    .unwrap();
    fs::write(directory.join("save004.dat"), changed).unwrap();
    let before = run
        .inspect_snapshot()
        .unwrap()
        .encode_with_layout(layout)
        .unwrap();
    assert!(run.load_slot(4, &mut backend).is_err());
    assert_eq!(
        run.inspect_snapshot()
            .unwrap()
            .encode_with_layout(layout)
            .unwrap(),
        before
    );
    if collision {
        // Corrupt the on-disk 0x749 count rather than asking the checked encoder
        // to produce an invalid scene. The old VM, mesh and particles must survive.
        let mut mesh_bytes = snapshot.encode_with_layout(layout).unwrap();
        let mut mesh_header = 0x749u16.to_le_bytes().to_vec();
        mesh_header.extend(44u32.to_le_bytes());
        mesh_header.extend(0u32.to_le_bytes());
        mesh_header.extend(3u32.to_le_bytes());
        let at = mesh_bytes
            .windows(mesh_header.len())
            .position(|w| w == mesh_header)
            .unwrap();
        mesh_bytes[at + 10..at + 14].copy_from_slice(&u32::MAX.to_le_bytes());
        let bytes =
            Csv2Save::encode(header, &mesh_bytes, &save.auxiliary, &save.script_image).unwrap();
        fs::write(directory.join("save004.dat"), bytes).unwrap();
        assert!(
            run.load_slot(4, &mut backend)
                .unwrap_err()
                .to_string()
                .contains("collision vertex count")
        );
        assert_eq!(
            run.inspect_snapshot()
                .unwrap()
                .encode_with_layout(layout)
                .unwrap(),
            before
        );
        assert_eq!(
            run.scene()
                .object(20)
                .unwrap()
                .collision_vertices
                .as_deref(),
            Some(collision_points.as_slice())
        );
    }
    if profile.is_some() {
        let mut malformed = snapshot.clone();
        malformed.raw_objects.get_mut(&(0x800, 0)).unwrap()[24..28]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        let bytes = Csv2Save::encode(
            header,
            &malformed.encode_with_layout(layout).unwrap(),
            &save.auxiliary,
            &save.script_image,
        )
        .unwrap();
        fs::write(directory.join("save004.dat"), bytes).unwrap();
        let particles: Vec<_> = run.scene().particle_effects()[&0]
            .particles
            .iter()
            .map(|p| p.words)
            .collect();
        let pc = run.machine().pc();
        assert!(
            run.load_slot(4, &mut backend)
                .unwrap_err()
                .to_string()
                .contains("saved particle count")
        );
        assert_eq!(run.machine().pc(), pc);
        assert_eq!(
            run.inspect_snapshot()
                .unwrap()
                .encode_with_layout(layout)
                .unwrap(),
            before
        );
        assert_eq!(
            run.scene().particle_effects()[&0]
                .particles
                .iter()
                .map(|p| p.words)
                .collect::<Vec<_>>(),
            particles
        );
    }
    // The new main has a callback at offset 2, inside an instruction in the old main.
    assert!(!run.machine().is_boundary(0, 2));
    let new_main = commands::script(&[0x00, 0x20, 0x14, 0x04, 0, 0]);
    let new_image = new_main.memory_image(&[]).unwrap();
    callback.script_sizes[0] = new_image.len() as u32;
    callback.script_data.clear();
    callback.callbacks[target_offset..target_offset + 4].copy_from_slice(&2u32.to_le_bytes());
    callback.pc = 0;
    callback.repeat_count = 7;
    callback.stack_pointer = (callback.stack.len() - 12) as u32;
    let bytes = Csv2Save::encode(
        header,
        &callback.encode_with_layout(layout).unwrap(),
        &save.auxiliary,
        &new_image,
    )
    .unwrap();
    fs::write(directory.join("save005.dat"), bytes).unwrap();
    run.load_slot(5, &mut backend).unwrap();
    assert_eq!(run.machine().pc(), 2);
    assert_eq!(run.machine().repeat_count(), 0);
    assert_eq!(run.machine().stack().len_bytes(), callback.stack.len());
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.machine().pc(), 0);
    assert_eq!(run.machine().repeat_count(), 7);
    assert_eq!(
        run.machine().stack().len_bytes(),
        callback.stack_pointer as usize
    );
    drop(run);
    fs::remove_dir_all(&directory).unwrap();
    if profile.is_some() {
        fs::remove_file(root.0.join("data/pack/Synthetic")).unwrap();
    }
}

#[test]
fn gameplay_saves_need_an_independent_user_directory() {
    let mut code = vec![];
    code.extend(0x22b2u16.to_le_bytes());
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("snapshot-no-directory", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    // Nothing to write before the first snapshot, as in the original.
    assert!(!run.save_slot(0).unwrap());
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::SnapshotCaptured
    );
    let error = run.save_slot(0).unwrap_err();
    assert!(error.to_string().contains("--user-data-dir"), "{error}");
    assert!(!root.0.join("save/save000.dat").exists());
}

#[test]
fn button_groups_are_saved_and_come_back_empty_on_object_zero() {
    use cmvs_core::format::snapshot::EngineSnapshot;
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
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("runner-buttons");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        match run.tick(&mut backend).unwrap() {
            RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured => {}
            RuntimeEvent::Unhandled {
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
fn old_save_comment_edit_preserves_payload_and_rejects_missing_or_invalid_slots() {
    for profile in ["cmvs-20071117", "cmvs-20080719"] {
        let mut code = vec![];
        for slot in [0, 1, 2, 3, 1000] {
            commands::command(&mut code, 0x22bc, &[slot, 0]);
        }
        let root = Root::new(&format!("comment-{profile}"), &code, b"updated\0");
        fs::write(root.0.join("save/system.dat"), auto_resume_save(0)).unwrap();
        let game = Registry::builtin()
            .identify_game(&root.0, Some(profile))
            .unwrap();
        let script = load_start(&root.0, &game.files).unwrap();
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        let user = root.0.with_extension("user");
        run.set_user_data_directory(&user).unwrap();
        let mut original = vec![0x5a; 900];
        original[..4].copy_from_slice(b"CSV2");
        fs::write(user.join("save000.dat"), &original).unwrap();
        fs::write(user.join("save002.dat"), b"CSV2").unwrap();
        fs::write(user.join("save003.dat"), vec![0; 600]).unwrap();
        let mut backend = RecordingBackend::default();
        for (i, expected) in [1, 0, 0, 0, 0].into_iter().enumerate() {
            let end = (i + 1) * 18;
            while run.machine().pc() < end {
                run.tick(&mut backend).unwrap();
            }
            assert_eq!(run.machine().state().system_integer(0), Some(expected));
            assert_eq!(run.machine().stack().len_bytes(), 0);
        }
        let mut expected = original;
        expected[272..280].copy_from_slice(b"updated\0");
        assert_eq!(fs::read(user.join("save000.dat")).unwrap(), expected);
        assert_eq!(fs::read(user.join("save002.dat")).unwrap(), b"CSV2");
        assert!(!user.join("save001.dat").exists());
        drop(run);
        fs::remove_dir_all(user).unwrap();
    }
}

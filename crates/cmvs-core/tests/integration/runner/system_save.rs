use super::*;

#[test]
fn user_configuration_round_trips_live_settings_and_preserves_the_game_file() {
    use cmvs_core::{host::settings::DisplayPreferences, render::raster::ScaleFilter};
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
    let original = b"[SOUND_INFO]\nINACTIVE_SOUND_PLAY=0\n[MESSAGE_INFO]\nTEXT_WAIT=2\nTEXT_BOLD=0\nTEXT_REVISION_ALPHA=1\n[CMVS_RS_DISPLAY]\nSCALE_FILTER=lanczos4\n[FUTURE]\nUNKNOWN=\x82\xa0\n";
    fs::write(root.0.join("cmvs.cfg"), original).unwrap();
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("preferences");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(!run.save_preferences(DisplayPreferences::default()).unwrap());
    run.set_user_data_directory(&directory).unwrap();
    assert_eq!(
        run.scale_filter_override(None).unwrap(),
        Some(ScaleFilter::Lanczos4)
    );
    assert_eq!(run.scale_filter_override(Some("game")).unwrap(), None);
    for (name, filter) in [
        ("nearest", ScaleFilter::Nearest),
        ("bilinear", ScaleFilter::Bilinear),
        ("keys", ScaleFilter::Keys),
        ("mitchell", ScaleFilter::Mitchell),
        (
            "lanczos3",
            ScaleFilter::Lanczos3 {
                anti_ringing: false,
            },
        ),
        (
            "lanczos3-anti-ringing",
            ScaleFilter::Lanczos3 { anti_ringing: true },
        ),
        ("lanczos4", ScaleFilter::Lanczos4),
    ] {
        assert_eq!(run.scale_filter_override(Some(name)).unwrap(), Some(filter));
    }
    assert!(run.scale_filter_override(Some("Typo")).is_err());
    let mut backend = RecordingBackend::default();
    let mut yielded = false;
    for _ in 0..100 {
        if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
            yielded = true;
            break;
        }
    }
    assert!(yielded);
    let settings = run.settings().clone();
    assert!(settings.text_output().revision_alpha);
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
    let mut resumed = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    resumed.set_user_data_directory(&directory).unwrap();
    assert_eq!(
        resumed.scale_filter_override(None).unwrap(),
        Some(ScaleFilter::Lanczos4)
    );
    assert_eq!(resumed.settings(), &settings);
    assert_eq!(resumed.audio().settings(), audio);
    assert_eq!(resumed.audio().voice_ducking(), duck);
    assert!(!resumed.audio().background_output());
    assert!(!cmvs_core::host::audio::AudioHost::from_ini(&first).background_output());
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
fn system_save_export_is_opt_in_external_and_becomes_the_read_source() {
    use cmvs_core::format::system_save::Css1Save;
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(
        run.set_system_save_output(&root.0.join("save/new.dat"))
            .is_err()
    );
    let mut backend = RecordingBackend::default();
    loop {
        if let RuntimeEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x2129);
            break;
        }
    }
    assert!(!output.exists());
    assert!(run.set_system_save_output(&output).is_err()); // Too late to configure.
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_system_save_output(&output).unwrap();
    loop {
        if let RuntimeEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
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
    use cmvs_core::{format::system_save::Css1Save, vm::evaluate::State};
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
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("runner-profile");
    assert!(!directory.exists());
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
        if let RuntimeEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
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
    let mut restarted = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    restarted.set_user_data_directory(&directory).unwrap();
    while restarted.machine().state().integer(2048).is_none() {
        restarted.tick(&mut backend).unwrap();
    }
    assert_eq!(restarted.machine().state().integer(2048), Some(99));
    drop(restarted);
    fs::write(directory.join("system.dat"), b"corrupt profile").unwrap();
    let mut corrupt = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        if skip {
            run.set_skip_auto_resume(true).unwrap();
        }
        let mut backend = RecordingBackend::default();
        for _ in 0..3 {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
            RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    assert!(run.set_skip_auto_resume(true).is_err());
    // A record verified for another game ID stops at 0x201d, before any system state.
    let registry = root.recorded_registry(0x1780_0016);
    let game = registry.identify_game(&root.0, None).unwrap();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
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
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
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
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
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
        let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
        run.set_record_text(true);
        let mut backend = RecordingBackend::default();
        let mut reached = false;
        for _ in 0..100 {
            match run.tick(&mut backend) {
                Ok(RuntimeEvent::Progress) => {}
                Ok(RuntimeEvent::Yield) => {
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
fn first_system_save_without_template_writes_reloads_and_survives_restart() {
    use cmvs_core::{format::system_save::Css1Save, host::settings::DisplayPreferences};
    let registry = Registry::new(vec![synthetic_profile("cmvs-20160201")], vec![]);
    for explicit_write in [false, true] {
        let mut code = vec![];
        commands::command(&mut code, 0x201d, &[0x12345678]);
        // Like real startup, initialize only local banks in the script.
        commands::command(&mut code, 0x2091, &[0, 2048, 0]);
        commands::command(&mut code, 0x2092, &[0, 2048, 0]);
        commands::command(&mut code, 0x2093, &[0, 1024, 0]);
        commands::command(&mut code, 0x2094, &[0, 64, 0]);
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
        let directory = crate::support::temp_dir::TempPath::in_target_tmp(&format!(
            "first-css1-{explicit_write}"
        ));
        let script = root.script();
        let game = registry
            .identify_game(&root.0, Some("cmvs-20160201"))
            .unwrap();
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        assert_eq!(run.machine().state().integer(2190), Some(0));
        assert_eq!(run.machine().state().integer(4095), Some(0));
        assert_eq!(run.machine().state().integer(4097), None);
        assert_eq!(run.machine().state().bit(262143), Some(false));
        assert_eq!(run.machine().state().float_bits(2047), Some(0));
        assert_eq!(run.machine().state().string(127), Some(b"".as_slice()));
        run.set_user_data_directory(&directory).unwrap();
        // An early close can write INI, but must not manufacture a CSS1.
        run.save_preferences(DisplayPreferences::default()).unwrap();
        assert!(!directory.join("system.dat").exists());
        let mut backend = RecordingBackend::default();
        loop {
            if let RuntimeEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
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
        let mut resumed = Runtime::new(&script, &root.0, &game, 0).unwrap();
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

#[test]
fn runtime_options_load_and_lock_the_selected_user_profile_before_execution() {
    let root = Root::new("runtime-options", &0x2000u16.to_le_bytes(), b"");
    let user = root.0.with_extension("user");
    fs::create_dir(&user).unwrap();
    fs::write(user.join("cmvs.cfg"), b"[MESSAGE_INFO]\nTEXT_WAIT=7\n").unwrap();
    let script = root.script();
    let game = root.game();
    let options = || RuntimeOptions {
        encoding: cmvs_core::TextEncoding::Gbk,
        saves: SaveStorage::UserDirectory(user.clone()),
        ..Default::default()
    };
    let mut run = Runtime::with_options(&script, &root.0, &game, options()).unwrap();
    assert_eq!(run.machine().text_encoding(), cmvs_core::TextEncoding::Gbk);
    assert_eq!(run.settings().text_output().speed, 7);
    assert_eq!(run.steps(), 0);
    assert!(Runtime::with_options(&script, &root.0, &game, options()).is_err());
    assert_eq!(
        run.tick(&mut RecordingBackend::default()).unwrap(),
        RuntimeEvent::Yield
    );
    assert!(!root.0.join("save/system.dat").exists());
    drop(run);
    drop(Runtime::with_options(&script, &root.0, &game, options()).unwrap());
    fs::remove_dir_all(user).unwrap();
}

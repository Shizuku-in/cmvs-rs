//! Gameplay save slots and the in-game menus through `Startup` on the original game: external
//! saves, route replays and the toolbar, save screens and backlog, read-only inputs.
//! Synthetic save tests live in tests/runner.rs.
use cmvs_rs::{
    history::HistoryEntry,
    input::{CodeInput, KeyBindings},
    message::AuxiliaryText,
    platform::RecordingBackend,
    profile::Registry,
    runner::{SnapshotPolicy, Startup, StartupEvent, load_start},
    save::Csv2Save,
    snapshot::{EngineSnapshot, SavedScene},
};
use std::{fs, path::PathBuf};

fn user_directory(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("save-slots-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The loader resets the draw +84 fade factor (0x14001f080).
fn reset_fades(scene: &mut SavedScene) {
    if let Some(draw) = &mut scene.draw {
        draw.words[21] = 256;
    }
    for child in scene.children.values_mut() {
        reset_fades(child);
    }
}

fn original_snapshot(root: &std::path::Path, slot: u16) -> EngineSnapshot {
    let path = root.join(format!("save/save{slot:03}.dat"));
    EngineSnapshot::decode(&Csv2Save::decode(&fs::read(path).unwrap()).unwrap().snapshot).unwrap()
}

/// Compares the hosts right after a load with the saved snapshot, normalizing only the
/// documented loader effects.
fn check_loaded(run: &Startup<'_>, mut expected: EngineSnapshot) {
    let mut actual = run.inspect_snapshot().unwrap();
    // The load callback 29 runs in a cross-script frame holding the restored context.
    let callback = &run.control().callbacks()[&29];
    assert_eq!(
        (actual.active_slot, actual.pc),
        (u32::from(callback.script_slot), callback.target)
    );
    assert_eq!(actual.stack_pointer, expected.stack_pointer + 12);
    let sp = expected.stack_pointer as usize;
    let frame: Vec<u32> = actual.stack[sp..sp + 12]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        frame,
        [expected.repeat_count, expected.pc, expected.active_slot]
    );
    assert_eq!(actual.stack[..sp], expected.stack[..sp]);
    (actual.active_slot, actual.pc, actual.repeat_count) =
        (expected.active_slot, expected.pc, expected.repeat_count);
    (actual.stack_pointer, actual.stack) = (expected.stack_pointer, expected.stack.clone());
    // Rust keeps no stale frame entries above the depth, nor results the executor cannot set.
    actual.frames = expected.frames;
    actual.system_results[7..].copy_from_slice(&expected.system_results[7..]);
    // Common-script callbacks keep the current registrations, whose clocks differ.
    let ticks = |record: &[u8]| -> Vec<u8> {
        let mut out = record.to_vec();
        for i in 0..64 {
            let at = 4 + 28 * i;
            if out[at + 4..at + 8] != [0; 4] {
                out[at + 24..at + 28].fill(0);
            }
        }
        out
    };
    (actual.callbacks, expected.callbacks) = (ticks(&actual.callbacks), ticks(&expected.callbacks));
    // After loading, 0x140092ea0 enables the second menu group.
    expected.word_16584 = 1;
    for scene in expected.objects.values_mut() {
        reset_fades(scene);
    }
    // 0x14005c8f0 redraws descriptors at their saved positions, while resetting
    // the text pen independently to the area origin.
    for text in expected.texts.iter_mut().flatten() {
        text.cursor = [text.area[0], text.area[1]];
    }
    // A stopped SE channel keeps its old name behind a cleared first byte in the original.
    for channel in expected.sound_channels.chunks_exact_mut(52) {
        if channel[0] == 0 {
            channel[..32].fill(0);
        }
    }
    let encoded = |s: &EngineSnapshot| s.encode().unwrap();
    let mut differences = Vec::new();
    macro_rules! check {
        ($($field:ident),*) => {$(
            if actual.$field != expected.$field {
                differences.push(stringify!($field));
            }
        )*};
    }
    check!(
        save_title,
        scene_title,
        script_names,
        script_sizes,
        timers,
        word_16584,
        callbacks,
        clear_color,
        word_11144,
        word_12092,
        depth_clear,
        frame_depth,
        system_results,
        variables,
        texts,
        windows,
        objects,
        raw_objects,
        effects,
        record_6e0,
        cameras,
        script_data,
        bgm,
        sound_channels,
        sound_device
    );
    if !differences.is_empty() {
        println!("differences: {differences:?}");
    }
    for (i, (a, e)) in actual
        .sound_channels
        .chunks_exact(52)
        .zip(expected.sound_channels.chunks_exact(52))
        .enumerate()
    {
        if a != e {
            println!(
                "channel {i}
  actual   {a:02x?}
  expected {e:02x?}"
            );
        }
    }
    assert!(differences.is_empty(), "{differences:?}");
    assert_eq!(encoded(&actual), encoded(&expected));
}

/// Original auto-resume: `main.ps3` loads slot 870 with 0x22b5 after the system save. The
/// snapshot re-captured from the hosts right after the load must equal the original apart
/// from the documented loader effects.
#[test]
#[ignore = "requires CMVS_GAME_ROOT with original system.dat and save870.dat in save/"]
fn external_auto_resume_restores_the_original_snapshot() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let user = user_directory("resume");
    for name in [
        "system.dat",
        "save870.dat",
        "save000.dat",
        "save001.dat",
        "save002.dat",
    ] {
        fs::copy(root.join("save").join(name), user.join(name)).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_snapshot_policy(SnapshotPolicy::Capture);
    let mut backend = RecordingBackend::default();
    // Run to the title menu idle, then load as 0x22b5 would.
    for _ in 0..2_000_000 {
        let event = run.tick(&mut backend).unwrap();
        if event == StartupEvent::Yield {
            run.advance_frame(16).unwrap();
            if run.machine().current_name() == b"menu.ps3" && run.steps() > 200_000 {
                break;
            }
        }
        assert!(
            matches!(
                event,
                StartupEvent::Progress | StartupEvent::Yield | StartupEvent::SnapshotCaptured
            ),
            "{event:?} at {} / 0x{:x}",
            run.machine().current_name().escape_ascii(),
            run.machine().pc()
        );
    }
    assert_eq!(run.machine().current_name(), b"menu.ps3");
    run.load_slot(870, &mut backend).unwrap();
    check_loaded(&run, original_snapshot(&root, 870));
    run.set_record_text(true);
    for slot in [0, 1, 2] {
        run.load_slot(slot, &mut backend).unwrap();
        check_loaded(&run, original_snapshot(&root, slot));
        // The restored game runs on with skip held (action 7, Ctrl): through the load
        // callback and text until the script reaches its next snapshot point.
        run.input_mut().set(7, true, false, false).unwrap();
        let (mut captured, start) = (0, run.steps());
        for _ in 0..3_000_000 {
            match run.tick(&mut backend).unwrap() {
                StartupEvent::Yield => run.advance_frame(16).unwrap(),
                StartupEvent::SnapshotCaptured => {
                    captured += 1;
                    if captured == 2 {
                        break;
                    }
                }
                StartupEvent::Progress => {}
                other => panic!(
                    "slot {slot}: {other:?} at {} / 0x{:x}",
                    run.machine().current_name().escape_ascii(),
                    run.machine().pc()
                ),
            }
        }
        run.input_mut().set(7, false, false, true).unwrap();
        println!(
            "slot {slot}: {} steps to {} / 0x{:x}, snapshots captured: {captured}",
            run.steps() - start,
            run.machine().current_name().escape_ascii(),
            run.machine().pc()
        );
        assert_eq!(
            captured, 2,
            "slot {slot}: the game stopped before two snapshots"
        );
        // Write the held snapshot to a new slot through 0x22b6's path and load it back.
        let held = match run.held_snapshot() {
            cmvs_rs::snapshot::SaveImage::Captured(c) => {
                EngineSnapshot::decode(&c.snapshot).unwrap()
            }
            other => panic!("{other:?}"),
        };
        assert!(run.save_slot(100 + slot).unwrap());
        run.load_slot(100 + slot, &mut backend).unwrap();
        check_loaded(&run, held);
    }
    fs::remove_dir_all(user).unwrap();
}

/// The held snapshot as the writer built it.
fn held(run: &Startup<'_>) -> EngineSnapshot {
    match run.held_snapshot() {
        cmvs_rs::snapshot::SaveImage::Captured(c) => EngineSnapshot::decode(&c.snapshot).unwrap(),
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "requires CMVS_GAME_ROOT; read-only game with fresh temporary user directory"]
fn external_mixed_skip_click_captures_active_image_change() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let user = user_directory("mixed-skip-click");
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    run.set_record_text(true);
    let bindings = KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap();
    let mut codes = CodeInput::default();
    let mut backend = RecordingBackend::default();
    let (mut started, mut frame, mut captures) = (false, 0u32, 0u32);
    let mut random = 0x1234_5678u32;
    let mut mixed = [false; 128];
    for _ in 0..8_000_000 {
        if !started && run.machine().current_name() == b"menu.ps3" && run.machine().pc() == 0x38f00
        {
            let b = run.buttons().group(0).unwrap().buttons[0].clone();
            run.input_mut()
                .set_pointer(b.rect[0] + b.rect[2] / 2, b.rect[1] + b.rect[3] / 2);
            started = true;
        }
        let at = format!(
            "{} / 0x{:x}",
            run.machine().current_name().escape_ascii(),
            run.machine().pc()
        );
        let event = run
            .tick(&mut backend)
            .unwrap_or_else(|e| panic!("{e} at {at}, frame={frame}"));
        if event == StartupEvent::SnapshotCaptured && started {
            captures += 1;
            if run.scene().objects().values().any(|o| o.change.is_some()) {
                let expected = held(&run);
                let live = run.scene().objects().clone();
                assert!(run.save_slot(0).unwrap());
                assert_eq!(run.scene().objects(), &live);
                run.load_slot(0, &mut backend).unwrap();
                check_loaded(&run, expected);
                assert!(run.scene().objects().values().all(|o| o.change.is_none()));
                println!(
                    "active change captured and loaded at {at}; frame={frame}, captures={captures}"
                );
                fs::remove_dir_all(user).unwrap();
                return;
            }
        }
        if event != StartupEvent::Yield {
            assert!(
                matches!(
                    event,
                    StartupEvent::Progress | StartupEvent::SnapshotCaptured
                ),
                "{event:?} at {at}"
            );
            continue;
        }
        run.advance_frame(16).unwrap();
        if started {
            frame += 1;
            if frame < 4 {
                mixed[1] = frame == 1;
            } else {
                run.input_mut().set_pointer(640, 650);
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                mixed[1] = random & 3 == 0;
                mixed[24] = random & 4 != 0;
            }
            codes.update(&bindings, &mixed, run.input_mut());
        }
    }
    panic!("no active change snapshot reached; frame={frame}, captures={captures}");
}

/// Replays one route from a fresh user directory with the default capture policy: skip
/// held after the title, the `CMVS_ROUTE` choice chain (1-based enabled buttons, e.g.
/// `2,2,2,1`), and a confirm after 300 idle frames outside the staff roll. Every snapshot
/// goes to slot 0 through the 0x22b6 path and must read back unchanged; every
/// `CMVS_ROUTE_LOAD_EVERY`th one (default 10, since each load replays callback 29 and the
/// scene setup) is also loaded as 0x22b5 would and the hosts must match it, and the route
/// then plays on from the loaded state until the title is ready again after the ending.
/// `CMVS_ROUTE_SYSTEM` optionally seeds the user system.dat (END2 needs END1's) and
/// `CMVS_ROUTE_SYSTEM_OUT` keeps the final one.
#[test]
#[ignore = "requires CMVS_GAME_ROOT and CMVS_ROUTE; a full route takes tens of minutes in release"]
fn external_route_saves_every_snapshot_and_plays_on_after_loads() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let chain: Vec<usize> = std::env::var("CMVS_ROUTE")
        .expect("CMVS_ROUTE")
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    let load_every: u32 = std::env::var("CMVS_ROUTE_LOAD_EVERY").map_or(10, |v| v.parse().unwrap());
    let user = user_directory("route");
    if let Some(system) = std::env::var_os("CMVS_ROUTE_SYSTEM") {
        fs::copy(system, user.join("system.dat")).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    let bindings = KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap();
    let mut codes = CodeInput::default();
    let mut backend = RecordingBackend::default();
    let (mut phase, mut point) = (0, [-1, -1]);
    let (mut choices, mut choice_idle, mut choice_step) = (0, None::<u32>, 0);
    let (mut yields, mut last_input, mut system_saved) = (0u64, 0u64, false);
    let (mut captures, mut loads) = (0u32, 0u32);
    let place = |run: &Startup<'_>| {
        format!(
            "{} / 0x{:x} after {} steps",
            run.machine().current_name().escape_ascii(),
            run.machine().pc(),
            run.steps()
        )
    };
    for iteration in 0..1_000_000_000u64 {
        // The recording backends keep at most 4096 requests; every load sets the title.
        if iteration % 100_000 == 0 {
            run.set_audio_backend(Box::new(cmvs_rs::audio::RecordingAudio::default()))
                .unwrap();
            backend = RecordingBackend::default();
        }
        let at_title = run.machine().current_name() == b"menu.ps3" && run.machine().pc() == 0x38f00;
        if at_title && system_saved {
            println!(
                "route {chain:?}: title ready, choices={choices} captures={captures} loads={loads}, {}",
                place(&run)
            );
            assert_eq!(choices, chain.len());
            if let Some(out) = std::env::var_os("CMVS_ROUTE_SYSTEM_OUT") {
                fs::copy(user.join("system.dat"), out).unwrap();
            }
            fs::remove_dir_all(user).unwrap();
            return;
        }
        if phase == 0 && at_title {
            let group = run.buttons().group(0).unwrap();
            let b = &group.buttons[0];
            point = [b.rect[0] + b.rect[2] / 2, b.rect[1] + b.rect[3] / 2];
            run.input_mut().set_pointer(point[0], point[1]);
            phase = 1;
        }
        let pc = run.machine().pc();
        let opcode = run
            .machine()
            .loaded_script(run.machine().current_slot())
            .and_then(|s| s.code().get(pc..pc + 2))
            .map(|b| u16::from_le_bytes([b[0], b[1]]));
        let event = run
            .tick(&mut backend)
            .unwrap_or_else(|e| panic!("{e} at {}", place(&run)));
        match event {
            StartupEvent::Progress | StartupEvent::Yield => {}
            StartupEvent::SnapshotCaptured => {
                captures += 1;
                let image = held(&run);
                assert!(run.save_slot(0).unwrap());
                let file = Csv2Save::decode(&fs::read(user.join("save000.dat")).unwrap()).unwrap();
                assert_eq!(EngineSnapshot::decode(&file.snapshot).unwrap(), image);
                if captures.is_multiple_of(load_every) {
                    loads += 1;
                    let main = image.script_names[0].escape_ascii().to_string();
                    run.load_slot(0, &mut backend)
                        .unwrap_or_else(|e| panic!("load: {e} in {main} at {}", place(&run)));
                    check_loaded(&run, image);
                    if loads.is_multiple_of(50) {
                        println!(
                            "load {loads} (capture {captures}) in {main} at {}",
                            place(&run)
                        );
                    }
                }
                continue;
            }
            other => panic!("{other:?} at {}", place(&run)),
        }
        if opcode == Some(0x2129) && run.machine().pending_system().is_none() {
            system_saved = true;
        }
        if opcode == Some(0x210b) && run.machine().pending_system().is_none() {
            choice_idle = Some(0);
        }
        if event != StartupEvent::Yield {
            continue;
        }
        yields += 1;
        run.advance_frame(16).unwrap();
        let waiting = matches!(run.machine().pending_system(), Some((_, 0x153)))
            && run.machine().repeat_count() == 2;
        let mut now = [false; 128];
        if phase > 0 && phase < 4 {
            phase += 1;
            now[1] = phase == 2;
        }
        now[24] = phase >= 4;
        if waiting {
            now[1] = true;
            choice_idle = None;
            last_input = yields;
        }
        if phase >= 4
            && !run.machine().current_name().starts_with(b"staff")
            && choice_idle.is_none()
            && choice_step == 0
            && yields - last_input > 300
        {
            now[1] = true;
            last_input = yields;
        }
        if let Some(idle) = choice_idle.as_mut() {
            *idle += 1;
            if *idle == 30 {
                let group = (0..7)
                    .rev()
                    .filter_map(|g| run.buttons().group(g))
                    .find(|group| !group.buttons.is_empty());
                if let Some(group) = group {
                    let pick = *chain.get(choices).expect("unexpected extra choice");
                    let b = group
                        .buttons
                        .iter()
                        .filter(|b| b.enabled)
                        .nth(pick - 1)
                        .expect("choice unavailable");
                    point = [b.rect[0] + b.rect[2] / 2, b.rect[1] + b.rect[3] / 2];
                    choices += 1;
                    choice_step = 1;
                    println!("choice {choices} id={} at {}", b.id, place(&run));
                }
            }
        }
        if choice_step > 0 {
            choice_step += 1;
            now[1] = choice_step == 3;
            if choice_step > 3 {
                choice_step = 0;
                choice_idle = None;
            }
        }
        codes.update(&bindings, &now, run.input_mut());
        run.input_mut().set_pointer(point[0], point[1]);
    }
    panic!("budget exhausted at {}", place(&run));
}

/// Drives the game's own UI: the toolbar (message window 7, run by the automatic
/// callbacks), its confirmation dialog (button group 2, id 1 = yes), the save and load
/// screens (button group 1, slot ids from 70) and the backlog. Input goes through key.cfg
/// bindings: code 1 left button, 3 right button, 4/5 wheel up/down.
struct Ui<'a, 'b> {
    native_audio: bool,
    run: &'b mut Startup<'a>,
    backend: RecordingBackend,
    bindings: KeyBindings,
    codes: CodeInput,
    /// Save commands completed (0x22b6 / 0x22b5) since the last check.
    commands: Vec<u16>,
    /// Voice files 0x20a6 played (SP-8) since the last check.
    voices: Vec<Vec<u8>>,
    /// SP-12 of each 0x20a6, the per-call volume before the global voice gain.
    voice_volumes: Vec<i32>,
    /// SP-20 of 0x20af, or 1.0 for 0x20a6.
    voice_rates: Vec<f32>,
    voice_fades: Vec<(u32, u32)>,
    /// Set when a load completes; frames do nothing until the test has checked the hosts.
    frozen: bool,
}

/// Title and toolbar settings entries, three pages and right-click return.
/// Recording-audio variant; the native variant shares the same assertions below.
#[cfg(feature = "desktop")]
#[test]
#[ignore = "requires CMVS_GAME_ROOT with system.dat/save000 and CMVS_FONT_PATH"]
fn external_system_settings_entries_render_and_return() {
    system_settings_entries_render_and_return(false);
}

#[cfg(all(feature = "desktop", feature = "audio-output"))]
#[test]
#[ignore = "requires CMVS_GAME_ROOT, CMVS_FONT_PATH and the native audio device"]
fn external_native_system_settings_entries_render_and_return() {
    system_settings_entries_render_and_return(true);
}

#[cfg(feature = "desktop")]
fn system_settings_entries_render_and_return(native_audio: bool) {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let font_path = PathBuf::from(std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH"));
    let mut font = cmvs_rs::desktop::font::FontRenderer::read(&font_path, 0).unwrap();
    let user = user_directory(if native_audio {
        "settings-native"
    } else {
        "settings"
    });
    for name in ["system.dat", "save000.dat"] {
        fs::copy(root.join("save").join(name), user.join(name)).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    #[cfg(feature = "audio-output")]
    if native_audio {
        run.set_audio_backend(Box::new(
            cmvs_rs::audio_output::NativeAudio::new(&root).unwrap(),
        ))
        .unwrap();
    }
    #[cfg(not(feature = "audio-output"))]
    assert!(!native_audio);
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    run.set_glyph_metrics(font.metrics());
    let mut ui = Ui {
        native_audio,
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
        voice_volumes: Vec::new(),
        voice_rates: Vec::new(),
        voice_fades: Vec::new(),
        frozen: false,
    };
    // The desktop host applies the startup filter; the sample cmvs.cfg word 0 means linear.
    let startup_filter = cmvs_rs::platform::supported_render_mode(
        &ui.backend,
        ui.run.display_preferences().render_mode,
    )
    .unwrap();
    assert_eq!(startup_filter, 1);
    cmvs_rs::platform::PlatformBackend::apply(
        &mut ui.backend,
        &cmvs_rs::platform::PlatformRequest::RenderMode(startup_filter),
    )
    .unwrap();
    for _ in 0..30_000 {
        if ui.run.machine().current_name() == b"menu.ps3"
            && if native_audio {
                ui.run
                    .buttons()
                    .group(0)
                    .is_some_and(|group| group.buttons.len() >= 4)
            } else {
                ui.run.steps() > 200_000
            }
        {
            break;
        }
        ui.frame([-1, -1], None);
    }
    assert_eq!(ui.run.machine().current_name(), b"menu.ps3");
    // The title is still fading in when its button group is first created.
    for _ in 0..600 {
        ui.frame([-1, -1], None);
    }
    ui.click(ui.button(0, 2), 120);
    assert_eq!(ui.run.buttons().group(1).unwrap().buttons.len(), 291);
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.press([640, 360], 3, 120);
    assert_eq!(ui.run.machine().current_name(), b"menu.ps3");
    ui.run.load_slot(0, &mut ui.backend).unwrap();
    ui.wait_for_text();
    ui.click(ui.toolbar(0), 120);
    let group = ui.run.buttons().group(1).unwrap();
    assert_eq!(group.object, 64);
    assert_eq!(group.buttons.len(), 292);
    assert!(ui.run.messages().window(6).is_some());
    let initial_audio = ui.run.audio().settings();
    for (mode, x) in [160, 264, 368, 472, 576].into_iter().enumerate() {
        ui.click([x, 626], 30);
        assert_eq!(ui.run.settings().cursor_auto_clear(), mode as u32);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    // 音声再生速度「少し速い」stores 1 in 2175; story scripts then play voices with 0x20af at 1.5.
    ui.click(ui.button(1, 43), 30);
    assert_eq!(ui.run.machine().state().integer(2175), Some(1));
    // フォント選択: 0x213d opens the host font dialog. The recording backend answers
    // ＭＳ 明朝, which the desktop library resolves from the system font directory; the
    // demonstration text then re-emits with glyphs in that face.
    let mincho = b"\x82l\x82r \x96\xbe\x92\xa9";
    ui.backend.answer_font_dialog(Some(mincho.to_vec()));
    ui.click(ui.button(1, 10), 30);
    assert_eq!(ui.run.settings().font_face(), mincho);
    let mut mincho_glyphs = false;
    for _ in 0..600 {
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        if (0..cmvs_rs::message::TEXT_OBJECTS)
            .filter_map(|slot| ui.run.messages().text(slot))
            .flat_map(|text| &text.glyphs)
            .any(|glyph| *glyph.face == *mincho)
        {
            mincho_glyphs = true;
            break;
        }
        ui.frame([640, 360], None);
    }
    assert!(
        mincho_glyphs,
        "the chosen face must reach newly emitted glyphs"
    );
    ui.backend.answer_font_dialog(None);
    ui.click(ui.button(1, 10), 30);
    assert_eq!(ui.run.settings().font_face(), mincho);
    ui.backend
        .answer_font_dialog(Some(cmvs_rs::settings::DEFAULT_FONT_FACE.to_vec()));
    ui.click(ui.button(1, 10), 30);
    assert_eq!(
        ui.run.settings().font_face(),
        cmvs_rs::settings::DEFAULT_FONT_FACE
    );
    assert_eq!(
        ui.backend.font_dialogs(),
        [
            cmvs_rs::settings::DEFAULT_FONT_FACE,
            mincho.as_slice(),
            mincho.as_slice()
        ]
    );
    for (x, bold) in [(472, false), (576, true), (472, false)] {
        ui.click([x, 107], 30);
        assert_eq!(ui.run.settings().text_output().bold, bold);
        assert!(
            (0..cmvs_rs::message::TEXT_OBJECTS)
                .filter_map(|slot| ui.run.messages().text(slot))
                .filter(|text| text.font_face.is_empty())
                .all(|text| text.bold == bold)
        );
        if bold {
            // The demonstration clears and re-emits its text on a timer.
            for _ in 0..600 {
                if (0..cmvs_rs::message::TEXT_OBJECTS)
                    .filter_map(|slot| ui.run.messages().text(slot))
                    .any(|text| text.glyphs.iter().any(|glyph| glyph.bold))
                {
                    break;
                }
                ui.frame([640, 360], None);
            }
            assert!(
                (0..cmvs_rs::message::TEXT_OBJECTS)
                    .filter_map(|slot| ui.run.messages().text(slot))
                    .any(|text| text.glyphs.iter().any(|glyph| glyph.bold))
            );
        }
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (mode, x) in [1008, 1112, 1216].into_iter().enumerate() {
        ui.click([x, 173], 30);
        assert_eq!(ui.run.message_flags().mode_1548(), mode as u32);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (x, expected) in [(174, false), (278, true)] {
        ui.click([x, 239], 30);
        assert_eq!(ui.run.settings().read_skip(), expected);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (outline, x) in [368, 472, 576].into_iter().enumerate() {
        ui.click([x, 173], 30);
        assert_eq!(ui.run.settings().text_output().outline, outline as u32);
        for _ in 0..30 {
            font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
                .unwrap();
            ui.frame([640, 360], None);
        }
    }
    let mut last_speed = ui.run.settings().text_output().speed;
    for x in [300, 560] {
        ui.click([x, 306], 30);
        let speed = ui.run.settings().text_output().speed;
        assert_ne!(speed, last_speed);
        last_speed = speed;
        for _ in 0..30 {
            font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
                .unwrap();
            ui.frame([640, 360], None);
        }
    }
    for (index, y) in [380, 420, 460].into_iter().enumerate() {
        let values =
            |s: cmvs_rs::settings::AutoAdvanceSettings| [s.base, s.per_glyph, s.voice_wait];
        let before = ui.run.settings().auto_advance();
        ui.click([220, y], 30);
        let after = ui.run.settings().auto_advance();
        assert_ne!(values(before)[index], values(after)[index]);
        for other in 0..3 {
            if other != index {
                assert_eq!(values(before)[other], values(after)[other]);
            }
        }
        assert_eq!(before.sync_voice, after.sync_voice);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    // The original slider logic quantizes the pointer and calls 0x22d9..0x22dd.
    for (index, y) in [252, 286, 320, 354, 388].into_iter().enumerate() {
        let before = ui.run.audio().settings();
        ui.click([900, y], 30);
        let after = ui.run.audio().settings();
        let fields = |s: cmvs_rs::audio::AudioSettings| [s.master, s.bgm, s.se, s.voice, s.movie];
        assert_ne!(fields(before)[index], fields(after)[index]);
        for other in 0..5 {
            if other != index {
                assert_eq!(fields(before)[other], fields(after)[other]);
            }
        }
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    // Sliders are rows of step buttons: 0x2217 returns -10 when press and release land
    // on different steps, and this page ignores -10, so a drag changes nothing.
    let before = ui.run.audio().settings();
    ui.drag([830, 252], [1150, 252]);
    assert_eq!(ui.run.audio().settings(), before);
    // Speaker icons toggle MUSIC_USE / SE_USE / VOICE_USE, without changing volume.
    for (index, y) in [286, 320, 354].into_iter().enumerate() {
        let before = ui.run.audio().settings();
        let flags =
            |s: cmvs_rs::audio::AudioSettings| [s.bgm_enabled, s.se_enabled, s.voice_enabled];
        ui.click([1194, y], 30);
        let mut expected = before;
        match index {
            0 => expected.bgm_enabled = !before.bgm_enabled,
            1 => expected.se_enabled = !before.se_enabled,
            _ => expected.voice_enabled = !before.voice_enabled,
        }
        assert_eq!(ui.run.audio().settings(), expected);
        assert_ne!(
            flags(ui.run.audio().settings())[index],
            flags(before)[index]
        );
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        ui.click([1194, y], 30);
        assert_eq!(ui.run.audio().settings(), before);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for y in [286, 320, 354] {
        ui.click([1194, y], 30);
    }
    let before_reset = ui.run.audio().settings();
    assert!(!before_reset.bgm_enabled && !before_reset.se_enabled && !before_reset.voice_enabled);
    ui.click([1215, 428], 30);
    assert_eq!(ui.run.audio().settings(), before_reset);
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.click(ui.button(2, 2), 30);
    assert_eq!(ui.run.audio().settings(), before_reset);
    ui.click([1215, 428], 30);
    ui.click(ui.button(2, 1), 30);
    assert_eq!(ui.run.audio().settings(), initial_audio);
    assert!(ui.run.buttons().group(2).is_none());
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    let initial_ducking = ui.run.audio().voice_ducking();
    for (index, y) in [508, 548, 588].into_iter().enumerate() {
        let before = ui.run.audio().voice_ducking();
        ui.click([900, y], 30);
        let after = ui.run.audio().voice_ducking();
        let values = |s: cmvs_rs::audio::VoiceDuckingSettings| [s.percent, s.fade_ms, s.wait_ms];
        assert_ne!(values(before)[index], values(after)[index]);
        for other in 0..3 {
            if other != index {
                assert_eq!(values(before)[other], values(after)[other]);
            }
        }
        assert_eq!(after.include_fading, before.include_fading);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    let before_ducking_reset = ui.run.audio().voice_ducking();
    ui.click([1215, 626], 30);
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.click(ui.button(2, 2), 30);
    assert_eq!(ui.run.audio().voice_ducking(), before_ducking_reset);
    ui.click([1215, 626], 30);
    ui.click(ui.button(2, 1), 30);
    assert_eq!(ui.run.audio().voice_ducking(), initial_ducking);
    assert!(ui.run.buttons().group(2).is_none());
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    for _ in 0..120 {
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        ui.frame([640, 360], None);
    }
    ui.click(ui.button(1, 3), 120);
    assert_eq!(ui.run.buttons().group(1).unwrap().buttons.len(), 107);
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    // Character page: five words per character from 2400 (volume, -, gradient, colors).
    let int = |ui: &Ui, index| ui.run.machine().state().integer(index);
    ui.click(ui.button(1, 42), 30);
    assert_eq!(int(&ui, 2402), Some(2));
    ui.click(ui.button(1, 53), 30);
    assert_eq!(int(&ui, 2400), Some(96));
    // Colour picker: 0x2034 reads the palette image to mark the current colour.
    ui.click(ui.button(1, 45), 30);
    assert!(ui.run.buttons().group(3).is_some());
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.click([640, 300], 30);
    ui.click(ui.button(3, 2), 30);
    assert!(ui.run.buttons().group(3).is_none());
    assert_eq!(int(&ui, 2403), Some(0x1effff));
    let lower = int(&ui, 2404);
    ui.click(ui.button(1, 47), 30);
    ui.click([760, 330], 30);
    ui.click(ui.button(3, 3), 30);
    assert_eq!(int(&ui, 2404), lower);
    ui.click(ui.button(1, 62), 30);
    assert_eq!(int(&ui, 2407), Some(2));
    ui.click(ui.button(1, 69), 30);
    ui.click(ui.button(2, 1), 30);
    assert_eq!(int(&ui, 2407), Some(1));
    let characters: Vec<_> = (2400..2450).map(|index| int(&ui, index)).collect();
    ui.click(ui.button(1, 16), 30);
    ui.click(ui.button(2, 2), 30);
    assert_eq!(
        (2400..2450)
            .map(|index| int(&ui, index))
            .collect::<Vec<_>>(),
        characters
    );
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.click(ui.button(1, 1), 120);
    assert_eq!(ui.run.buttons().group(1).unwrap().buttons.len(), 50);
    for (x, expected) in [(472, 0), (576, 1)] {
        ui.click([x, 239], 30);
        assert_eq!(ui.run.machine().state().integer(2197), Some(expected));
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (point, variable, expected, other) in
        [([300, 400], 2196, 1, 2195), ([500, 440], 2195, 6, 2196)]
    {
        let before = ui.run.machine().state().integer(other);
        ui.click(point, 30);
        assert_eq!(ui.run.machine().state().integer(variable), Some(expected));
        assert_eq!(ui.run.machine().state().integer(other), before);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (x, active) in [(472, false), (576, true)] {
        ui.click([x, 306], 30);
        assert_eq!(ui.run.settings().active_draw(), active);
        let steps = ui.run.steps();
        ui.run.set_focused(false);
        if active {
            ui.frame([-1, -1], None);
            assert!(ui.run.steps() > steps);
        } else {
            assert_eq!(
                ui.run.tick(&mut ui.backend).unwrap(),
                StartupEvent::Inactive
            );
            assert_eq!(ui.run.steps(), steps);
        }
        ui.run.set_focused(true);
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    for (x, fullscreen) in [(576, true), (472, false)] {
        ui.click([x, 107], 30);
        assert_eq!(
            cmvs_rs::platform::PlatformBackend::fullscreen(&ui.backend).unwrap(),
            fullscreen
        );
        if fullscreen {
            // 0x216e in fullscreen updates the future windowed size without
            // leaving fullscreen; repeating the reset sends no second request.
            ui.click([576, 626], 30);
            assert!(cmvs_rs::platform::PlatformBackend::fullscreen(&ui.backend).unwrap());
            assert_eq!(
                cmvs_rs::platform::PlatformBackend::windowed_size(&ui.backend).unwrap(),
                [1280, 720]
            );
            let requests = ui.backend.requests().len();
            ui.click([576, 626], 30);
            assert_eq!(ui.backend.requests().len(), requests);
            for (mode, x) in [904, 1008, 1112, 1216].into_iter().enumerate() {
                ui.click([x, 137], 30);
                assert_eq!(
                    cmvs_rs::platform::PlatformBackend::aspect_mode(&ui.backend).unwrap(),
                    mode as u32
                );
                let image = font
                    .render(ui.run.scene(), ui.run.camera(), ui.run.messages())
                    .unwrap();
                image
                    .draw_aspect(&mut vec![0; 1000 * 1000], 1000, 1000, mode as u32)
                    .unwrap();
            }
        }
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
    }
    // Draw-mode group 22: only modes 0/1 exist without pixel shaders (0x216b = 3).
    for (x, mode) in [(264, 0), (368, 1), (264, 0)] {
        ui.click([x, 560], 30);
        assert_eq!(
            cmvs_rs::platform::PlatformBackend::render_mode(&ui.backend).unwrap(),
            mode
        );
        let image = font
            .render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        image
            .draw_scaled(&mut vec![0; 1000 * 1000], 1000, 1000, 1, mode == 1)
            .unwrap();
    }
    for _ in 0..120 {
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        ui.frame([640, 360], None);
    }
    ui.click([576, 626], 30);
    assert_eq!(
        cmvs_rs::platform::PlatformBackend::window_size(&ui.backend).unwrap(),
        [1280, 720]
    );
    font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
        .unwrap();
    ui.press([640, 360], 3, 120);
    ui.wait_for_text();
    assert!(ui.run.buttons().group(1).is_none());
    assert_eq!(ui.run.machine().state().integer(6), Some(1));
    assert_eq!(ui.run.machine().state().integer(2195), Some(6));
    let before_auto = ui.run.history().entries().len();
    ui.voice_fades.clear();
    ui.voices.clear();
    ui.voice_volumes.clear();
    ui.click(ui.toolbar(5), 0);
    assert!(ui.run.message_flags().auto_advance());
    let mut auto_frames = 0;
    let mut first_voice_history = None;
    let mut character_glyphs = None;
    for _ in 0..8000 {
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        auto_frames += 1;
        ui.frame([640, 360], None);
        if !ui.voices.is_empty() && first_voice_history.is_none() {
            first_voice_history = Some(ui.run.history().entries().len());
        }
        if character_glyphs.is_none() && !ui.voices.is_empty() {
            character_glyphs = (0..cmvs_rs::message::TEXT_OBJECTS)
                .filter_map(|slot| ui.run.messages().text(slot))
                .flat_map(|text| &text.glyphs)
                .find(|glyph| glyph.colors[0] == 0x1effff)
                .map(|glyph| (glyph.colors, glyph.gradient));
        }
        if ui.run.history().entries().len() >= before_auto + 3
            && first_voice_history
                .is_some_and(|count| ui.run.history().entries().len() >= count + 2)
        {
            break;
        }
    }
    assert!(ui.run.history().entries().len() >= before_auto + 3);
    assert!(
        first_voice_history.is_some_and(|count| ui.run.history().entries().len() >= count + 2),
        "auto must advance beyond actual voice playback"
    );
    // The first voice belongs to こころ, whose settings were changed above.
    assert!(ui.voices[0].starts_with(b"co"));
    assert_eq!(ui.voice_volumes[0], 96);
    assert_eq!(ui.voice_rates[0], 1.5);
    assert_eq!(
        character_glyphs,
        Some(([0x1effff, lower.unwrap() as u32], 2)),
        "character text colour and gradient reach the dialogue glyphs"
    );
    assert!(
        !ui.voice_fades.is_empty(),
        "fade-out stop mode must execute voice fades"
    );
    assert!(ui.voice_fades.iter().all(|&(channel, _)| channel < 4));
    ui.click(ui.toolbar(5), 0);
    assert!(!ui.run.message_flags().auto_advance());
    ui.wait_for_text();
    let stopped = ui.run.history().entries().len();
    for _ in 0..240 {
        font.render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        ui.frame([640, 360], None);
    }
    assert_eq!(ui.run.history().entries().len(), stopped);
    assert!(ui.waiting_for_text());
    let panels: Vec<_> = ui
        .run
        .messages()
        .window(0)
        .unwrap()
        .object
        .children
        .range(64..=96)
        .filter(|(_, child)| child.shown)
        .map(|(_, child)| child)
        .collect();
    assert!(!panels.is_empty());
    assert!(ui.run.messages().window(0).unwrap().object.image.is_some());
    assert!(
        panels
            .iter()
            .all(|child| child.draw.as_ref().unwrap().words[17] == 32)
    );
    println!(
        "settings: native_audio={native_audio}; title + toolbar; background/opacity/fullscreen/aspect/filter, cursor hide, voice stop modes, read-skip policy, bold/text/audio/auto settings, character volume/gradient/colour picker/reset, 1.5x voice speed, font dialog, three pages, 444 settings frames; auto advanced at least three entries in {auto_frames} rendered frames with {} voices and {} voice fades; disabled then idle 240 rendered frames",
        ui.voices.len(),
        ui.voice_fades.len()
    );
    let settings = ui.run.settings().clone();
    let audio = ui.run.audio().settings();
    let ducking = ui.run.audio().voice_ducking();
    let voice_mode = ui.run.message_flags().mode_1548();
    let persistent = [2195, 2196, 2197].map(|index| ui.run.machine().state().integer(index));
    let characters: Vec<_> = (2400..2450)
        .map(|index| ui.run.machine().state().integer(index))
        .collect();
    ui.run
        .save_preferences(cmvs_rs::settings::DisplayPreferences {
            fullscreen: false,
            aspect_mode: 3,
            render_mode: cmvs_rs::platform::PlatformBackend::render_mode(&ui.backend).unwrap(),
        })
        .unwrap();
    drop(ui);
    drop(run);
    let mut resumed = Startup::new(&script, &root, &game, 0).unwrap();
    resumed.set_user_data_directory(&user).unwrap();
    assert_eq!(resumed.settings(), &settings);
    assert_eq!(resumed.audio().settings(), audio);
    assert_eq!(resumed.audio().voice_ducking(), ducking);
    assert_eq!(resumed.message_flags().mode_1548(), voice_mode);
    assert_eq!(
        resumed.display_preferences(),
        cmvs_rs::settings::DisplayPreferences {
            fullscreen: false,
            aspect_mode: 3,
            render_mode: 0
        }
    );
    let mut backend = RecordingBackend::default();
    let mut loaded = false;
    for _ in 0..100_000 {
        let vm = resumed.machine();
        let reading = vm
            .loaded_script(vm.current_slot())
            .and_then(|s| s.code().get(vm.pc()..vm.pc() + 2))
            == Some(0x2128u16.to_le_bytes().as_slice());
        let event = resumed.tick(&mut backend).unwrap();
        if reading {
            loaded = true;
            break;
        }
        if event == StartupEvent::Yield {
            resumed.advance_frame(16).unwrap();
        }
        assert!(matches!(
            event,
            StartupEvent::Progress | StartupEvent::Yield
        ));
    }
    assert!(loaded);
    assert_eq!(
        [2195, 2196, 2197].map(|index| resumed.machine().state().integer(index)),
        persistent
    );
    assert_eq!(
        (2400..2450)
            .map(|index| resumed.machine().state().integer(index))
            .collect::<Vec<_>>(),
        characters
    );
    println!(
        "settings: saved user INI + CSS1 and reloaded runtime settings and script variables on restart"
    );
}

impl Ui<'_, '_> {
    fn frame(&mut self, pointer: [i32; 2], code: Option<usize>) {
        if self.frozen {
            return;
        }
        loop {
            let pc = self.run.machine().pc();
            let opcode = self
                .run
                .machine()
                .loaded_script(self.run.machine().current_slot())
                .and_then(|s| s.code().get(pc..pc + 2))
                .map(|b| u16::from_le_bytes([b[0], b[1]]));
            let voice = matches!(opcode, Some(0x20a6 | 0x20af))
                .then(|| self.run.machine().stack().peek_u32(8).ok())
                .flatten()
                .and_then(|raw| self.run.machine().resolve_string(raw).ok());
            let voice_volume = self.run.machine().stack().peek_u32(12).ok();
            let voice_rate = if opcode == Some(0x20af) {
                self.run
                    .machine()
                    .stack()
                    .peek_u32(20)
                    .ok()
                    .map(f32::from_bits)
            } else {
                Some(1.0)
            };
            let voice_fade = (opcode == Some(0x20a9))
                .then(|| {
                    self.run
                        .machine()
                        .stack()
                        .peek_u32(4)
                        .ok()
                        .zip(self.run.machine().stack().peek_u32(8).ok())
                })
                .flatten();
            let event = self.run.tick(&mut self.backend).unwrap_or_else(|e| {
                panic!(
                    "{e} at {} / 0x{pc:x}",
                    self.run.machine().current_name().escape_ascii()
                )
            });
            if let Some(op @ (0x22b5 | 0x22b6)) = opcode
                && self.run.machine().pending_system().is_none()
            {
                self.commands.push(op);
                if op == 0x22b5 {
                    self.frozen = true;
                    return;
                }
            }
            if let Some(name) = voice
                && self.run.machine().pending_system().is_none()
            {
                self.voices.push(name);
                self.voice_volumes
                    .push(voice_volume.expect("0x20a6 has four arguments") as i32);
                self.voice_rates
                    .push(voice_rate.expect("0x20af has five arguments"));
            }
            if let Some(fade) = voice_fade
                && self.run.machine().pending_system().is_none()
            {
                self.voice_fades.push(fade);
            }
            match event {
                StartupEvent::Yield => break,
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                other => panic!("{other:?}"),
            }
        }
        if self.native_audio {
            std::thread::sleep(std::time::Duration::from_millis(16));
            self.run.advance_decoded_frame(16).unwrap();
        } else {
            self.run.advance_frame(16).unwrap();
        }
        let mut now = [false; 128];
        if let Some(code) = code {
            now[code] = true;
        }
        self.codes
            .update(&self.bindings, &now, self.run.input_mut());
        self.run.input_mut().set_pointer(pointer[0], pointer[1]);
    }
    /// Moves there, presses for three frames, releases and lets `wait` frames pass.
    fn press(&mut self, at: [i32; 2], code: usize, wait: u32) {
        for i in 0..(8 + wait) {
            self.frame(at, (3..6).contains(&i).then_some(code));
        }
    }
    fn click(&mut self, at: [i32; 2], wait: u32) {
        self.press(at, 1, wait);
    }
    /// Presses at `from`, moves there while holding over 20 frames, releases at `to`.
    #[cfg(feature = "desktop")]
    fn drag(&mut self, from: [i32; 2], to: [i32; 2]) {
        for i in 0..4 {
            self.frame(from, (i >= 2).then_some(1));
        }
        for step in 0..=20 {
            let at = [
                from[0] + (to[0] - from[0]) * step / 20,
                from[1] + (to[1] - from[1]) * step / 20,
            ];
            self.frame(at, Some(1));
        }
        for _ in 0..30 {
            self.frame(to, None);
        }
    }
    fn toolbar(&self, button: usize) -> [i32; 2] {
        let window = self.run.messages().window(7).expect("toolbar window");
        let [x, y, w, h] = window.buttons[button].hit.map(i32::from);
        [
            window.position[0] + x + w / 2,
            window.position[1] + y + h / 2,
        ]
    }
    fn button(&self, group: usize, id: u32) -> [i32; 2] {
        let b = self
            .run
            .buttons()
            .group(group)
            .and_then(|g| g.buttons.iter().find(|b| b.id == id))
            .unwrap_or_else(|| panic!("button {id} of group {group}"));
        [b.rect[0] + b.rect[2] / 2, b.rect[1] + b.rect[3] / 2]
    }
    fn waiting_for_text(&self) -> bool {
        matches!(self.run.machine().pending_system(), Some((_, 0x153)))
            && self.run.machine().current_name() == b"sn1001.ps3"
    }
    fn wait_for_text(&mut self) {
        for _ in 0..2000 {
            if self.waiting_for_text() && self.run.machine().repeat_count() >= 2 {
                return;
            }
            self.frame([640, 360], None);
        }
        panic!("no message wait");
    }
}

/// The in-game toolbar and menus save and load through the automatic callbacks: quick
/// save (slot 860) and quick load, then the save screen (slot 004 = save003) and the load
/// screen (slot 001 = save000). Each load must leave the hosts equal to the slot's snapshot.
#[test]
#[ignore = "requires CMVS_GAME_ROOT with original system.dat and save000..002 in save/"]
fn external_in_game_menus_save_and_load() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let user = user_directory("menus");
    for name in ["system.dat", "save000.dat", "save001.dat", "save002.dat"] {
        fs::copy(root.join("save").join(name), user.join(name)).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    let mut ui = Ui {
        native_audio: false,
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
        voice_volumes: Vec::new(),
        voice_rates: Vec::new(),
        voice_fades: Vec::new(),
        frozen: false,
    };
    // Title menu idle, then the first slot, as in the auto-resume test.
    while !(ui.run.machine().current_name() == b"menu.ps3" && ui.run.steps() > 200_000) {
        ui.frame([-1, -1], None);
    }
    ui.run.load_slot(0, &mut ui.backend).unwrap();
    ui.wait_for_text();

    // Capture visible real-script glyphs between VM commands, then restore the
    // exact snapshot through the normal slot loader. Inputs remain read-only.
    let visible = ui.run.inspect_snapshot().unwrap();
    #[cfg(feature = "desktop")]
    let mut visual = std::env::var_os("CMVS_FONT_PATH").map(|path| {
        let mut font =
            cmvs_rs::desktop::font::FontRenderer::read(std::path::Path::new(&path), 0).unwrap();
        let frame = font
            .render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        (font, frame)
    });
    let bytes = visible.encode().unwrap();
    assert_eq!(
        EngineSnapshot::decode(&bytes).unwrap().encode().unwrap(),
        bytes
    );
    let source = fs::read(user.join("save000.dat")).unwrap();
    let vm = ui.run.machine().capture_vm().unwrap();
    let csv = Csv2Save::encode(
        source[..600].try_into().unwrap(),
        &bytes,
        &[],
        &vm.main_image,
    )
    .unwrap();
    fs::write(user.join("save005.dat"), &csv).unwrap();
    let mut count = 0;
    let mut manifest = String::new();
    for (slot, text) in visible.texts.iter().enumerate() {
        for glyph in text.iter().flat_map(|t| &t.glyphs) {
            count += 1;
            manifest.push_str(&format!(
                "{slot} {} {} {} {} {} {} {} {} {}\n",
                glyph.code,
                glyph.position[0],
                glyph.position[1],
                glyph.size,
                glyph.gradient,
                glyph.colors[0],
                glyph.colors[1],
                glyph.outline[0],
                glyph.outline[1]
            ));
        }
    }
    assert!(count > 0);
    if let Some(directory) = std::env::var_os("CMVS_GLYPH_OUTPUT") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("visible.dat"), csv).unwrap();
        fs::write(directory.join("glyphs.txt"), manifest).unwrap();
    }
    ui.run.load_slot(5, &mut ui.backend).unwrap();
    #[cfg(feature = "desktop")]
    if let Some((font, before)) = &mut visual {
        let after = font
            .render(ui.run.scene(), ui.run.camera(), ui.run.messages())
            .unwrap();
        assert_eq!(
            after, *before,
            "visible glyph snapshot changed rendered pixels"
        );
        println!("visible snapshot: rendered pixels identical");
    }
    for (slot, expected) in visible.texts.iter().enumerate() {
        if let Some(expected) = expected {
            let actual = ui.run.messages().text(slot).unwrap();
            assert_eq!(actual.glyphs.len(), expected.glyphs.len());
            for (a, e) in actual.glyphs.iter().zip(&expected.glyphs) {
                assert_eq!(
                    (a.code, a.position, a.size, a.colors, a.gradient, a.outline),
                    (e.code, e.position, e.size, e.colors, e.gradient, e.outline)
                );
                assert!(a.shown && !a.active && a.opacity == 255);
            }
        }
    }
    println!("visible snapshot: {count} glyphs restored");
    ui.wait_for_text();

    // Quick save: toolbar button 3, then yes.
    ui.click(ui.toolbar(3), 30);
    ui.click(ui.button(2, 1), 60);
    assert_eq!(ui.commands, [0x22b6]);
    let quick = fs::read(user.join("save860.dat")).unwrap();
    let quick = EngineSnapshot::decode(&Csv2Save::decode(&quick).unwrap().snapshot).unwrap();
    ui.wait_for_text();

    // Quick load: toolbar button 4, then yes; the hosts equal the quick save.
    ui.commands.clear();
    ui.click(ui.toolbar(4), 30);
    ui.click(ui.button(2, 1), 0);
    for _ in 0..120 {
        if ui.commands.contains(&0x22b5) {
            break;
        }
        ui.frame([640, 360], None);
    }
    assert_eq!(ui.commands, [0x22b5]);
    check_loaded(ui.run, quick);
    ui.frozen = false;
    ui.wait_for_text();

    // Save screen: toolbar button 1, slot 004 (id 73), yes.
    ui.commands.clear();
    ui.click(ui.toolbar(1), 60);
    // The save screen contains anonymous images decoded from real CSV2 thumbnails.
    // They have no pixel payload in a native scene-image record.
    let menu_snapshot = ui.run.inspect_snapshot().unwrap();
    fn anonymous(scene: &SavedScene) -> usize {
        [&scene.image, &scene.secondary_image]
            .into_iter()
            .flatten()
            .filter(|i| i.surface.is_none() && i.name.is_empty())
            .count()
            + scene.children.values().map(anonymous).sum::<usize>()
    }
    let anonymous_count: usize = menu_snapshot.objects.values().map(anonymous).sum();
    assert!(anonymous_count > 0);
    let menu_bytes = menu_snapshot.encode().unwrap();
    assert_eq!(
        EngineSnapshot::decode(&menu_bytes)
            .unwrap()
            .encode()
            .unwrap(),
        menu_bytes
    );
    let image = ui.run.machine().capture_vm().unwrap().main_image;
    let menu_file =
        Csv2Save::encode(source[..600].try_into().unwrap(), &menu_bytes, &[], &image).unwrap();
    fs::write(user.join("save006.dat"), &menu_file).unwrap();
    if let Some(directory) = std::env::var_os("CMVS_MEMORY_IMAGE_OUTPUT") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("menu.dat"), menu_file).unwrap();
        fs::write(directory.join("count.txt"), anonymous_count.to_string()).unwrap();
    }
    ui.click(ui.button(1, 73), 60);
    ui.click(ui.button(2, 1), 60);
    assert_eq!(ui.commands, [0x22b6]);
    let saved = fs::read(user.join("save003.dat")).unwrap();
    EngineSnapshot::decode(&Csv2Save::decode(&saved).unwrap().snapshot).unwrap();
    // Back (id 5) to the story.
    ui.click(ui.button(1, 5), 0);
    ui.wait_for_text();

    // Load screen: toolbar button 2, slot 001 (id 70), yes; the hosts equal save000.
    ui.commands.clear();
    ui.click(ui.toolbar(2), 60);
    ui.click(ui.button(1, 70), 60);
    ui.click(ui.button(2, 1), 0);
    for _ in 0..200 {
        if ui.commands.contains(&0x22b5) {
            break;
        }
        ui.frame([640, 360], None);
    }
    assert_eq!(ui.commands, [0x22b5]);
    check_loaded(ui.run, original_snapshot(&root, 0));
    ui.run.load_slot(6, &mut ui.backend).unwrap();
    fn black_images(object: &cmvs_rs::scene::SceneObject) -> usize {
        let mut count = 0;
        for image in [&object.image, &object.secondary_image]
            .into_iter()
            .flatten()
        {
            if !image.surface && image.name.is_empty() {
                assert!(image.alpha);
                assert!(
                    image
                        .pixels
                        .pixels()
                        .chunks_exact(4)
                        .all(|p| p == [0, 0, 0, 255])
                );
                count += 1;
            }
        }
        count + object.children.values().map(black_images).sum::<usize>()
    }
    assert_eq!(
        ui.run
            .scene()
            .objects()
            .values()
            .map(black_images)
            .sum::<usize>(),
        anonymous_count
    );
    println!("save menu snapshot: {anonymous_count} anonymous thumbnail images rebuilt black");
    drop(ui);
    fs::remove_dir_all(user).unwrap();
}

/// Shift-JIS pairs of a history text as the backlog shows it: escapes dropped and ruby groups
/// reduced to their base. Derived from the text alone, independently of the layout.
fn backlog_codes(text: &[u8]) -> Vec<u16> {
    let lead = |b: u8| (0x81..=0x9f).contains(&b) || (0xe0..=0xfc).contains(&b);
    let (mut codes, mut i, mut ruby) = (Vec::new(), 0, false);
    while i < text.len() {
        let b = text[i];
        if ruby {
            i += if lead(b) { 2 } else { 1 };
            ruby = b != b'}';
        } else if b == b'\\' {
            i += if text.get(i + 1) == Some(&b'w') { 3 } else { 2 };
        } else if b == b'{' || b == b'/' {
            // The base follows `{`; the ruby text follows `/` up to `}`.
            ruby = b == b'/';
            i += 1;
        } else if lead(b) {
            codes.push(u16::from_be_bytes([b, text[i + 1]]));
            i += 2;
        } else {
            panic!("half-width history text");
        }
    }
    codes
}

/// A glyph has a cell exactly when its padded cell meets the bounds at the group position.
fn check_attached(group: &AuxiliaryText) {
    let float = |at| f32::from_bits(group.word(at).unwrap());
    let ([left, top, width, height], origin) = ([32, 36, 40, 44].map(float), [8, 12].map(float));
    let mut attached = 0;
    for (i, glyph) in group.glyphs.iter().enumerate() {
        let [x, y] = [0, 1].map(|axis| glyph.position[axis] as f32 + origin[axis]);
        let size = glyph.size as f32;
        let inside = !(x - 2.0 > left + width
            || left > x + size + 2.0
            || y - 2.0 > top + height
            || top > y + size + 2.0);
        assert_eq!(glyph.cell != 0, inside, "glyph {i}");
        attached += usize::from(inside);
    }
    assert!(attached > 0);
    assert_eq!(group.attached_glyphs().count(), attached);
}

/// The backlog (wheel up at a story text wait) lays out every history entry, oldest first, in
/// auxiliary text group 0 and attaches the glyphs inside its bounds. Its list group 2 holds a
/// voice button (192 + index) for each voiced entry and a jump target (320 + index) for each
/// entry, relative to the list's object. It scrolls one entry per wheel step, replays voices,
/// jumps back to an entry after the confirmation, and closes on right click.
#[test]
#[ignore = "requires CMVS_GAME_ROOT with original system.dat and save870 in save/"]
fn external_backlog_scrolls_replays_voices_and_jumps_back() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let user = user_directory("backlog");
    for name in ["system.dat", "save870.dat"] {
        fs::copy(root.join("save").join(name), user.join(name)).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    let mut ui = Ui {
        native_audio: false,
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
        voice_volumes: Vec::new(),
        voice_rates: Vec::new(),
        voice_fades: Vec::new(),
        frozen: false,
    };
    while !(ui.run.machine().current_name() == b"menu.ps3" && ui.run.steps() > 200_000) {
        ui.frame([-1, -1], None);
    }
    ui.run.load_slot(870, &mut ui.backend).unwrap();
    ui.wait_for_text();
    let history: Vec<HistoryEntry> = ui.run.history().entries().iter().rev().cloned().collect();
    let open = |ui: &mut Ui<'_, '_>| {
        ui.press([640, 360], 4, 0);
        for _ in 0..120 {
            ui.frame([640, 360], None);
        }
        ui.run
            .messages()
            .auxiliary_text(0)
            .expect("backlog text group")
            .clone()
    };
    let list = |ui: &Ui<'_, '_>| {
        ui.run
            .buttons()
            .list_group(2)
            .expect("backlog list")
            .clone()
    };
    let group = open(&mut ui);
    let codes: Vec<u16> = group.glyphs.iter().map(|g| g.code).collect();
    let expected: Vec<u16> = history
        .iter()
        .flat_map(|e| backlog_codes(&e.strings[2]))
        .collect();
    assert_eq!(codes, expected);
    check_attached(&group);
    let entries = list(&ui);
    let jumps: Vec<u32> = entries
        .buttons
        .iter()
        .map(|b| b.id)
        .filter(|&id| id >= 320)
        .collect();
    assert_eq!(jumps, (320..320 + history.len() as u32).collect::<Vec<_>>());
    let voiced: Vec<u32> = (0..history.len() as u32)
        .filter(|&i| !history[i as usize].strings[1].is_empty())
        .map(|i| 192 + i)
        .collect();
    assert!(!voiced.is_empty());
    assert_eq!(
        entries
            .buttons
            .iter()
            .map(|b| b.id)
            .filter(|&id| id < 320)
            .collect::<Vec<_>>(),
        voiced
    );

    // One wheel step scrolls one 106-pixel entry and back.
    let y = |ui: &Ui<'_, '_>| {
        f32::from_bits(
            ui.run
                .messages()
                .auxiliary_text(0)
                .unwrap()
                .word(12)
                .unwrap(),
        )
    };
    let bottom = y(&ui);
    ui.press([640, 360], 4, 60);
    assert_eq!(y(&ui), bottom + 106.0);
    check_attached(ui.run.messages().auxiliary_text(0).unwrap());
    ui.press([640, 360], 5, 60);
    assert_eq!(y(&ui), bottom);

    // Screen centres of list buttons, relative to the list's object, fully inside its area.
    let visible = |ui: &Ui<'_, '_>, ids: std::ops::Range<u32>| {
        let list = list(ui);
        let base = ui
            .run
            .scene()
            .object(list.object)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .words[9] as i32;
        let [_, top, _, height] = list.hit_area;
        list.buttons
            .iter()
            .filter(|b| ids.contains(&b.id))
            .map(|b| (b.id, b.rect[0] + b.rect[2] / 2, b.rect[1] + base, b.rect[3]))
            .filter(|&(_, _, y, h)| y >= top && y + h <= top + height)
            .map(|(id, x, y, h)| (id, [x, y + h / 2]))
            .collect::<Vec<_>>()
    };
    // Voice replay of the last visible voiced entry.
    let (id, at) = *visible(&ui, 192..320).last().expect("visible voice button");
    ui.press(at, 1, 60);
    assert_eq!(ui.voices, [history[id as usize - 192].strings[1].clone()]);
    assert!(ui.run.messages().auxiliary_text(0).is_some());

    // Jump back to the first fully visible older entry. The scene replays from its start, so
    // the rebuilt history ends with the saved entries up to that one.
    let (id, at) = visible(&ui, 320..448)[0];
    let index = id as usize - 320;
    assert!(index + 1 < history.len());
    ui.click(at, 60);
    ui.click(ui.button(2, 1), 0);
    ui.wait_for_text();
    assert!(ui.run.messages().auxiliary_text(0).is_none());
    let after: Vec<HistoryEntry> = ui.run.history().entries().iter().rev().cloned().collect();
    assert!(after.ends_with(&history[..=index]));
    println!(
        "backlog: {} entries, {} glyphs, voice {:?}, jump to entry {index} -> {} entries",
        history.len(),
        expected.len(),
        ui.voices[0].escape_ascii().to_string(),
        after.len()
    );

    // Reopened on the shorter history, it closes on right click without changes.
    let group = open(&mut ui);
    assert_eq!(
        group.glyphs.iter().map(|g| g.code).collect::<Vec<_>>(),
        after
            .iter()
            .flat_map(|e| backlog_codes(&e.strings[2]))
            .collect::<Vec<_>>()
    );
    ui.press([640, 360], 3, 0);
    ui.wait_for_text();
    assert!(ui.run.messages().auxiliary_text(0).is_none());
    assert_eq!(ui.run.history().entries().len(), after.len());
    drop(ui);
    fs::remove_dir_all(user).unwrap();
}

#[test]
#[ignore = "requires CMVS_GAME_ROOT; original save000 resources with an injected rotation track"]
fn external_rotation_snapshot_resumes_on_original_resources() {
    use cmvs_rs::scene::{RotationKey, RotationTrack};
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let user = user_directory("rotation-snapshot");
    fs::copy(root.join("save/system.dat"), user.join("system.dat")).unwrap();
    let source = fs::read(root.join("save/save000.dat")).unwrap();
    let save = Csv2Save::decode(&source).unwrap();
    let mut snapshot = EngineSnapshot::decode(&save.snapshot).unwrap();
    let (&slot, object) = snapshot
        .objects
        .iter_mut()
        .find(|(_, o)| o.image.is_some() && o.draw.is_some() && o.children.is_empty())
        .expect("a named real image");
    let draw = object.draw.as_mut().unwrap();
    draw.words[20] = 2250;
    let mut keys = [RotationKey::default(); 8];
    keys[0] = RotationKey {
        next: -1,
        value: 9000,
        mode: 0,
        duration: 400,
    };
    draw.rotation_track = Some(Box::new(RotationTrack {
        playing: true,
        base: 0,
        start: 0,
        current: 0,
        elapsed: 100,
        keys,
    }));
    let raw = Csv2Save::encode(
        source[..600].try_into().unwrap(),
        &snapshot.encode().unwrap(),
        &save.auxiliary,
        &save.script_image,
    )
    .unwrap();
    fs::write(user.join("save005.dat"), &raw).unwrap();
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_skip_auto_resume(true).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..2_000_000 {
        match run.tick(&mut backend).unwrap() {
            StartupEvent::Yield => {
                run.advance_frame(16).unwrap();
                if run.machine().current_name() == b"menu.ps3" && run.steps() > 200_000 {
                    break;
                }
            }
            StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
            other => panic!("{other:?}"),
        }
    }
    run.load_slot(5, &mut backend).unwrap();
    let angle = |run: &Startup<'_>| {
        run.scene()
            .object(slot)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .words[20]
    };
    assert_eq!(angle(&run), 2250);
    assert_eq!(
        run.scene()
            .object(slot)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .rotation_track,
        snapshot.objects[&slot]
            .draw
            .as_ref()
            .unwrap()
            .rotation_track
    );
    let mut angles = vec![2250];
    for expected in [4500, 6750, 9000] {
        run.advance_frame(100).unwrap();
        assert_eq!(angle(&run), expected);
        angles.push(expected);
    }
    assert!(
        run.scene()
            .object(slot)
            .unwrap()
            .draw
            .as_ref()
            .unwrap()
            .rotation_track
            .is_none()
    );
    if let Some(directory) = std::env::var_os("CMVS_ROTATION_OUTPUT") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("rotation.dat"), raw).unwrap();
        fs::write(
            directory.join("angles.txt"),
            angles
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
    println!("rotation on real image slot {slot}: {angles:?}");
    drop(run);
    fs::remove_dir_all(user).unwrap();
}

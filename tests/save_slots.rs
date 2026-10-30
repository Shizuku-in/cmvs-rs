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
            run.set_audio_backend(Box::new(cmvs_rs::audio::RecordingAudio::default()));
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
    run: &'b mut Startup<'a>,
    backend: RecordingBackend,
    bindings: KeyBindings,
    codes: CodeInput,
    /// Save commands completed (0x22b6 / 0x22b5) since the last check.
    commands: Vec<u16>,
    /// Voice files 0x20a6 played (SP-8) since the last check.
    voices: Vec<Vec<u8>>,
    /// Set when a load completes; frames do nothing until the test has checked the hosts.
    frozen: bool,
}

/// First settings milestone: the toolbar opens the original scripted settings and
/// right click returns to gameplay. Rendering, edits and persistence are separate checks.
#[cfg(feature = "desktop")]
#[test]
#[ignore = "requires CMVS_GAME_ROOT with system.dat/save000 and CMVS_FONT_PATH"]
fn external_system_settings_toolbar_opens_and_returns_to_text() {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
    let font_path = PathBuf::from(std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH"));
    let font = cmvs_rs::desktop::font::FontRenderer::read(&font_path, 0).unwrap();
    let user = user_directory("settings");
    for name in ["system.dat", "save000.dat"] {
        fs::copy(root.join("save").join(name), user.join(name)).unwrap();
    }
    let game = Registry::builtin().identify_game(&root, None).unwrap();
    let script = load_start(&root, &game.files).unwrap();
    let mut run = Startup::new(&script, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    run.set_record_text(true);
    run.set_glyph_metrics(font.metrics());
    let mut ui = Ui {
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
        frozen: false,
    };
    for _ in 0..30_000 {
        if ui.run.machine().current_name() == b"menu.ps3" && ui.run.steps() > 200_000 {
            break;
        }
        ui.frame([-1, -1], None);
    }
    assert_eq!(ui.run.machine().current_name(), b"menu.ps3");
    ui.run.load_slot(0, &mut ui.backend).unwrap();
    ui.wait_for_text();
    ui.click(ui.toolbar(0), 120);
    let group = ui.run.buttons().group(1).unwrap();
    assert_eq!(group.object, 64);
    assert_eq!(group.buttons.len(), 292);
    assert!(ui.run.messages().window(6).is_some());
    ui.press([640, 360], 3, 120);
    ui.wait_for_text();
    assert!(ui.run.buttons().group(1).is_none());
    println!("settings: 292 buttons; toolbar open, idle, right-click close and gameplay resume");
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
            let voice = (opcode == Some(0x20a6))
                .then(|| self.run.machine().stack().peek_u32(8).ok())
                .flatten()
                .and_then(|raw| self.run.machine().resolve_string(raw).ok());
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
            }
            match event {
                StartupEvent::Yield => break,
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                other => panic!("{other:?}"),
            }
        }
        self.run.advance_frame(16).unwrap();
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
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
        frozen: false,
    };
    // Title menu idle, then the first slot, as in the auto-resume test.
    while !(ui.run.machine().current_name() == b"menu.ps3" && ui.run.steps() > 200_000) {
        ui.frame([-1, -1], None);
    }
    ui.run.load_slot(0, &mut ui.backend).unwrap();
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
        run: &mut run,
        backend: RecordingBackend::default(),
        bindings: KeyBindings::parse(&fs::read_to_string(root.join("key.cfg")).unwrap()).unwrap(),
        codes: CodeInput::default(),
        commands: Vec::new(),
        voices: Vec::new(),
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

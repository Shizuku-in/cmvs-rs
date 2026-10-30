//! Gameplay save slots through `Startup` on the original game: external saves and route
//! replays only, read-only inputs. Synthetic save tests live in tests/runner.rs.
use cmvs_rs::{
    input::{CodeInput, KeyBindings},
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

/// Drives the game's own save UI: the toolbar (message window 7, run by the automatic
/// callbacks), its confirmation dialog (button group 2, id 1 = yes), the save and load
/// screens (button group 1, slot ids from 70). Clicks go through key.cfg bindings.
struct Ui<'a, 'b> {
    run: &'b mut Startup<'a>,
    backend: RecordingBackend,
    bindings: KeyBindings,
    codes: CodeInput,
    /// Save commands completed (0x22b6 / 0x22b5) since the last check.
    commands: Vec<u16>,
    /// Set when a load completes; frames do nothing until the test has checked the hosts.
    frozen: bool,
}

impl Ui<'_, '_> {
    fn frame(&mut self, pointer: [i32; 2], click: bool) {
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
            match event {
                StartupEvent::Yield => break,
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                other => panic!("{other:?}"),
            }
        }
        self.run.advance_frame(16).unwrap();
        let mut now = [false; 128];
        now[1] = click;
        self.codes
            .update(&self.bindings, &now, self.run.input_mut());
        self.run.input_mut().set_pointer(pointer[0], pointer[1]);
    }
    /// Moves there, presses for three frames, releases and lets `wait` frames pass.
    fn click(&mut self, at: [i32; 2], wait: u32) {
        for i in 0..(8 + wait) {
            self.frame(at, (3..6).contains(&i));
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
        frozen: false,
    };
    // Title menu idle, then the first slot, as in the auto-resume test.
    while !(ui.run.machine().current_name() == b"menu.ps3" && ui.run.steps() > 200_000) {
        ui.frame([-1, -1], false);
    }
    ui.run.load_slot(0, &mut ui.backend).unwrap();
    let wait_for_text = |ui: &mut Ui<'_, '_>| {
        for _ in 0..2000 {
            if ui.waiting_for_text() && ui.run.machine().repeat_count() >= 2 {
                return;
            }
            ui.frame([640, 360], false);
        }
        panic!("no message wait");
    };
    wait_for_text(&mut ui);

    // Quick save: toolbar button 3, then yes.
    ui.click(ui.toolbar(3), 30);
    ui.click(ui.button(2, 1), 60);
    assert_eq!(ui.commands, [0x22b6]);
    let quick = fs::read(user.join("save860.dat")).unwrap();
    let quick = EngineSnapshot::decode(&Csv2Save::decode(&quick).unwrap().snapshot).unwrap();
    wait_for_text(&mut ui);

    // Quick load: toolbar button 4, then yes; the hosts equal the quick save.
    ui.commands.clear();
    ui.click(ui.toolbar(4), 30);
    ui.click(ui.button(2, 1), 0);
    for _ in 0..120 {
        if ui.commands.contains(&0x22b5) {
            break;
        }
        ui.frame([640, 360], false);
    }
    assert_eq!(ui.commands, [0x22b5]);
    check_loaded(ui.run, quick);
    ui.frozen = false;
    wait_for_text(&mut ui);

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
    wait_for_text(&mut ui);

    // Load screen: toolbar button 2, slot 001 (id 70), yes; the hosts equal save000.
    ui.commands.clear();
    ui.click(ui.toolbar(2), 60);
    ui.click(ui.button(1, 70), 60);
    ui.click(ui.button(2, 1), 0);
    for _ in 0..200 {
        if ui.commands.contains(&0x22b5) {
            break;
        }
        ui.frame([640, 360], false);
    }
    assert_eq!(ui.commands, [0x22b5]);
    check_loaded(ui.run, original_snapshot(&root, 0));
    drop(ui);
    fs::remove_dir_all(user).unwrap();
}

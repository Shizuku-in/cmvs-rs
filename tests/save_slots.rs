//! Gameplay save slots through `Startup`: external original saves only, read-only inputs.
use cmvs_rs::{
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
    println!("differences: {differences:?}");
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

use super::*;

#[test]
fn immediate_readback_retries_until_a_draw_and_ignores_unflushed_scene_changes() {
    use cmvs_core::render::raster::RgbaImage;
    let mut code = vec![];
    commands::command(&mut code, 0x2020, &[3]);
    commands::command(&mut code, 0x22cb, &[0xabcdef]);
    commands::command(&mut code, 0x203e, &[3, u32::MAX]);
    commands::command(&mut code, 0x22cb, &[0x123456]);
    commands::command(&mut code, 0x203e, &[3, u32::MAX]);
    let root = Root::new("immediate-readback", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    while run.machine().pc() < code.len() - 2 {
        if run.tick(&mut backend).is_err() {
            assert_eq!(run.machine().pending_system().unwrap().1, 0x3e);
            assert_eq!(run.machine().stack().len_bytes(), 8);
            assert!(run.scene().object(3).unwrap().image.is_none());
            // Completed device pixels can differ from current, not-yet-drawn VM state.
            backend.present_frame(RgbaImage::new(1, 1, vec![11, 22, 33, 0]).unwrap());
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
    }
    assert_eq!(run.scene().clear().color, 0x123456);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Quit);
    assert_eq!(run.machine().pc(), 2);
    assert!(run.machine().pending_system().is_none());
    let steps = run.steps();
    run.set_focused(false);
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Quit);
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    while run.machine().pc() != code.len() - 2 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    for _ in 0..2 {
        assert!(run.tick(&mut backend).is_err());
        assert_eq!(run.machine().pending_system(), Some((code.len() - 2, 3)));
        assert_eq!(run.scene().clear().color, 0x123456);
        assert!(run.message_flags().skip());
    }
}

#[test]
fn recording_frames_advance_mgv_and_release_it_at_the_nominal_end() {
    use cmvs_core::{format::mgv, render::raster::RgbaImage};
    #[derive(Debug)]
    struct Decoder;
    impl mgv::Decoder for Decoder {
        fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
            Ok(RgbaImage::new(1, 1, vec![frame as u8, 0, 0, 255])?)
        }
    }
    #[derive(Debug)]
    struct Factory;
    impl mgv::DecoderFactory for Factory {
        fn open(
            &self,
            _: Box<dyn mgv::ReadSeek>,
            _: &mgv::MgvIndex,
        ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error + Send + Sync>> {
            Ok(Box::new(Decoder))
        }
    }
    let mut code = vec![];
    commands::command(&mut code, 0x2015, &[14]);
    commands::command(&mut code, 0x20c4, &[0]);
    commands::command(&mut code, 0x2000, &[]);
    let root = Root::new("mgv-recording-clock", &code, b"synthetic.mgv\0\0");
    let mut movie = b"MGV1".to_vec();
    for word in [32u32, 1, 1, 3, 0x00010001, 1, 10, 1, 1 << 16, 2 << 16] {
        movie.extend(word.to_le_bytes());
    }
    movie.extend([0, 0]);
    fs::write(root.0.join("synthetic.mgv"), movie).unwrap();
    let script = root.script();
    let game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20190305"))
        .unwrap();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    run.set_block_unreviewed(false);
    run.set_mgv_factory(Box::new(Factory));
    let mut backend = RecordingBackend::default();
    for _ in 0..20 {
        run.tick(&mut backend).unwrap();
        if run.movies().has_player() {
            break;
        }
    }
    assert!(run.movies().has_player());
    assert_eq!(run.movies().player_image().unwrap().pixels()[0], 0);
    run.advance_frame(100).unwrap();
    assert_eq!(run.movies().player_image().unwrap().pixels()[0], 1);
    run.advance_frame(100).unwrap();
    assert_eq!(run.movies().player_image().unwrap().pixels()[0], 2);
    run.advance_frame(100).unwrap();
    assert!(!run.movies().has_player());
}

#[test]
fn inactive_policy_freezes_vm_and_scene_but_keeps_audio_updates_without_time_catchup() {
    use cmvs_core::{host::audio::AudioBackend, host::audio::AudioRequest};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Audio(Arc<AtomicUsize>);
    impl AudioBackend for Audio {
        fn configure_voice_ducking(
            &mut self,
            _: cmvs_core::host::audio::VoiceDuckingSettings,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            Ok(())
        }
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
            unreachable!()
        }
        fn update(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
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
        let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
        let updates = Arc::new(AtomicUsize::new(0));
        run.set_audio_backend(Box::new(Audio(updates.clone())))
            .unwrap();
        let mut backend = RecordingBackend::default();
        for _ in 0..30 {
            if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
                break;
            }
        }
        run.advance_playback(16).unwrap();
        let steps = run.steps();
        let clock = run.timers().clock();
        let fade = run.scene().fade().current().unwrap().elapsed;
        run.set_focused(false);
        assert_eq!(run.frame_running(), active_draw);
        for _ in 0..5 {
            assert_eq!(
                run.tick(&mut backend).unwrap(),
                if active_draw {
                    RuntimeEvent::Yield
                } else {
                    RuntimeEvent::Inactive
                }
            );
            run.advance_playback(10).unwrap();
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
        run.advance_playback(16).unwrap();
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    let mut yielded = false;
    for _ in 0..20 {
        if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
            yielded = true;
            break;
        }
    }
    assert!(yielded);
    assert_eq!(backend.requests(), &[PlatformRequest::Fullscreen(true)]);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    run.advance_frame(16).unwrap();
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        if run.machine().state().system_integer(0) == Some(1) {
            return;
        }
    }
    panic!("fullscreen query did not run after the yield");
}

#[test]
fn audio_backend_receives_startup_ducking_and_failed_replacement_keeps_old_device() {
    use cmvs_core::{
        host::audio::AudioBackend, host::audio::AudioRequest, host::audio::VoiceDuckingSettings,
    };
    use std::sync::{Arc, Mutex};
    struct Probe {
        settings: Arc<Mutex<Vec<VoiceDuckingSettings>>>,
        updates: Arc<Mutex<usize>>,
        reject: bool,
    }
    impl AudioBackend for Probe {
        fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
            unreachable!()
        }
        fn configure_voice_ducking(
            &mut self,
            settings: VoiceDuckingSettings,
        ) -> Result<(), Box<dyn Error + Send + Sync>> {
            if self.reject {
                return Err("synthetic configuration failure".into());
            }
            self.settings.lock().unwrap().push(settings);
            Ok(())
        }
        fn update(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
            *self.updates.lock().unwrap() += 1;
            Ok(())
        }
    }
    let root = Root::new("ducking-backend", &0x20a1u16.to_le_bytes(), b"");
    fs::write(root.0.join("cmvs.cfg"), b"[SOUND_INFO]\nVOICE_FADE_VOLOME=42\nVOICE_FADE_TIMER=123\nVOICE_FADE_WAIT=321\nVOICE_FADE_USE=1\n").unwrap();
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
    run.advance_playback(16).unwrap();
    assert_eq!(*updates.lock().unwrap(), 1);
    assert_eq!(settings.lock().unwrap().len(), 1);
}

#[test]
fn startup_main_snapshot_barrier_is_sticky_and_does_not_execute_new_main() {
    let root = Root::new("barrier", &string_call(0x2080), b"fixture.ps3\0");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_snapshot_policy(SnapshotPolicy::Barrier);
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::MainPreparedNeedsSnapshot
    );
    assert_eq!(run.machine().current_name(), b"fixture.ps3");
    let before = (
        run.steps(),
        run.machine().pc(),
        run.machine().stack().len_bytes(),
    );
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::MainPreparedNeedsSnapshot
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_snapshot_policy(SnapshotPolicy::ContinueWithoutImage);
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(*run.save_image(), SaveImage::Absent);
    assert_eq!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::MainEnteredWithoutSnapshot
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
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.scene().clear().rgba(), [0xff, 0x80, 0, 255]);
    assert!(backend.requests().is_empty());
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.set_yield_watchdog(3);
    for _ in 0..3 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    for _ in 0..2 {
        let error = run.tick(&mut backend).unwrap_err().to_string();
        assert!(error.contains("watchdog"), "{error}");
        assert_eq!((run.steps(), run.machine().pc()), (3, 0));
    }
    run.set_yield_watchdog(5);
    for _ in 0..2 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.set_yield_watchdog(2);
    for _ in 0..1000 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    assert_eq!(run.steps(), 2000);
    // One step after the last yield already reaches a watchdog of 1.
    run.set_yield_watchdog(1);
    assert!(run.tick(&mut backend).is_err());
    assert_eq!(run.steps(), 2000);
}
struct Reject;
impl PlatformBackend for Reject {
    fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("synthetic backend failure".into())
    }
}
#[test]
fn startup_backend_error_can_be_retried_without_losing_pending_request() {
    let mut code = string_call(0x2160);
    code.extend(0x27ffu16.to_le_bytes());
    let root = Root::new("retry", &code, b"title\0");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.tick(&mut Reject).unwrap();
    run.tick(&mut Reject).unwrap();
    assert!(run.tick(&mut Reject).is_err());
    let pending = run.machine().pending_system();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert!(pending.is_some());
    assert!(run.machine().pending_system().is_none());
    assert_eq!(backend.requests().len(), 1);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.machine().state().integer(2048), None);
    assert_eq!(run.machine().pc(), 2);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    for _ in 0..6 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    let pc = run.machine().pc();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.scene().fade().current().unwrap().elapsed, 0);
    run.advance_frame(100).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.machine().pc(), pc + 2);
    assert!(run.scene().fade().current().is_none());
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
            command: 0x27ff,
            ..
        }
    ));
}

#[test]
fn message_wait_resumes_without_advancing_time_on_release_completion() {
    let mut code = vec![];
    // Runtime READ_SKIP now follows the INI default, so establish unread ID 0.
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
            break;
        }
    }
    let pc = run.machine().pc();
    assert_eq!(run.machine().pending_system(), Some((pc, 0x153)));
    run.advance_frame(16).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.timers().values()[0], 16);
    run.input_mut().set(0, false, false, true).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert!(!run.input().released(0));
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.timers().values()[0], 16);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    loop {
        if let RuntimeEvent::Unhandled { command, .. } = run.tick(&mut backend).unwrap() {
            assert_eq!(command, 0x2152);
            break;
        }
    }
    assert!(run.messages().text(7).unwrap().glyphs.is_empty());
    run.set_record_text(true);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 1);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 2);
    assert_eq!(run.messages().text(7).unwrap().glyphs.len(), 1);
    run.advance_frame(400).unwrap();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.machine().repeat_count(), 0);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.timers().values()[0], 400);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    run.advance_frame(16).unwrap();
    for expected in [16, 116] {
        for _ in 0..2 {
            run.tick(&mut backend).unwrap();
        }
        assert_eq!(run.machine().state().system_integer(0), Some(expected));
    }
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    // No new frame delta: retrying the host must not reuse the previous 16ms.
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
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
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut backend = RecordingBackend::default();
    run.advance_frame(100).unwrap();
    run.tick(&mut backend).unwrap(); // Mark at 100ms.
    run.advance_frame(7).unwrap(); // A budget slice does not advance the VM clock.
    run.tick(&mut backend).unwrap();
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    run.advance_frame(16).unwrap();
    for _ in 0..2 {
        run.tick(&mut backend).unwrap();
        assert_eq!(run.machine().state().system_integer(0), Some(16));
    }
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Yield);
    run.tick(&mut backend).unwrap(); // No new delta: the previous 16ms is not reused.
    assert_eq!(run.machine().state().system_integer(0), Some(16));
    assert_eq!(run.timers().clock(), 116);
}

#[test]
fn font_dialog_sets_the_global_face_refreshes_default_texts_and_persists() {
    use cmvs_core::{host::settings::DEFAULT_FONT_FACE, host::settings::DisplayPreferences};
    let mut code = vec![];
    commands::command(&mut code, 0x2100, &[1, 0, 0]);
    for _ in 0..3 {
        commands::command(&mut code, 0x213d, &[]);
    }
    let root = Root::new("font-dialog", &code, b"\0");
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("font-dialog");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    backend.answer_font_dialog(Some(b"Face A".to_vec()));
    backend.answer_font_dialog(None);
    while backend.font_dialogs().len() < 2 {
        assert_ne!(run.tick(&mut backend).unwrap(), RuntimeEvent::Inactive);
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
    let mut resumed = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
        let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
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
            cmvs_core::host::settings::DEFAULT_FONT_FACE
        );
    }
}

#[test]
fn verified_startup_globals_are_zero_but_unverified_profiles_remain_unknown() {
    let root = Root::new("verified-globals", &[0, 0x20], b"\0");
    let script = root.script();
    for id in [
        "cmvs-20070119",
        "cmvs-20071117",
        "cmvs-20080719",
        "cmvs-20080903",
        "cmvs-20090815",
        "cmvs-20100314",
        "cmvs-20101211",
        "cmvs-20110525",
        "cmvs-20120309",
        "cmvs-20130131",
        "cmvs-20140217",
        "cmvs-20150405",
        "cmvs-20160201",
        "cmvs-20171114",
        "cmvs-20190305",
        "cmvs-20190405",
        "cmvs-20221105",
        "synthetic-base",
    ] {
        let registry = Registry::new(vec![synthetic_profile(id)], vec![]);
        let game = registry.identify_game(&root.0, Some(id)).unwrap();
        let run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        let state = run.machine().state();
        let verified = id != "synthetic-base";
        assert_eq!(state.integer(2190), verified.then_some(0), "{id}");
        assert_eq!(state.integer(4095), verified.then_some(0), "{id}");
        assert_eq!(state.bit(262143), verified.then_some(false), "{id}");
        assert_eq!(state.float_bits(2047), verified.then_some(0), "{id}");
        assert_eq!(
            state.string(127),
            verified.then_some(b"".as_slice()),
            "{id}"
        );
        assert_eq!(state.integer(4096), None, "{id}");
    }
}

#[test]
fn realive_startup_uses_the_extended_child_table() {
    for id in ["cmvs-20190305", "cmvs-20190405"] {
        let mut code = vec![];
        commands::command(&mut code, 0x2020, &[7]);
        commands::command(&mut code, 0x2022, &[7, 1023]);
        commands::command(&mut code, 0x2000, &[]);
        let root = Root::new("realive-child-startup", &code, b"");
        let script = root.script();
        let game = Registry::builtin()
            .identify_game(&root.0, Some(id))
            .unwrap();
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        run.set_block_unreviewed(false);
        let mut backend = RecordingBackend::default();
        for _ in 0..20 {
            if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
                break;
            }
        }
        assert!(run.scene().object(7).unwrap().children.contains_key(&1023));
    }
}

#[test]
fn realive_fullscreen_monitor_selection_survives_configuration_restart() {
    for id in ["cmvs-20190305", "cmvs-20190405"] {
        let mut code = vec![];
        commands::command(&mut code, 0x23c6, &[5]);
        commands::command(&mut code, 0x2000, &[]);
        let root = Root::new("realive-monitor-preference", &code, b"");
        let script = root.script();
        let game = Registry::builtin()
            .identify_game(&root.0, Some(id))
            .unwrap();
        let directory = crate::support::temp_dir::TempPath::new("realive-monitor-user");
        let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
        run.set_user_data_directory(&directory).unwrap();
        run.set_block_unreviewed(false);
        let mut backend = RecordingBackend::default();
        for _ in 0..20 {
            if run.tick(&mut backend).unwrap() == RuntimeEvent::Yield {
                break;
            }
        }
        assert_eq!(run.platform().fullscreen_monitor_mode(), 5);
        assert_eq!(backend.requests(), [PlatformRequest::FullscreenMonitor(5)]);
        run.save_preferences(Default::default()).unwrap();
        drop(run);
        let mut resumed = Runtime::new(&script, &root.0, &game, 0).unwrap();
        resumed.set_user_data_directory(&directory).unwrap();
        assert_eq!(resumed.platform().fullscreen_monitor_mode(), 5);
        drop(resumed);
        assert!(directory.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn realive_list_selection_receives_input_and_scene_hosts_in_startup() {
    let mut code = vec![];
    for (op, args) in [
        (0x2020, vec![10]),
        (0x2040, vec![10]),
        (0x2022, vec![10, 3]),
        (0x2050, vec![10, 3]),
        (0x2370, vec![7, 10]),
        (0x2372, vec![7, 9]),
        (0x2373, vec![7, 9, 10, 20, 30, 40]),
        (0x2374, vec![7, 9, 1, 3, 10, 0, 4, 5, 0, 0]),
        (0x237a, vec![7, 9, 1]),
        (0x2000, vec![]),
    ] {
        commands::command(&mut code, op, &args);
    }
    let root = Root::new("realive-list-select", &code, b"");
    let script = root.script();
    let game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20190305"))
        .unwrap();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    run.set_block_unreviewed(false);
    let mut backend = RecordingBackend::default();
    let mut yielded = false;
    for _ in 0..100 {
        let event = run.tick(&mut backend).unwrap();
        assert!(!matches!(event, RuntimeEvent::Unhandled { .. }));
        if event == RuntimeEvent::Yield {
            yielded = true;
            break;
        }
    }
    assert!(yielded);
    assert_eq!(run.buttons().list_group(7).unwrap().current, Some(0));
    assert_eq!(run.input().pointer(), [25, 40]);
    assert_eq!(run.machine().state().system_integer(0), Some(1));
}

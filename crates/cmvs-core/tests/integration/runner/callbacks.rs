use super::*;

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
        let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
        let mut backend = RecordingBackend::default();
        let mut yields = 0;
        loop {
            match run.tick(&mut backend).unwrap() {
                RuntimeEvent::Progress => {}
                RuntimeEvent::Yield => {
                    yields += 1;
                    // The callback has not run inside the run that registered it.
                    assert_eq!(run.machine().state().integer(42), None);
                }
                RuntimeEvent::Unhandled { pc, command } => {
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
    let root = Root::new("callback-snapshot", &code, b"Synthetic\0");
    fs::write(root.0.join("save/system.dat"), auto_resume_save(0x12345678)).unwrap();
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("runner-callback");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    let run_to_end = |run: &mut Runtime, backend: &mut RecordingBackend| {
        let mut captured = 0;
        loop {
            match run.tick(backend).unwrap() {
                RuntimeEvent::Progress | RuntimeEvent::Yield => {}
                RuntimeEvent::SnapshotCaptured => {
                    captured += 1;
                    // Captured inside the callback, above its return frame.
                    assert_eq!(run.machine().stack().len_bytes(), 12);
                }
                RuntimeEvent::Unhandled { pc, command } => {
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
    let directory = crate::support::temp_dir::TempPath::in_target_tmp("runner-save-callback");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_user_data_directory(&directory).unwrap();
    let mut backend = RecordingBackend::default();
    let error = loop {
        match run.tick(&mut backend) {
            Ok(RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured) => {}
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

use super::{Root, Runtime, RuntimeEvent, commands};
use cmvs_core::{
    host::platform::DeviceFrameResult, host::platform::PlatformBackend,
    host::platform::PlatformRequest, host::platform::RecordingBackend, render::raster::RgbaImage,
    runtime::FrameBudget, runtime::FrameDriver, runtime::FrameOutcome, runtime::RuntimeError,
};
use std::{error::Error, time::Duration};

fn slice(driver: &mut FrameDriver, run: &mut Runtime, steps: usize) -> FrameOutcome {
    driver
        .run_slice(run, &mut FrameBudget::steps(steps), |run| {
            run.tick(&mut RecordingBackend::default())
        })
        .unwrap()
}

#[test]
fn slices_preserve_input_time_and_hide_uninitialized_children() {
    let mut code = Vec::new();
    for (op, args) in [
        (0x2000, vec![]),
        (0x2020, vec![0]),
        (0x2040, vec![0]),
        (0x2043, vec![0, u32::MAX, 1, 1, 0xff0000]),
        (0x2022, vec![0, 0]),
        (0x2050, vec![0, 0]),
        (0x2043, vec![0, 0, 1, 1, 0x00ff00]),
        (0x2000, vec![]),
        (0x2000, vec![]),
        (0x2001, vec![]),
    ] {
        commands::command(&mut code, op, &args);
    }
    let root = Root::new("frame-slices", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut driver = FrameDriver::default();
    driver.sample_input(run.input_mut(), |_| {});
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    driver.finish_frame(&mut run, 10).unwrap();
    assert!(driver.ready());

    let mut samples = 0;
    let mut partial = false;
    driver.latch_delta(16);
    for i in 0..100 {
        driver.sample_input(run.input_mut(), |input| {
            samples += 1;
            input.set_pointer(i, 0);
            input.set(0, true, true, false).unwrap();
        });
        driver.latch_delta(999);
        let event = slice(&mut driver, &mut run, 1);
        assert!(!driver.ready());
        assert_eq!(run.input().pointer(), [0, 0]);
        assert_eq!(run.input().pressed(0), i == 0);
        run.input_mut().consume(0);
        if run
            .scene()
            .object(0)
            .and_then(|o| o.children.get(&0))
            .is_some_and(|c| c.draw.is_none())
        {
            partial = true;
            assert!(run.render_scene_frame().is_err());
        }
        if event == FrameOutcome::Yield {
            // A pending completion cannot execute another VM instruction.
            let steps = run.steps();
            assert_eq!(slice(&mut driver, &mut run, 10), FrameOutcome::Yield);
            assert_eq!(run.steps(), steps);
            assert!(driver.finish_frame(&mut run, 999).unwrap());
            assert!(!driver.finish_frame(&mut run, 999).unwrap());
            break;
        }
        assert_eq!(event, FrameOutcome::BudgetExhausted);
        assert!(!driver.finish_frame(&mut run, 999).unwrap());
        assert_eq!(run.timers().clock(), 10);
    }
    assert!(partial && driver.ready());
    assert_eq!(samples, 1);
    run.render_scene_frame().unwrap();
    driver.sample_input(run.input_mut(), |_| {});
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    assert_eq!(run.timers().clock(), 26);
    assert_eq!(run.timers().delta_ms(), 16);
    driver.finish_frame(&mut run, 7).unwrap();
    assert_eq!(
        slice(&mut driver, &mut run, 1),
        FrameOutcome::Stopped(RuntimeEvent::Quit)
    );
    assert!(!driver.ready());
}

#[test]
fn inactivity_and_modal_frames_do_not_publish_or_keep_stale_deltas() {
    let mut code = Vec::new();
    commands::command(&mut code, 0x2000, &[]);
    commands::command(&mut code, 0x201e, &[1]);
    for _ in 0..4 {
        commands::command(&mut code, 0x2000, &[]);
    }
    let root = Root::new("frame-pause", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut driver = FrameDriver::default();
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    driver.finish_frame(&mut run, 10).unwrap();
    run.set_focused(false);
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Inactive);
    assert!(driver.ready()); // The previously completed canvas remains safe.
    run.set_focused(true);
    driver.latch_delta(200);
    driver.sample_input(run.input_mut(), |_| {});
    assert_eq!(
        slice(&mut driver, &mut run, 1),
        FrameOutcome::BudgetExhausted
    );
    assert!(!driver.ready());
    run.set_focused(false);
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Inactive);
    assert!(!driver.ready()); // Inactivity cannot promote the partial frame.
    assert!(driver.needs_input());
    run.set_focused(true);
    driver.sample_input(run.input_mut(), |input| input.set_pointer(3, 4));
    assert_eq!(slice(&mut driver, &mut run, 10), FrameOutcome::Yield);
    driver.finish_frame(&mut run, 5).unwrap();
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    assert_eq!(run.timers().delta_ms(), 5);
    driver.latch_delta(200);
    driver.skip_frame();
    assert!(!driver.ready());
    assert!(driver.needs_input());
    assert!(!driver.finish_frame(&mut run, 200).unwrap());
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    assert_eq!(run.timers().delta_ms(), 0);
    driver.finish_frame(&mut run, 3).unwrap();
    assert_eq!(slice(&mut driver, &mut run, 1), FrameOutcome::Yield);
    assert_eq!(run.timers().delta_ms(), 3);
}

#[test]
fn device_retry_keeps_slice_budget_and_structured_stops() {
    let root = Root::new("frame-retry", &[0x00, 0x20], b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    let mut driver = FrameDriver::default();
    driver.sample_input(run.input_mut(), |_| {});
    driver.latch_delta(16);
    let mut budget = FrameBudget::steps(2);
    let error = driver
        .run_slice(&mut run, &mut budget, |_| {
            Err::<RuntimeEvent, _>("readback pending")
        })
        .unwrap_err();
    assert_eq!(error, "readback pending");
    assert!(!driver.needs_input());
    let mut calls = 0;
    assert_eq!(
        driver
            .run_slice(&mut run, &mut budget, |_| {
                calls += 1;
                Ok::<_, Box<dyn Error + Send + Sync>>(RuntimeEvent::Progress)
            })
            .unwrap(),
        FrameOutcome::BudgetExhausted
    );
    assert_eq!(calls, 1);
    assert_eq!(
        driver
            .run_slice(
                &mut run,
                &mut FrameBudget::time(Duration::ZERO),
                |_| -> Result<RuntimeEvent, ()> { panic!("expired budget must not execute") }
            )
            .unwrap(),
        FrameOutcome::BudgetExhausted
    );
    for stop in [
        RuntimeEvent::Unhandled {
            pc: 12,
            command: 0x27ff,
        },
        RuntimeEvent::MainPreparedNeedsSnapshot,
        RuntimeEvent::MainEnteredWithoutSnapshot,
    ] {
        let outcome = driver
            .run_slice(&mut run, &mut FrameBudget::steps(10), |_| Ok::<_, ()>(stop))
            .unwrap();
        assert_eq!(outcome, FrameOutcome::Stopped(stop));
        assert!(!driver.ready());
        assert!(!driver.finish_frame(&mut run, 100).unwrap());
    }
}

#[test]
fn async_device_frame_pending_retains_input_delta_watchdog_and_resumes() {
    #[derive(Default)]
    struct AsyncBackend {
        frame: Option<RgbaImage>,
    }
    impl PlatformBackend for AsyncBackend {
        fn poll_device_frame(&mut self) -> Result<DeviceFrameResult, Box<dyn Error + Send + Sync>> {
            Ok(match self.frame.clone() {
                Some(image) => DeviceFrameResult::Ready(image),
                None => DeviceFrameResult::Pending,
            })
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
            unreachable!()
        }
    }

    let mut code = Vec::new();
    for (op, args) in [
        (0x2020, vec![3]),
        (0x2040, vec![3]),
        (0x203e, vec![3, u32::MAX]),
        (0x2000, vec![]),
        (0x2000, vec![]),
    ] {
        commands::command(&mut code, op, &args);
    }
    let root = Root::new("async-device-frame", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    // Strict yield watchdog must not trip while waiting for async GPU readback.
    // 6 setup steps precede 0x203e, 0x203e adds 1 when it completes, and 0x2000 yields at 7 < 8.
    run.set_yield_watchdog(8);
    let mut driver = FrameDriver::default();
    let mut backend = AsyncBackend::default();
    let mut budget = FrameBudget::steps(20);

    driver.latch_delta(16);
    driver.sample_input(run.input_mut(), |input| {
        input.set(0, true, true, false).unwrap();
    });
    for _ in 0..5 {
        let outcome = driver
            .run_slice(&mut run, &mut budget, |run| run.tick(&mut backend))
            .unwrap();
        assert_eq!(
            outcome,
            FrameOutcome::Stopped(RuntimeEvent::DeviceFramePending)
        );
        assert!(!driver.needs_input());
        assert!(!driver.ready());
        assert!(!driver.finish_frame(&mut run, 999).unwrap());
        assert_eq!(run.timers().delta_ms(), 0);
        assert!(run.input().pressed(0));
    }

    backend.frame = Some(RgbaImage::new(1, 1, vec![12, 34, 56, 78]).unwrap());
    let outcome = driver
        .run_slice(&mut run, &mut budget, |run| run.tick(&mut backend))
        .unwrap();
    assert_eq!(outcome, FrameOutcome::Yield);
    assert!(driver.finish_frame(&mut run, 999).unwrap());
    assert!(driver.ready());
    assert_eq!(
        run.scene()
            .object(3)
            .unwrap()
            .image
            .as_ref()
            .unwrap()
            .pixels
            .pixels(),
        &[12, 34, 56, 255]
    );
    // The latched 16 ms delta is applied on the next VM run, not the ignored 999 ms.
    assert_eq!(slice(&mut driver, &mut run, 10), FrameOutcome::Yield);
    assert_eq!(run.timers().delta_ms(), 16);
}

#[test]
fn runtime_error_exposes_structured_variants_and_sources() {
    use cmvs_core::profile::GateErrorKind;
    // Watchdog error
    let root = Root::new("runtime-error-watchdog", &[0x00, 0x20], b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    run.set_yield_watchdog(0);
    let err = run.tick(&mut RecordingBackend::default()).unwrap_err();
    assert!(matches!(err, RuntimeError::Watchdog { steps: 0 }));
    assert_eq!(
        err.to_string(),
        "research watchdog: 0 VM steps without a yield"
    );
    assert!(err.source().is_none());

    // Gate error (absent command 0x27ff in cmvs-20160201)
    let mut code = Vec::new();
    commands::command(&mut code, 0x27ff, &[]);
    let root = Root::new("runtime-error-gate", &code, b"");
    let game = cmvs_core::profile::Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20160201"))
        .unwrap();
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let err = run.tick(&mut RecordingBackend::default()).unwrap_err();
    let gate = err.as_gate().expect("structured gate error");
    assert_eq!(gate.kind, GateErrorKind::Absent);
    assert_eq!(gate.command, 0x7ff);

    // Dispatch error wrapping an underlying io::Error through platform device frame readback
    let mut code = Vec::new();
    commands::command(&mut code, 0x2020, &[3]);
    commands::command(&mut code, 0x203e, &[3, u32::MAX]);
    let root = Root::new("runtime-error-dispatch-io", &code, b"");
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &root.game(), 0).unwrap();
    struct FailingDevice;
    impl PlatformBackend for FailingDevice {
        fn read_device_frame(&mut self) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gpu pipe lost").into())
        }
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
            unreachable!()
        }
    }
    let mut backend = FailingDevice;
    for _ in 0..4 {
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    }
    let err = run.tick(&mut backend).unwrap_err();
    assert!(err.as_dispatch().is_some());
    assert_eq!(err.as_io().unwrap().kind(), std::io::ErrorKind::BrokenPipe);
}

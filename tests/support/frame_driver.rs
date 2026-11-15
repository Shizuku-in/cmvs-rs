use super::{Root, Runtime, RuntimeEvent, commands};
use cmvs_rs::{
    platform::RecordingBackend,
    runtime::{FrameBudget, FrameDriver, FrameOutcome},
};
use std::{error::Error, time::Duration};

fn slice(driver: &mut FrameDriver, run: &mut Runtime<'_>, steps: usize) -> FrameOutcome {
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
                Ok::<_, Box<dyn Error>>(RuntimeEvent::Progress)
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

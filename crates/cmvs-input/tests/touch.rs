use cmvs_core::input::{ActionInput, KeyBindings};
use cmvs_input::{TouchEvent, TouchInput, TouchPhase, Viewport};

fn bindings() -> KeyBindings {
    KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=1").unwrap()
}

fn viewport() -> Viewport {
    Viewport::new([1000, 1000]).unwrap()
}

fn event(touch: &mut TouchInput, input: &mut ActionInput, id: u64, phase: TouchPhase, p: [f64; 2]) {
    touch.event(
        TouchEvent {
            id,
            phase,
            position: p,
        },
        viewport(),
        input,
    );
}

fn focused() -> (TouchInput, ActionInput) {
    let mut touch = TouchInput::default();
    let mut input = ActionInput::default();
    touch.focus(true, &mut input);
    (touch, input)
}

#[test]
fn short_tap_between_frames_retains_press_and_release_position() {
    let (mut touch, mut input) = focused();
    event(
        &mut touch,
        &mut input,
        7,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    event(&mut touch, &mut input, 7, TouchPhase::Ended, [500.0, 500.0]);
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.pressed(0) && input.held(0));
    assert_eq!(input.mouse(0), [false, true, true]);
    assert_eq!(input.pointer(), [640, 360]);
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.released(0) && !input.held(0));
    assert_eq!(input.pointer(), [640, 360]);
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(!input.released(0) && !input.pressed(0));
}

#[test]
fn drag_outside_and_back_does_not_generate_an_extra_press() {
    let (mut touch, mut input) = focused();
    event(
        &mut touch,
        &mut input,
        1,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    touch.frame(&bindings(), viewport(), &mut input);
    for point in [[500.0, 20.0], [600.0, 500.0]] {
        event(&mut touch, &mut input, 1, TouchPhase::Moved, point);
        touch.frame(&bindings(), viewport(), &mut input);
        assert!(input.held(0));
        assert!(!input.pressed(0) && !input.released(0));
    }
    event(&mut touch, &mut input, 1, TouchPhase::Ended, [600.0, 500.0]);
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.released(0));
    assert_eq!(input.pointer(), [768, 360]);
}

#[test]
fn secondary_contacts_never_take_over_even_after_primary_ends() {
    let (mut touch, mut input) = focused();
    event(
        &mut touch,
        &mut input,
        1,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    event(
        &mut touch,
        &mut input,
        2,
        TouchPhase::Started,
        [600.0, 500.0],
    );
    touch.frame(&bindings(), viewport(), &mut input);
    assert_eq!(input.pointer(), [640, 360]);
    event(&mut touch, &mut input, 1, TouchPhase::Ended, [500.0, 500.0]);
    event(&mut touch, &mut input, 2, TouchPhase::Moved, [700.0, 500.0]);
    event(
        &mut touch,
        &mut input,
        3,
        TouchPhase::Started,
        [700.0, 500.0],
    );
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.released(0) && !input.held(0));
    assert_eq!(input.pointer(), [640, 360]);
    event(&mut touch, &mut input, 2, TouchPhase::Ended, [700.0, 500.0]);
    event(&mut touch, &mut input, 3, TouchPhase::Ended, [700.0, 500.0]);
    event(
        &mut touch,
        &mut input,
        4,
        TouchPhase::Started,
        [600.0, 500.0],
    );
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.pressed(0));
    assert_eq!(input.pointer(), [768, 360]);
}

#[test]
fn a_gesture_starting_in_a_bar_or_at_nan_is_ignored_until_all_contacts_end() {
    for outside in [[500.0, 20.0], [f64::NAN, 500.0]] {
        let (mut touch, mut input) = focused();
        event(&mut touch, &mut input, 1, TouchPhase::Started, outside);
        event(&mut touch, &mut input, 1, TouchPhase::Moved, [500.0, 500.0]);
        event(
            &mut touch,
            &mut input,
            2,
            TouchPhase::Started,
            [500.0, 500.0],
        );
        event(&mut touch, &mut input, 1, TouchPhase::Ended, [500.0, 500.0]);
        event(&mut touch, &mut input, 2, TouchPhase::Ended, [500.0, 500.0]);
        touch.frame(&bindings(), viewport(), &mut input);
        assert!(!input.held(0) && !input.pressed(0) && !input.released(0));
        assert_eq!(input.pointer(), [-1, -1]);
    }
}

#[test]
fn cancellation_drops_sampled_and_unsampled_clicks_without_release() {
    for sample in [false, true] {
        let (mut touch, mut input) = focused();
        event(
            &mut touch,
            &mut input,
            1,
            TouchPhase::Started,
            [500.0, 500.0],
        );
        if sample {
            touch.frame(&bindings(), viewport(), &mut input);
        }
        event(
            &mut touch,
            &mut input,
            2,
            TouchPhase::Started,
            [600.0, 500.0],
        );
        event(
            &mut touch,
            &mut input,
            1,
            TouchPhase::Cancelled,
            [500.0, 500.0],
        );
        // Duplicate starts cannot promote a secondary contact.
        event(
            &mut touch,
            &mut input,
            2,
            TouchPhase::Started,
            [600.0, 500.0],
        );
        touch.frame(&bindings(), viewport(), &mut input);
        assert_eq!(input.mouse(0), [false; 3]);
        assert!(!input.held(0) && !input.pressed(0) && !input.released(0));
        assert_eq!(input.pointer(), [-1, -1]);
    }
}

#[test]
fn focus_loss_clears_pending_input_and_ignores_stale_moves_after_resume() {
    let (mut touch, mut input) = focused();
    event(
        &mut touch,
        &mut input,
        1,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    touch.focus(false, &mut input);
    event(
        &mut touch,
        &mut input,
        2,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    touch.focus(true, &mut input);
    event(&mut touch, &mut input, 1, TouchPhase::Moved, [500.0, 500.0]);
    touch.frame(&bindings(), viewport(), &mut input);
    assert_eq!(input.mouse(0), [false; 3]);
    assert_eq!(input.pointer(), [-1, -1]);
    event(
        &mut touch,
        &mut input,
        2,
        TouchPhase::Started,
        [500.0, 500.0],
    );
    touch.frame(&bindings(), viewport(), &mut input);
    assert!(input.pressed(0));
    touch.cancel(&mut input);
    touch.frame(&bindings(), viewport(), &mut input);
    assert_eq!(input.mouse(0), [false; 3]);
}

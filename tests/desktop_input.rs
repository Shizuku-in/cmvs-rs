#![cfg(feature = "desktop")]
use cmvs_rs::{
    desktop::game_input::{GameInput, Viewport},
    input::{ActionInput, KeyBindings},
};

#[test]
fn cover_and_stretch_mapping_and_navigation_use_the_selected_mode() {
    let cover = Viewport::with_mode([1000, 1000], 2).unwrap();
    assert!(cover.to_script([0.0, 500.0]).unwrap()[0] > 0);
    let stretch = Viewport::with_mode([1000, 1000], 3).unwrap();
    assert_eq!(stretch.to_script([0.0, 0.0]), Some([0, 0]));
    for mode in 0..=3 {
        let mut game = GameInput::default();
        let mut input = ActionInput::default();
        game.focus(true, &mut input);
        game.set_aspect_mode(mode);
        let point = game.warp([1000, 1000], [640, 360]).unwrap();
        assert_eq!(
            Viewport::with_mode([1000, 1000], mode)
                .unwrap()
                .to_script(point),
            Some([640, 360])
        );
        game.frame(&KeyBindings::default(), [1000, 1000], &mut input);
        assert_eq!(input.pointer(), [640, 360]);
    }
}

#[test]
fn aspect_fit_maps_pixels_and_rejects_bars_and_zero_surface() {
    let v = Viewport::new([1000, 1000]).unwrap();
    assert_eq!(v.to_script([0.0, 219.0]), Some([0, 0]));
    assert_eq!(v.to_script([500.0, 500.0]), Some([640, 360]));
    assert_eq!(v.to_script([500.0, 218.0]), None);
    assert_eq!(v.to_script([500.0, 781.0]), None);
    assert_eq!(v.to_script([f64::NAN, 0.0]), None);
    assert!(Viewport::new([0, 100]).is_none());
    let p = v.to_window([640, 360]);
    assert_eq!(v.to_script(p), Some([640, 360]));
}

#[test]
fn focus_loss_cancels_click_and_wheel_is_a_single_frame_pulse() {
    let bindings = KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=4").unwrap();
    let mut native = GameInput::default();
    let mut input = ActionInput::default();
    native.focus(true, &mut input);
    native.pointer(480.0, 270.0);
    native.wheel(1.0);
    native.frame(&bindings, [960, 540], &mut input);
    assert!(input.pressed(0) && input.held(0));
    assert_eq!(input.pointer(), [640, 360]);
    native.frame(&bindings, [960, 540], &mut input);
    assert!(input.released(0) && !input.held(0));
    native.wheel(1.0);
    native.frame(&bindings, [960, 540], &mut input);
    native.focus(false, &mut input);
    native.frame(&bindings, [960, 540], &mut input);
    assert!(!input.released(0) && !input.held(0));
    assert_eq!(input.pointer(), [-1, -1]);
}

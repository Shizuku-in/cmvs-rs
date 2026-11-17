//! Native event adapter for the verified CMVS input-code table. No Windows APIs in core.
use cmvs_core::{host::input::ActionInput, host::input::KeyBindings};
use cmvs_host::InputState;
pub use cmvs_input::Viewport;
use std::collections::BTreeSet;
use winit::{
    event::MouseButton,
    keyboard::{Key, KeyCode, NamedKey},
};

/// Named navigation keys retain their virtual-key meaning, including arrows emitted
/// by a numpad with NumLock off. OEM/letter positions still use physical codes.
pub fn event_key(physical: KeyCode, logical: &Key) -> KeyCode {
    match logical {
        Key::Named(NamedKey::ArrowUp) => KeyCode::ArrowUp,
        Key::Named(NamedKey::ArrowDown) => KeyCode::ArrowDown,
        Key::Named(NamedKey::ArrowLeft) => KeyCode::ArrowLeft,
        Key::Named(NamedKey::ArrowRight) => KeyCode::ArrowRight,
        Key::Named(NamedKey::Home) => KeyCode::Home,
        Key::Named(NamedKey::End) => KeyCode::End,
        Key::Named(NamedKey::PageUp) => KeyCode::PageUp,
        Key::Named(NamedKey::PageDown) => KeyCode::PageDown,
        Key::Named(NamedKey::Insert) => KeyCode::Insert,
        Key::Named(NamedKey::Delete) => KeyCode::Delete,
        _ => physical,
    }
}

fn mouse_button_code(button: MouseButton) -> Option<usize> {
    match button {
        MouseButton::Left => Some(1),
        MouseButton::Middle => Some(2),
        MouseButton::Right => Some(3),
        MouseButton::Back => Some(6),
        MouseButton::Forward => Some(7),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct GameInput {
    keys: BTreeSet<KeyCode>,
    buttons: BTreeSet<MouseButton>,
    state: InputState,
    pointer: Option<[f64; 2]>,
    wheel: f64,
    focused: bool,
    aspect_mode: Option<u32>,
    screen_size: cmvs_core::render::viewport::ScreenSize,
}
impl GameInput {
    pub fn set_screen_size(&mut self, size: cmvs_core::render::viewport::ScreenSize) {
        self.screen_size = size;
    }
    pub fn set_aspect_mode(&mut self, mode: u32) {
        self.aspect_mode = Some(mode);
    }
    pub fn cursor_position(&self, size: [u32; 2]) -> Option<[i32; 2]> {
        self.pointer.and_then(|p| {
            Viewport::for_canvas(
                self.screen_size.pixels(),
                size,
                self.aspect_mode.unwrap_or(1),
            )?
            .to_script(p)
        })
    }
    pub fn key(&mut self, key: KeyCode, pressed: bool) {
        if !self.focused {
            return;
        }
        if pressed {
            if self.keys.insert(key)
                && let Some(code) = key_code(key)
            {
                let _ = self.state.pulse(usize::from(code));
            }
        } else {
            self.keys.remove(&key);
        }
    }
    pub fn button(&mut self, button: MouseButton, pressed: bool) {
        if !self.focused {
            return;
        }
        if pressed {
            if self.buttons.insert(button)
                && let Some(code) = mouse_button_code(button)
            {
                let _ = self.state.pulse(code);
            }
        } else {
            self.buttons.remove(&button);
        }
    }
    pub fn pointer(&mut self, x: f64, y: f64) {
        if self.focused {
            self.pointer = Some([x, y]);
        }
    }
    pub fn leave(&mut self) {
        self.pointer = None;
    }
    pub fn wheel(&mut self, y: f64) {
        if self.focused {
            self.wheel += y;
        }
    }
    pub fn focus(&mut self, focused: bool, input: &mut ActionInput) {
        self.focused = focused;
        if !focused {
            self.keys.clear();
            self.buttons.clear();
            self.wheel = 0.0;
            self.pointer = None;
            self.state.clear(input);
        }
    }
    pub fn frame(&mut self, bindings: &KeyBindings, size: [u32; 2], input: &mut ActionInput) {
        // Recompute currently held codes from physical keys/buttons so shared codes
        // (e.g. Left/Right Shift or Control) stay held until both keys release, while
        // sub-frame taps stay latched via `self.state.pulse`.
        self.state.clear_held();
        for &key in &self.keys {
            if let Some(code) = key_code(key) {
                let _ = self.state.set_held(usize::from(code), true);
            }
        }
        for &button in &self.buttons {
            if let Some(code) = mouse_button_code(button) {
                let _ = self.state.set_held(code, true);
            }
        }
        // Native wheel units: positive is up. CMVS code 4=up, 5=down.
        if self.wheel > 0.0 {
            let _ = self.state.pulse(4);
        } else if self.wheel < 0.0 {
            let _ = self.state.pulse(5);
        }
        self.wheel = 0.0;
        let [x, y] = self.cursor_position(size).unwrap_or([-1, -1]);
        self.state.set_pointer(x, y);
        self.state.sample(bindings, input);
    }
    pub fn warp(&mut self, size: [u32; 2], script: [i32; 2]) -> Option<[f64; 2]> {
        let point = Viewport::for_canvas(
            self.screen_size.pixels(),
            size,
            self.aspect_mode.unwrap_or(1),
        )?
        .to_window(script);
        self.pointer = Some(point);
        Some(point)
    }
}

/// Keyboard portion of 0x140056780. Physical positions for OEM/letter keys are
/// the current desktop policy; non-US layout equivalence is not yet verified.
pub fn key_code(key: KeyCode) -> Option<u8> {
    use KeyCode::*;
    Some(match key {
        Backspace => 20,
        Tab => 21,
        Enter | NumpadEnter => 22,
        ShiftLeft | ShiftRight => 23,
        ControlLeft | ControlRight => 24,
        Escape => 25,
        Space => 26,
        PageUp => 27,
        PageDown => 28,
        End => 29,
        Home => 30,
        Insert => 31,
        Delete => 32,
        ArrowUp => 33,
        ArrowDown => 34,
        ArrowLeft => 35,
        ArrowRight => 36,
        Numpad0 => 37,
        Numpad1 => 38,
        Numpad2 => 39,
        Numpad3 => 40,
        Numpad4 => 41,
        Numpad5 => 42,
        Numpad6 => 43,
        Numpad7 => 44,
        Numpad8 => 45,
        Numpad9 => 46,
        NumpadMultiply => 47,
        NumpadAdd => 48,
        NumpadSubtract => 49,
        NumpadDecimal => 50,
        NumpadDivide => 51,
        F1 => 52,
        F2 => 53,
        F3 => 54,
        F4 => 55,
        F5 => 56,
        F6 => 57,
        F7 => 58,
        F8 => 59,
        F9 => 60,
        F10 => 61,
        F11 => 62,
        F12 => 63,
        F13 => 64,
        F14 => 65,
        F15 => 66,
        F16 => 67,
        F17 => 68,
        F18 => 69,
        F19 => 70,
        F20 => 71,
        F21 => 72,
        F22 => 73,
        F23 => 74,
        F24 => 75,
        Digit0 => 76,
        Digit1 => 77,
        Digit2 => 78,
        Digit3 => 79,
        Digit4 => 80,
        Digit5 => 81,
        Digit6 => 82,
        Digit7 => 83,
        Digit8 => 84,
        Digit9 => 85,
        KeyA => 86,
        KeyB => 87,
        KeyC => 88,
        KeyD => 89,
        KeyE => 90,
        KeyF => 91,
        KeyG => 92,
        KeyH => 93,
        KeyI => 94,
        KeyJ => 95,
        KeyK => 96,
        KeyL => 97,
        KeyM => 98,
        KeyN => 99,
        KeyO => 100,
        KeyP => 101,
        KeyQ => 102,
        KeyR => 103,
        KeyS => 104,
        KeyT => 105,
        KeyU => 106,
        KeyV => 107,
        KeyW => 108,
        KeyX => 109,
        KeyY => 110,
        KeyZ => 111,
        Semicolon => 112,
        Equal => 113,
        Comma => 114,
        Minus => 115,
        Period => 116,
        Slash => 117,
        Backquote => 118,
        BracketLeft => 119,
        Backslash => 120,
        BracketRight => 121,
        Quote => 122,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_without_system_cursor_warp_persists_until_mouse_motion() {
        let mut game = GameInput::default();
        let mut input = ActionInput::default();
        let bindings = KeyBindings::default();
        let size = [800, 600];
        game.set_screen_size(cmvs_core::render::viewport::ScreenSize::new(size).unwrap());
        game.focus(true, &mut input);
        game.pointer(40.0, 50.0);
        game.frame(&bindings, size, &mut input);
        for (key, target) in [
            (KeyCode::ArrowDown, [560, 350]),
            (KeyCode::ArrowUp, [560, 290]),
        ] {
            game.key(key, true);
            game.frame(&bindings, size, &mut input);
            game.warp(size, target).unwrap();
            // Wayland does not deliver a system cursor move for keyboard navigation.
            game.key(key, false);
            for _ in 0..3 {
                game.frame(&bindings, size, &mut input);
                assert_eq!(input.pointer(), target);
            }
        }
        game.pointer(100.0, 200.0);
        game.frame(&bindings, size, &mut input);
        assert_eq!(input.pointer(), [100, 200]);
    }

    #[test]
    fn short_native_events_survive_until_a_frame_and_release_once() {
        let b = KeyBindings::parse(
            "[KEY_FUNCTION_01]\nKEY_DATA_01=1\n[KEY_FUNCTION_04]\nKEY_DATA_01=34",
        )
        .unwrap();
        let mut host = GameInput::default();
        let mut input = ActionInput::default();
        host.focus(true, &mut input);
        host.button(MouseButton::Left, true);
        host.button(MouseButton::Left, false);
        let key = event_key(KeyCode::Numpad2, &Key::Named(NamedKey::ArrowDown));
        host.key(key, true);
        host.key(key, false);
        host.frame(&b, [960, 540], &mut input);
        assert!(input.pressed(0) && input.pressed(3));
        input.consume(0);
        host.frame(&b, [960, 540], &mut input);
        assert!(input.released(0) && input.released(3));
        host.frame(&b, [960, 540], &mut input);
        assert!(!input.released(0) && !input.released(3));
    }
    #[test]
    fn both_modifier_keys_share_a_code_without_releasing_the_other() {
        let b = KeyBindings::parse("[KEY_FUNCTION_09]\nKEY_DATA_01=24").unwrap();
        let mut host = GameInput::default();
        let mut input = ActionInput::default();
        host.focus(true, &mut input);
        host.key(KeyCode::ControlLeft, true);
        host.key(KeyCode::ControlRight, true);
        host.frame(&b, [960, 540], &mut input);
        host.key(KeyCode::ControlLeft, false);
        host.frame(&b, [960, 540], &mut input);
        assert!(input.held(8) && !input.released(8));
        host.focus(false, &mut input);
        host.frame(&b, [960, 540], &mut input);
        assert!(!input.held(8) && !input.released(8));
    }
}

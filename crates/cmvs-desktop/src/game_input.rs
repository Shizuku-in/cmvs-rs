//! Native event adapter for the verified CMVS input-code table. No Windows APIs in core.
use cmvs_core::input::{ActionInput, CodeInput, KeyBindings};
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

#[derive(Debug, Default)]
pub struct GameInput {
    keys: BTreeSet<KeyCode>,
    buttons: BTreeSet<MouseButton>,
    key_pulses: BTreeSet<KeyCode>,
    button_pulses: BTreeSet<MouseButton>,
    history: CodeInput,
    pointer: Option<[f64; 2]>,
    wheel: f64,
    focused: bool,
}
impl GameInput {
    pub fn key(&mut self, key: KeyCode, pressed: bool) {
        if !self.focused {
            return;
        }
        if pressed {
            if self.keys.insert(key) {
                self.key_pulses.insert(key);
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
            if self.buttons.insert(button) {
                self.button_pulses.insert(button);
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
            self.key_pulses.clear();
            self.button_pulses.clear();
            self.wheel = 0.0;
            self.pointer = None;
            self.history.clear(input);
        }
    }
    pub fn frame(&mut self, bindings: &KeyBindings, size: [u32; 2], input: &mut ActionInput) {
        let mut codes = [false; 128];
        // Latch a short press until sampled: even a complete down/up between two
        // frames produces one down frame followed by one release frame.
        for &key in self.keys.union(&self.key_pulses) {
            if let Some(code) = key_code(key) {
                codes[usize::from(code)] = true;
            }
        }
        for &(button, code) in &[
            (MouseButton::Left, 1),
            (MouseButton::Middle, 2),
            (MouseButton::Right, 3),
            (MouseButton::Back, 6),
            (MouseButton::Forward, 7),
        ] {
            codes[code] = self.buttons.contains(&button) || self.button_pulses.contains(&button);
        }
        // Native wheel units: positive is up. CMVS code 4=up, 5=down.
        codes[4] = self.wheel > 0.0;
        codes[5] = self.wheel < 0.0;
        self.wheel = 0.0;
        // VK_F1..VK_F12 whose hold counter becomes 1 this frame (0x140056780), for 0x234e.
        let function_keys = [
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
            KeyCode::F8,
            KeyCode::F9,
            KeyCode::F10,
            KeyCode::F11,
            KeyCode::F12,
        ];
        let pressed = function_keys
            .iter()
            .enumerate()
            .filter(|(_, key)| self.key_pulses.contains(key))
            .fold(0u16, |mask, (bit, _)| mask | 1 << bit);
        input.press_function_keys(pressed);
        self.key_pulses.clear();
        self.button_pulses.clear();
        self.history.update(bindings, &codes, input);
        let [x, y] = self
            .pointer
            .and_then(|p| Viewport::new(size)?.to_script(p))
            .unwrap_or([-1, -1]);
        input.set_pointer(x, y);
    }
    pub fn warp(&mut self, size: [u32; 2], script: [i32; 2]) -> Option<[f64; 2]> {
        let point = Viewport::new(size)?.to_window(script);
        self.pointer = Some(point);
        Some(point)
    }
}

/// Exactly the integer aspect fit used by RgbaImage::draw_fit for the 1280x720 scene.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}
impl Viewport {
    pub fn new([width, height]: [u32; 2]) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let mut w = u64::from(width);
        let mut h = (720 * w / 1280).max(1);
        if h > u64::from(height) {
            h = u64::from(height);
            w = (1280 * h / 720).max(1);
        }
        Some(Self {
            left: (width - w as u32) / 2,
            top: (height - h as u32) / 2,
            width: w as u32,
            height: h as u32,
        })
    }
    pub fn to_script(self, [x, y]: [f64; 2]) -> Option<[i32; 2]> {
        let x = x - f64::from(self.left);
        let y = y - f64::from(self.top);
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= f64::from(self.width)
            || y >= f64::from(self.height)
        {
            return None;
        }
        Some([
            (x * 1280.0 / f64::from(self.width)).floor() as i32,
            (y * 720.0 / f64::from(self.height)).floor() as i32,
        ])
    }
    pub fn to_window(self, [x, y]: [i32; 2]) -> [f64; 2] {
        [
            f64::from(self.left) + (f64::from(x) + 0.5) * f64::from(self.width) / 1280.0,
            f64::from(self.top) + (f64::from(y) + 0.5) * f64::from(self.height) / 720.0,
        ]
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

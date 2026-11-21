//! Shared 128-code CMVS input state with sub-frame pulse latching and pointer tracking.
use cmvs_core::host::input::{ActionInput, CodeInput, KeyBindings};

/// Tracks 128 CMVS key/mouse/wheel codes (`0..128`), sub-frame press pulses, edge state,
/// and script-space pointer coordinates across frame boundaries.
#[derive(Debug, Clone)]
pub struct InputState {
    held: [bool; 128],
    pulses: [bool; 128],
    codes: CodeInput,
    pointer: [i32; 2],
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    /// Creates an idle input state with the pointer at `[0, 0]`.
    pub fn new() -> Self {
        Self {
            held: [false; 128],
            pulses: [false; 128],
            codes: CodeInput::default(),
            pointer: [0; 2],
        }
    }

    /// Creates an idle input state with the given initial pointer position.
    pub fn with_pointer(pointer: [i32; 2]) -> Self {
        Self {
            pointer,
            ..Self::new()
        }
    }

    /// Updates a CMVS input code (`0..128`), latching a press pulse if transitioning
    /// from unheld to held so sub-frame press+release pairs survive until [`Self::sample`].
    pub fn set_key(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
        let held = self.held.get_mut(code).ok_or("input code outside 0..128")?;
        if pressed && !*held {
            self.pulses[code] = true;
        }
        *held = pressed;
        Ok(())
    }

    /// Directly sets the held state of `code` (`0..128`) without altering latched pulses.
    pub fn set_held(&mut self, code: usize, is_held: bool) -> Result<(), &'static str> {
        let held = self.held.get_mut(code).ok_or("input code outside 0..128")?;
        *held = is_held;
        Ok(())
    }

    pub fn cancel_primary_button(&mut self) {
        self.held[1] = false;
        self.pulses[1] = false;
        self.codes.forget_primary_button();
    }

    /// Drop mouse buttons, wheel pulses and the old location when the viewport changes.
    pub fn cancel_pointer(&mut self) {
        self.held[1..=7].fill(false);
        self.pulses[1..=7].fill(false);
        self.codes.forget_pointer();
        self.set_pointer(-1, -1);
    }

    /// Clears all currently held codes while keeping any already-latched pulses intact.
    pub fn clear_held(&mut self) {
        self.held.fill(false);
    }

    /// Replaces the currently held codes from a slice of active code indices (ignoring `>= 128`).
    pub fn set_held_codes(&mut self, active_codes: &[u8]) {
        self.held.fill(false);
        for &code in active_codes {
            if let Some(slot) = self.held.get_mut(usize::from(code)) {
                *slot = true;
            }
        }
    }

    /// Latches a single-frame press pulse for `code` (`0..128`) consumed by the next [`Self::sample`].
    pub fn pulse(&mut self, code: usize) -> Result<(), &'static str> {
        let slot = self
            .pulses
            .get_mut(code)
            .ok_or("input code outside 0..128")?;
        *slot = true;
        Ok(())
    }

    /// Sets the script-space pointer position forwarded on [`Self::sample`].
    pub fn set_pointer(&mut self, x: i32, y: i32) {
        self.pointer = [x, y];
    }

    /// Returns the current script-space pointer position.
    pub fn pointer(&self) -> [i32; 2] {
        self.pointer
    }

    /// Cancels all held keys, pending pulses, and engine edge history without emitting release edges.
    pub fn clear(&mut self, input: &mut ActionInput) {
        self.held.fill(false);
        self.pulses.fill(false);
        self.codes.clear(input);
    }

    /// Commits held keys, latched pulses, and pointer position into `input` at a frame boundary.
    pub fn sample(&mut self, bindings: &KeyBindings, input: &mut ActionInput) {
        let now = std::array::from_fn(|i| self.held[i] || self.pulses[i]);
        self.pulses.fill(false);
        self.codes.update(bindings, &now, input);
        input.set_pointer(self.pointer[0], self.pointer[1]);
    }
}

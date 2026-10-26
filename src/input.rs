//! Action-level input of the engine +4656 object; addresses refer to the documented binary hash.
//! Each frame 0x140055260 derives three flags for each of 24 actions from five bound input
//! codes (tables at +944 + 10 * action) and per-code frame counters (+1540 now, +2564 before).
//! Which physical keys are bound is not verified yet, so hosts that cannot map input supply
//! the default: no action active.

pub const ACTIONS: usize = 24;

/// Flags at +1232 + 12 * action: +0 released, +4 held, +8 pressed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionInput {
    /// A bound code was down on the previous frame and is up now.
    released: [bool; ACTIONS],
    /// A bound code is down.
    held: [bool; ACTIONS],
    /// A bound code went down on this frame (its counter is 1).
    pressed: [bool; ACTIONS],
    /// +868/+872: pointer position in script coordinates (constructor 0, 0).
    pointer: [i32; 2],
    /// Set by button keyboard navigation (0x140056530 plus SetCursorPos); the host moves
    /// the system cursor and takes it.
    warp: Option<[i32; 2]>,
}

impl ActionInput {
    /// Host update for one action; indices outside 0..24 are rejected.
    pub fn set(
        &mut self,
        action: usize,
        held: bool,
        pressed: bool,
        released: bool,
    ) -> Result<(), &'static str> {
        if action >= ACTIONS {
            return Err("input action outside 0..23");
        }
        self.held[action] = held;
        self.pressed[action] = pressed;
        self.released[action] = released;
        Ok(())
    }
    pub fn held(&self, action: usize) -> bool {
        self.held.get(action).copied().unwrap_or(false)
    }
    pub fn pressed(&self, action: usize) -> bool {
        self.pressed.get(action).copied().unwrap_or(false)
    }
    pub fn released(&self, action: usize) -> bool {
        self.released.get(action).copied().unwrap_or(false)
    }
    pub fn pointer(&self) -> [i32; 2] {
        self.pointer
    }
    /// Host update of the pointer in script coordinates.
    pub fn set_pointer(&mut self, x: i32, y: i32) {
        self.pointer = [x, y];
    }
    /// Pending system cursor move requested by the engine, cleared when taken.
    pub fn take_warp(&mut self) -> Option<[i32; 2]> {
        self.warp.take()
    }
    pub(crate) fn warp_pointer(&mut self, x: i32, y: i32) {
        self.pointer = [x, y];
        self.warp = Some([x, y]);
    }
    /// 0x140054d90 for action 0 (siblings such as 0x140054dd0 for later actions): clears the
    /// release and press flags and keeps the held flag.
    pub fn consume(&mut self, action: usize) {
        if action < ACTIONS {
            self.released[action] = false;
            self.pressed[action] = false;
        }
    }
}

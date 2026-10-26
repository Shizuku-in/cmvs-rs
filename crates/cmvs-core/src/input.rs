//! Action-level input of the engine +4656 object; addresses refer to the documented binary hash.
//! Each frame 0x140055260 derives three flags for each of 24 actions from five bound input
//! codes (tables at +944 + 10 * action) and per-code frame counters (+1540 now, +2564 before).
//! Which physical keys are bound is not verified yet, so hosts that cannot map input supply
//! the default: no action active.
use crate::{bytecode::DecodeError, machine::Machine, vm::SystemResult};

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
    /// 0x140054e20 / 0x140054e30 read +1320 / +1332: action 7 or 8 held. 0x2224 and 0x22ef
    /// treat it like the script skip flag.
    pub fn skip_held(&self) -> bool {
        self.held(7) || self.held(8)
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

/// 0x21a0 (0x140078b10) / 0x21a2 (0x140078b90): system result 0 = action 0 / 1 released,
/// system result 4 (+82832) = held. 0x21a1 / 0x21a3 (inline 0x140054d90 / 0x140054dd0)
/// consume action 0 / 1. All return 0x4000 and pop nothing.
pub fn try_handle(vm: &mut Machine<'_>, input: &mut ActionInput) -> Result<bool, DecodeError> {
    let Some((_, command @ 0x1a0..=0x1a3)) = vm.pending_system() else {
        return Ok(false);
    };
    let action = usize::from(command >= 0x1a2);
    vm.complete_system(SystemResult(0x4000))?;
    if command & 1 == 0 {
        let state = vm.state_mut();
        state
            .set_system_integer(0, u32::from(input.released(action)))
            .expect("slot 0 exists");
        state
            .set_system_integer(4, u32::from(input.held(action)))
            .expect("slot 4 exists");
    } else {
        input.consume(action);
    }
    Ok(true)
}

/// 0x22ef / 0x14007c8e0: system result 0 = 1 when the +1520 script skip flag (0x22eb) is set
/// or action 7 or 8 is held, else 0; returns 0x4000 and pops nothing.
pub fn try_handle_skipping(
    vm: &mut Machine<'_>,
    input: &ActionInput,
    skip: bool,
) -> Result<bool, DecodeError> {
    let Some((_, 0x2ef)) = vm.pending_system() else {
        return Ok(false);
    };
    vm.complete_system(SystemResult(0x4000))?;
    vm.state_mut()
        .set_system_integer(0, u32::from(skip || input.skip_held()))
        .expect("slot 0 exists");
    Ok(true)
}

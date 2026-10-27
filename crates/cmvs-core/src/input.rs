//! Action-level input of the engine +4656 object; addresses refer to the documented binary hash.
//! Each frame 0x140055260 derives three flags for each of 24 actions from five bound input
//! codes (tables at +944 + 10 * action) and per-code frame counters (+1540 now, +2564 before).
//! KeyBindings reads the verified key.cfg bank; hosts without native input supply the
//! default: no action active. Physical event translation stays in the desktop backend.
use crate::{bytecode::DecodeError, machine::Machine, vm::SystemResult};

pub const ACTIONS: usize = 24;

/// The first 24 KEY_FUNCTION sections of key.cfg (0x140057740).
/// Registry overrides and the second bank (51..) are deliberately not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyBindings([[u8; 5]; ACTIONS]);

impl KeyBindings {
    pub fn codes(&self) -> &[[u8; 5]; ACTIONS] {
        &self.0
    }
    /// Bounded ASCII INI subset. Missing entries and out-of-range input codes are zero,
    /// as in the original profile reader. Malformed relevant values are errors.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.len() > 65536 || !text.is_ascii() {
            return Err("key.cfg must be ASCII and at most 64 KiB");
        }
        let mut result = Self::default();
        let mut section = None;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with([';', '#']) || line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                section = name
                    .to_ascii_uppercase()
                    .strip_prefix("KEY_FUNCTION_")
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| (1..=ACTIONS).contains(v))
                    .map(|v| v - 1);
            } else if let Some(action) = section
                && let Some((key, value)) = line.split_once('=')
                && let Some(slot) = key
                    .trim()
                    .to_ascii_uppercase()
                    .strip_prefix("KEY_DATA_")
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| (1..=5).contains(v))
            {
                let value = value
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| "invalid key.cfg input code")?;
                result.0[action][slot - 1] = if (0..128).contains(&value) {
                    value as u8
                } else {
                    0
                };
            }
        }
        Ok(result)
    }
}

/// Per-code history, independent of consumed action flags. One update per VM frame,
/// never per instruction or cooperative execution slice (0x140055260/0x140056780).
#[derive(Debug, Clone)]
pub struct CodeInput {
    previous: [bool; 128],
}
impl Default for CodeInput {
    fn default() -> Self {
        Self {
            previous: [false; 128],
        }
    }
}
impl CodeInput {
    pub fn update(&mut self, bindings: &KeyBindings, now: &[bool; 128], input: &mut ActionInput) {
        for (action, codes) in bindings.0.iter().enumerate() {
            let mut held = false;
            let mut pressed = false;
            let mut released = false;
            for &code in codes.iter().filter(|&&c| c != 0) {
                let code = usize::from(code);
                held |= now[code];
                pressed |= now[code] && !self.previous[code];
                released |= !now[code] && self.previous[code];
            }
            input
                .set(action, held, pressed, released)
                .expect("bounded action");
        }
        self.previous = *now;
    }
    /// Focus loss cancels state without turning it into a click on release.
    pub fn clear(&mut self, input: &mut ActionInput) {
        self.previous.fill(false);
        *input = ActionInput::default();
        input.set_pointer(-1, -1);
    }
}

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

/// Action of each 0x21a0 + 2 * n command pair, from the getter/consumer offsets
/// (+1232 + 12 * action). 0x21a5/0x21a7 and 0x21be/0x21bf fall into the dispatcher's shared
/// default case 0x1400823e7 and stay unhandled.
const ACTION_COMMANDS: [(u16, usize); 15] = [
    (0x1a0, 0),
    (0x1a2, 1),
    (0x1a4, 7),
    (0x1a6, 8),
    (0x1a8, 6),
    (0x1aa, 9),
    (0x1ac, 10),
    (0x1ae, 11),
    (0x1b0, 12),
    (0x1b2, 13),
    (0x1b4, 14),
    (0x1b6, 2),
    (0x1b8, 3),
    (0x1ba, 4),
    (0x1bc, 5),
];

/// Even commands of ACTION_COMMANDS (0x140078b10 and identical siblings up to 0x140078fd0):
/// system result 0 = action released, system result 4 (+82832) = held. The following odd
/// command (inline 0x140054d90 and siblings) consumes the action; actions 7 and 8 have none.
/// All return 0x4000 and pop nothing.
pub fn try_handle(vm: &mut Machine<'_>, input: &mut ActionInput) -> Result<bool, DecodeError> {
    let Some((_, command)) = vm.pending_system() else {
        return Ok(false);
    };
    let Some(&(_, action)) = ACTION_COMMANDS.iter().find(|&&(query, action)| {
        command == query || (command == query + 1 && !matches!(action, 7 | 8))
    }) else {
        return Ok(false);
    };
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

//! Action-level input of the engine +4656 object; addresses refer to the documented binary hash.
//! Each frame 0x140055260 derives three flags for each of 24 actions from five bound input
//! codes (tables at +944 + 10 * action) and per-code frame counters (+1540 now, +2564 before).
//! KeyBindings reads the verified key.cfg bank; hosts without native input supply the
//! default: no action active. Physical event translation stays in the desktop backend.
use crate::{bytecode::DecodeError, dispatch::Call, machine::Machine};

pub const ACTIONS: usize = 24;
/// Input codes behind the mouse flags at +1184 + 12 * i (0x140055260): left button, right
/// button, wheel up and wheel down.
pub const MOUSE_CODES: [usize; 4] = [1, 3, 4, 5];

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
        for (flags, code) in input.mouse.iter_mut().zip(MOUSE_CODES) {
            *flags = [
                !now[code] && self.previous[code],
                now[code],
                now[code] && !self.previous[code],
            ];
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
    /// +1184 + 12 * i: released, held and pressed for each of `MOUSE_CODES`, read by the
    /// pointer gesture recognizer.
    mouse: [[bool; 3]; 4],
    /// +868/+872: pointer position in script coordinates (constructor 0, 0).
    pointer: [i32; 2],
    /// Set by button keyboard navigation (0x140056530 plus SetCursorPos); the host moves
    /// the system cursor and takes it.
    warp: Option<[i32; 2]>,
    /// +876/+880: the pointer at the previous per-frame update (0x140056530).
    last_pointer: [i32; 2],
    /// +936: the pointer moved at the last per-frame update or navigation warp.
    pointer_moved: bool,
    /// +0x5fc: function key reporting, enabled by 0x234d (no command disables it).
    function_keys: bool,
    /// +0x600: while enabled, the function key 1..12 whose VK_F1..VK_F12 hold counter is 1
    /// this frame (the lowest one when several are), else 0; 0x234f clears it.
    function_key: u32,
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
    /// Host update per frame (0x140056780): the function keys F1..F12 that went down on this
    /// frame, by bit (bit 0 = F1). Ignored until a script enables reporting with 0x234d.
    pub fn press_function_keys(&mut self, pressed: u16) {
        if self.function_keys {
            self.function_key = (0..12)
                .find(|bit| pressed & (1 << bit) != 0)
                .map_or(0, |bit| bit + 1);
        }
    }
    /// Pending system cursor move requested by the engine, cleared when taken.
    pub fn take_warp(&mut self) -> Option<[i32; 2]> {
        self.warp.take()
    }
    pub(crate) fn warp_pointer(&mut self, x: i32, y: i32) {
        self.pointer = [x, y];
        self.warp = Some([x, y]);
        self.latch_pointer();
    }
    /// 0x140056530 once per frame before the VM run (and on navigation warps): +936 = the
    /// pointer differs from the previous update, which it then replaces.
    pub fn latch_pointer(&mut self) {
        self.pointer_moved = self.pointer != self.last_pointer;
        self.last_pointer = self.pointer;
    }
    /// Released, held and pressed flags of `MOUSE_CODES[index]`.
    pub fn mouse(&self, index: usize) -> [bool; 3] {
        self.mouse.get(index).copied().unwrap_or_default()
    }
    /// 0x140054bf0: clears every pressed flag (mouse and actions) and every action release,
    /// keeping held flags and the mouse releases.
    pub(crate) fn clear_edges(&mut self) {
        for flags in &mut self.mouse {
            flags[2] = false;
        }
        self.released = [false; ACTIONS];
        self.pressed = [false; ACTIONS];
    }
    /// 0x140059560: also clears the four mouse releases before 0x140054bf0.
    pub(crate) fn clear_all_edges(&mut self) {
        for flags in &mut self.mouse {
            flags[0] = false;
        }
        self.clear_edges();
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

/// Action of each 0x21a0 + 2 * n and 0x2344..0x234b command pair, from the getter/consumer
/// offsets (+1232 + 12 * action). 0x21a5/0x21a7, 0x21be/0x21bf, 0x2341..0x2343 and 0x234c fall
/// into the dispatcher's shared case 0x1400823e7 and stay unhandled. So do 0x2348/0x2349
/// (action 21): the frame update 0x140019430 also consumes that action while +11152 is set,
/// which is not modeled.
const ACTION_COMMANDS: [(u16, usize); 20] = [
    (0x19c, 15),
    (0x19e, 16),
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
    (0x344, 22),
    (0x346, 23),
    (0x34a, 20),
];

/// Even commands of ACTION_COMMANDS (0x140078b10 and identical siblings up to 0x140078fd0;
/// 0x1400791b0, 0x140079200 and 0x1400792a0 for 0x2344/0x2346/0x234a): system result 0 =
/// action released, system result 4 (+82832) = held. The following odd command (inline
/// 0x140054d90 and siblings, 0x1400551c0/0x1400551f0/0x140055160 for the 0x234x pairs)
/// consumes the action; actions 7 and 8 have none. All return 0x4000 and pop nothing.
pub fn try_handle(vm: &mut Machine<'_>, input: &mut ActionInput) -> Result<bool, DecodeError> {
    let Some((pc, command)) = vm.pending_system() else {
        return Ok(false);
    };
    if !recognizes(command) {
        return Ok(false);
    }
    handle(&mut Call::new(vm, pc, command), input)?;
    Ok(true)
}

fn recognizes(command: u16) -> bool {
    matches!(command, 0xc8..=0xca | 0x19a | 0x19c..=0x1a4 | 0x1a6 | 0x1a8..=0x1bd | 0x344..=0x347 | 0x34a | 0x34b | 0x34d..=0x34f | 0x35c | 0x35d)
}

pub(crate) fn handle(call: &mut Call<'_, '_>, input: &mut ActionInput) -> Result<(), DecodeError> {
    let command = call.command;
    if matches!(command, 0xc8 | 0xc9) {
        // 0x140074ed0: system results 1/2 = pointer x/y (0x1400565a0); 0x140074f00: system
        // result 0 = +936 (0x1400565c0). Both return 0x4000.
        call.complete(0x4000)?;
        let state = call.vm.state_mut();
        if command == 0xc8 {
            let [x, y] = input.pointer();
            state
                .set_system_integer(1, x as u32)
                .expect("result exists");
            state
                .set_system_integer(2, y as u32)
                .expect("result exists");
        } else {
            state
                .set_system_integer(0, u32::from(input.pointer_moved))
                .expect("result exists");
        }
        return Ok(());
    }
    if matches!(command, 0x35c | 0x35d) {
        // 0x140078b60 / 0x140078be0: system result 0 = action 0 / 1 went down on this frame
        // (0x140054d80 / 0x140054dc0); 0x4000.
        let pressed = input.pressed(usize::from(command == 0x35d));
        call.complete(0x4000)?;
        call.vm
            .state_mut()
            .set_system_integer(0, u32::from(pressed))
            .expect("slot 0 exists");
        return Ok(());
    }
    if command == 0x19a {
        // Inline 0x14008370b: 0x140059560 on the +4656 input object, 0x4000.
        call.complete(0x4000)?;
        input.clear_all_edges();
        return Ok(());
    }
    if matches!(command, 0x34d..=0x34f) {
        // 0x140055230 sets +0x5fc, 0x140055240 reads +0x600 into system result 0 and
        // 0x140055250 clears it; all return 0x4000 and pop nothing.
        call.complete(0x4000)?;
        match command {
            0x34d => input.function_keys = true,
            0x34e => call
                .vm
                .state_mut()
                .set_system_integer(0, input.function_key)
                .expect("slot 0 exists"),
            _ => input.function_key = 0,
        }
        return Ok(());
    }
    if command == 0xca {
        // 0x140074f30: inclusive signed rectangle, no normalization or consumption.
        let mut rect = [0; 4];
        for (i, value) in rect.iter_mut().enumerate() {
            *value = call.argument((i + 1) * 4, ["left", "top", "width", "height"][i])? as i32;
        }
        let [left, top, width, height] = rect;
        let [x, y] = input.pointer();
        let hit =
            x >= left && x <= left.wrapping_add(width) && y >= top && y <= top.wrapping_add(height);
        call.complete(0x4010)?;
        call.vm
            .state_mut()
            .set_system_integer(0, u32::from(hit))
            .expect("result exists");
        return Ok(());
    }
    let Some(&(_, action)) = ACTION_COMMANDS.iter().find(|&&(query, action)| {
        command == query || (command == query + 1 && !matches!(action, 7 | 8))
    }) else {
        return Err(call.error("unregistered input command"));
    };
    call.complete(0x4000)?;
    if command & 1 == 0 {
        let state = call.vm.state_mut();
        state
            .set_system_integer(0, u32::from(input.released(action)))
            .expect("slot 0 exists");
        state
            .set_system_integer(4, u32::from(input.held(action)))
            .expect("slot 4 exists");
    } else {
        input.consume(action);
    }
    Ok(())
}

/// 0x22ef / 0x14007c8e0: system result 0 = 1 when the +1520 script skip flag (0x22eb) is set
/// or action 7 or 8 is held, else 0; returns 0x4000 and pops nothing.
pub fn try_handle_skipping(
    vm: &mut Machine<'_>,
    input: &ActionInput,
    skip: bool,
) -> Result<bool, DecodeError> {
    let Some((pc, 0x2ef)) = vm.pending_system() else {
        return Ok(false);
    };
    handle_skipping(&mut Call::new(vm, pc, 0x2ef), input, skip)?;
    Ok(true)
}

pub(crate) fn handle_skipping(
    call: &mut Call<'_, '_>,
    input: &ActionInput,
    skip: bool,
) -> Result<(), DecodeError> {
    call.complete(0x4000)?;
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(skip || input.skip_held()))
        .expect("slot 0 exists");
    Ok(())
}

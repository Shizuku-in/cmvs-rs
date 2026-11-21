//! Frame-level action state and configurable input-code bindings.
//!
//! [`KeyBindings`] reads the supported `key.cfg` bank. Hosts translate physical
//! input into [`ActionInput`] once per frame; command handlers consume its action
//! flags. The default has no active actions.

// Engine +4656: 0x140055260 derives three flags for each of 24 actions from
// five bound codes (table +944 + 10 * action) and frame counters (+1540/+2564).
use crate::{bytecode::DecodeError, dispatch::Call};

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
    /// Reads `key.cfg` (case-insensitively under `root`) when `profile.uses_key_cfg()` is
    /// true, or returns empty bindings for CVNS builds that use fixed virtual key codes.
    pub fn load_for_profile_with_fs(
        fs: &dyn crate::filesystem::HostFs,
        root: &std::path::Path,
        profile: &crate::profile::EngineProfile,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::load_with_policy(fs, root, profile, true)
    }

    /// Reads `key.cfg` under `root` with [`StdFs`](crate::filesystem::StdFs) when used by `profile`.
    #[cfg(feature = "std-fs")]
    pub fn load_for_profile(
        root: &std::path::Path,
        profile: &crate::profile::EngineProfile,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::load_for_profile_with_fs(&crate::filesystem::StdFs, root, profile)
    }

    /// Like [`Self::load_for_profile_with_fs`], returning empty bindings when `key.cfg` is absent.
    pub fn load_optional_for_profile_with_fs(
        fs: &dyn crate::filesystem::HostFs,
        root: &std::path::Path,
        profile: &crate::profile::EngineProfile,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::load_with_policy(fs, root, profile, false)
    }

    #[cfg(feature = "std-fs")]
    pub fn load_optional_for_profile(
        root: &std::path::Path,
        profile: &crate::profile::EngineProfile,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::load_optional_for_profile_with_fs(&crate::filesystem::StdFs, root, profile)
    }

    fn load_with_policy(
        fs: &dyn crate::filesystem::HostFs,
        root: &std::path::Path,
        profile: &crate::profile::EngineProfile,
        required: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        use std::io::Read as _;
        if !profile.uses_key_cfg() {
            return Ok(Self::default());
        }
        let path = match crate::filesystem::resolve_under_with_fs(fs, root, b"key.cfg") {
            Ok(path) => path,
            Err(error) if !required && error.is_not_found() => return Ok(Self::default()),
            Err(error) => return Err(Box::new(error)),
        };
        let mut text = String::new();
        fs.open_read(&path)?.take(65537).read_to_string(&mut text)?;
        Ok(Self::parse(&text)?)
    }
}

/// Direct code indices matching [`CodeInput`] / `cmvs-desktop` for CVNS hold counters:
/// Ctrl, Shift, Escape, Return, Space, PageUp, PageDown.
const CVNS_HOLD_CODES: [usize; 7] = [24, 23, 25, 22, 26, 27, 28];

/// Mapping from `CodeInput` key index to CVNS `0x2040` bit when triggered as a single-frame edge.
const CVNS_EDGE_BITS: [(usize, u32); 17] = [
    (21, 1 << 4),  // Tab
    (33, 1 << 8),  // Up
    (34, 1 << 9),  // Down
    (35, 1 << 10), // Left
    (36, 1 << 11), // Right
    (52, 1 << 12), // F1
    (53, 1 << 13), // F2
    (54, 1 << 14), // F3
    (55, 1 << 15), // F4
    (56, 1 << 16), // F5
    (57, 1 << 17), // F6
    (58, 1 << 18), // F7
    (59, 1 << 19), // F8
    (60, 1 << 20), // F9
    (61, 1 << 21), // F10
    (62, 1 << 22), // F11
    (63, 1 << 23), // F12
];

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
        // Wheel codes are per-frame pulses, including on consecutive frames.
        if now[4] {
            input.cvns_wheel = input.cvns_wheel.saturating_sub(1);
        }
        if now[5] {
            input.cvns_wheel = input.cvns_wheel.saturating_add(1);
        }
        input.cvns_middle_pressed = now[2] && !self.previous[2];
        input.cvns_edges = CVNS_EDGE_BITS
            .iter()
            .filter(|&&(code, _)| now[code] && !self.previous[code])
            .fold(0u32, |mask, &(_, bit)| mask | bit);
        for (counter, &code) in input.cvns_holds.iter_mut().zip(&CVNS_HOLD_CODES) {
            *counter = if now[code] {
                counter.saturating_add(1)
            } else {
                0
            };
        }
        for (counter, code) in input.arrow_holds.iter_mut().zip([35, 36]) {
            *counter = if now[code] {
                counter.saturating_add(1)
            } else {
                0
            };
        }
        let fn_pressed = (0..12).fold(0u16, |mask, bit| {
            mask | (u16::from(now[52 + bit] && !self.previous[52 + bit]) << bit)
        });
        input.press_function_keys(fn_pressed);
        self.previous = *now;
    }
    pub fn forget_primary_button(&mut self) {
        self.previous[1] = false;
    }

    pub fn forget_pointer(&mut self) {
        self.previous[1..=7].fill(false);
    }

    /// Focus loss cancels state without turning it into a click on release.
    pub fn clear(&mut self, input: &mut ActionInput) {
        self.previous.fill(false);
        *input = ActionInput::default();
        input.set_pointer(-1, -1);
    }
}

// Flags at +1232 + 12 * action: +0 released, +4 held, +8 pressed.
/// Per-frame held, pressed, and released actions plus logical pointer state.
///
/// Refresh once per frame, then retain it across VM budget slices. Physical event
/// translation and key bindings belong to the host.
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
    /// CVNS +0xabc (this+687): accumulated wheel notches (-1 up, +1 down), reset by 0x2039.
    cvns_wheel: i32,
    /// CVNS this+426: middle mouse button click pulse for 0x203d / 0x2042(3).
    cvns_middle_pressed: bool,
    /// CVNS `sub_46EEE0` non-counter single-frame press bits (Tab, arrows, F1..F12).
    cvns_edges: u32,
    /// CVNS `sub_46E800` hold frame counters for Ctrl, Shift, Esc, Enter, Space, PgUp, PgDn.
    cvns_holds: [u32; 7],
    /// Left and right hold durations for the in-game RGB sliders.
    arrow_holds: [u32; 2],
    /// Positive hit rectangles queried via `0x20ca` / `0x2031` during the current frame.
    hit_rects: Vec<[i32; 4]>,
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
    pub fn hit_rects(&self) -> &[[i32; 4]] {
        &self.hit_rects
    }
    pub(crate) fn clear_hit_rects(&mut self) {
        self.hit_rects.clear();
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
        self.cvns_middle_pressed = false;
        self.cvns_edges = 0;
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
    pub(crate) fn consume_mouse_press(&mut self, index: usize) {
        if let Some(flags) = self.mouse.get_mut(index) {
            flags[2] = false;
        }
    }
    /// True when CVNS Ctrl hold (`0x100`) or standard skip hold is active (`sub_41cf90`).
    pub(crate) fn cvns_skip_held(&self) -> bool {
        self.skip_held() || self.cvns_holds[0] != 0
    }
    /// CVNS `sub_405960`: left click (`this+1680`) or Return edge (`this+1724 == 1`) ends
    /// modal `0x20c0` movie playback and consumes the edge (`1680 = 0`, `1724 = 2`).
    pub(crate) fn take_cvns_movie_skip(&mut self) -> bool {
        let hit = self.mouse[0][2] || self.cvns_holds[3] == 1;
        if hit {
            self.mouse[0][2] = false;
            if self.cvns_holds[3] == 1 {
                self.cvns_holds[3] = 2;
            }
        }
        hit
    }
    pub(crate) fn repeat_horizontal(&self) -> i8 {
        // Give single presses pixel precision and accelerate sustained adjustment.
        let [left, right] = self.arrow_holds.map(|frames| {
            if frames >= 18 && frames.is_multiple_of(2) {
                4
            } else {
                0
            }
        });
        right - left
    }
    /// CVNS `sub_46f6e0` wait interrupt mask check with optional edge consumption.
    pub(crate) fn cvns_wait_interrupt(&mut self, mask: u32, consume: bool) -> bool {
        let mut hit = false;
        if mask & 1 != 0 && self.cvns_wheel < 0 {
            if consume {
                self.cvns_wheel = 0;
            }
            hit = true;
        }
        if mask & 2 != 0 && self.cvns_wheel > 0 {
            if consume {
                self.cvns_wheel = 0;
            }
            hit = true;
        }
        // Bits 2..8 correspond to Ctrl(0), Shift(1), Esc(2), Enter(3), Space(4), PgUp(5).
        const HOLD_MASKS: [(usize, u32); 6] = [
            (0, 1 << 2),
            (1, 1 << 3),
            (2, 1 << 4),
            (3, 1 << 5),
            (4, 1 << 6),
            (5, 1 << 7),
        ];
        for &(slot, bit) in &HOLD_MASKS {
            if mask & bit != 0 && self.cvns_holds[slot] == 1 {
                if consume {
                    self.cvns_holds[slot] = 2;
                }
                hit = true;
            }
        }
        let edge_bits = mask & 0x00FF_FF00;
        if self.cvns_edges & edge_bits != 0 {
            if consume {
                self.cvns_edges &= !edge_bits;
            }
            hit = true;
        }
        if mask & (1 << 26) != 0 && self.mouse[0][2] {
            if consume {
                self.mouse[0][2] = false;
            }
            hit = true;
        }
        if mask & (1 << 27) != 0 && self.mouse[1][2] {
            if consume {
                self.mouse[1][2] = false;
            }
            hit = true;
        }
        hit
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

/// Inspected early x86 queries report the down edge, unlike the base build's up edge.
/// The additional 0x22f0/2/4 pairs address actions 17/18/19; docs/games/{asukimi,harouse}.
pub(crate) fn handle_press(
    call: &mut Call<'_>,
    input: &mut ActionInput,
) -> Result<(), DecodeError> {
    let command = call.command;
    let action = if (0x2f0..=0x2f5).contains(&command) {
        17 + usize::from((command - 0x2f0) / 2)
    } else {
        ACTION_COMMANDS
            .iter()
            .find(|&&(query, _)| query == command)
            .map(|&(_, action)| action)
            .ok_or_else(|| call.error("unregistered press query"))?
    };
    call.complete(0x4000)?;
    if command & 1 == 0 {
        let state = call.vm.state_mut();
        state
            .set_system_integer(0, u32::from(input.pressed(action)))
            .expect("slot 0 exists");
        state
            .set_system_integer(4, u32::from(input.held(action)))
            .expect("slot 4 exists");
    } else {
        input.consume(action);
    }
    Ok(())
}

pub(crate) fn handle(call: &mut Call<'_>, input: &mut ActionInput) -> Result<(), DecodeError> {
    let command = call.command;
    if try_handle_cvns(call, input)?.is_some() {
        return Ok(());
    }
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
        if width > 0
            && height > 0
            && input.hit_rects.len() < 256
            && !input.hit_rects.contains(&rect)
        {
            input.hit_rects.push(rect);
        }
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

pub(crate) fn handle_skipping(
    call: &mut Call<'_>,
    input: &ActionInput,
    skip: bool,
) -> Result<(), DecodeError> {
    let cvns_ctrl = call.command == 0x447 && input.cvns_holds[0] != 0;
    call.complete(0x4000)?;
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(skip || input.skip_held() || cvns_ctrl))
        .expect("slot 0 exists");
    Ok(())
}

/// CVNS input/pointer handlers (`0x433, 0x438..=0x43b, 0x43d, 0x440, 0x442`), matching
/// `cvns.exe` `0x45dc90..0x45dfa5`, `0x46e800`, `0x46eee0`, and `0x46fc80`.
fn try_handle_cvns(
    call: &mut Call<'_>,
    input: &mut ActionInput,
) -> Result<Option<()>, DecodeError> {
    let result = match call.command {
        0x433 => {
            let x = call.argument(4, "x")? as i32;
            let y = call.argument(8, "y")? as i32;
            call.complete(0x4008)?;
            input.warp_pointer(x, y);
            Some(())
        }
        0x438 | 0x43a | 0x43c | 0x43d => {
            let value = match call.command {
                0x438 => input.cvns_wheel as u32,
                0x43a => 0,
                0x43c => 1,
                _ => u32::from(input.cvns_middle_pressed),
            };
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            Some(())
        }
        0x439 | 0x43b => {
            if call.command == 0x439 {
                input.cvns_wheel = 0;
            }
            call.complete(0x4000)?;
            Some(())
        }
        0x440 => {
            let peek = call.argument(4, "peek")? != 0;
            let mut mask = input.cvns_edges;
            if input.mouse[0][2] {
                mask |= 1;
            }
            if input.mouse[1][2] {
                mask |= 2;
            }
            const HOLD_BITS: [(usize, u32); 5] = [
                (3, 1 << 2),
                (4, 1 << 3),
                (2, 1 << 5),
                (5, 1 << 6),
                (6, 1 << 7),
            ];
            for &(slot, bit) in &HOLD_BITS {
                if input.cvns_holds[slot] == 1 {
                    mask |= bit;
                }
            }
            if input.mouse[0][1] {
                mask |= 1 << 24;
            }
            if input.mouse[1][1] {
                mask |= 1 << 25;
            }
            call.complete(0x4004)?;
            if !peek {
                for &(slot, _) in &HOLD_BITS {
                    if input.cvns_holds[slot] == 1 {
                        input.cvns_holds[slot] = 2;
                    }
                }
                input.mouse[0][2] = false;
                input.mouse[1][2] = false;
                input.cvns_edges = 0;
            }
            call.vm
                .state_mut()
                .set_system_integer(0, mask)
                .expect("slot 0 exists");
            Some(())
        }
        0x442 => {
            let code = call.argument(4, "input code")?;
            let value = match code {
                1 => u32::from(input.mouse[0][2]),
                2 => u32::from(input.mouse[1][2]),
                3 => u32::from(input.cvns_middle_pressed),
                0x100..=0x106 => input.cvns_holds[(code - 0x100) as usize],
                _ => 0,
            };
            call.complete(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            Some(())
        }
        0x46c | 0x46e => {
            call.complete(0x4000)?;
            input.mouse[0][2] = call.command == 0x46c;
            Some(())
        }
        _ => None,
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Machine;

    fn vm_pending(command: u16, args: &[u32]) -> (Machine, usize) {
        let mut code = Vec::new();
        for &value in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend((0x2000 | (command & 0x7ff)).to_le_bytes());
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(&code);
        let mut raw = b"PS2A".to_vec();
        for n in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            0,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        let script = Box::leak(Box::new(crate::ps3::Script::decode(&raw).unwrap()));
        let mut vm = Machine::new(script).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        (vm, pc)
    }

    #[test]
    fn cvns_wheel_counts_consecutive_pulses_in_both_directions() {
        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();
        let mut input = ActionInput::default();
        for (code, expected) in [(4, -2), (5, 2)] {
            let mut now = [false; 128];
            now[code] = true;
            codes.update(&bindings, &now, &mut input);
            codes.update(&bindings, &now, &mut input);
            assert_eq!(input.cvns_wheel, expected);
            let (mut vm, pc) = vm_pending(0x439, &[]);
            try_handle_cvns(&mut Call::new(&mut vm, pc, 0x439), &mut input).unwrap();
        }
    }

    #[test]
    fn cvns_wheel_middle_mask_and_holds_match_winproc_contract() {
        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();
        let mut input = ActionInput::default();
        let mut now = [false; 128];
        now[1] = true; // left click
        now[2] = true; // middle click
        now[4] = true; // wheel up (-1)
        now[24] = true; // Ctrl
        now[22] = true; // Enter
        now[52] = true; // F1
        codes.update(&bindings, &now, &mut input);

        // 0x438 query wheel == -1, 0x439 resets to 0.
        let (mut vm, pc) = vm_pending(0x438, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x438), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
        let (mut vm, pc) = vm_pending(0x439, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x439), &mut input).unwrap();
        let (mut vm, pc) = vm_pending(0x438, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x438), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(0));

        // 0x43d middle click, 0x447 skip via Ctrl.
        let (mut vm, pc) = vm_pending(0x43d, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x43d), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1));
        let (mut vm, pc) = vm_pending(0x447, &[]);
        handle_skipping(&mut Call::new(&mut vm, pc, 0x447), &input, false).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1));

        // 0x440 peek=1 preserves bits, peek=0 consumes edges and bumps Enter hold to 2.
        let expected = 1 | (1 << 2) | (1 << 12) | (1 << 24);
        let (mut vm, pc) = vm_pending(0x440, &[1]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x440), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        let (mut vm, pc) = vm_pending(0x440, &[0]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x440), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(expected));
        let (mut vm, pc) = vm_pending(0x440, &[1]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x440), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1 << 24));
        let (mut vm, pc) = vm_pending(0x442, &[0x103]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x442), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(2));

        // 0x46c / 0x46e set and clear the left mouse click pulse (`mouse[0][2]`).
        let (mut vm, pc) = vm_pending(0x46e, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x46e), &mut input).unwrap();
        assert!(!input.mouse[0][2]);
        let (mut vm, pc) = vm_pending(0x46c, &[]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x46c), &mut input).unwrap();
        assert!(input.mouse[0][2]);
        let (mut vm, pc) = vm_pending(0x442, &[1]);
        try_handle_cvns(&mut Call::new(&mut vm, pc, 0x442), &mut input).unwrap();
        assert_eq!(vm.state().system_integer(0), Some(1));
    }

    #[cfg(feature = "std-fs")]
    #[test]
    fn load_for_profile_skips_absent_key_cfg_on_cvns_and_resolves_uppercase_on_cmvs() {
        let registry = crate::profile::Registry::builtin();
        let cvns = registry.profile("cvns-20090203").unwrap();
        let cmvs = registry.profile("cmvs-20160201").unwrap();
        assert!(!cvns.uses_key_cfg());
        assert!(cmvs.uses_key_cfg());

        let root = std::env::temp_dir().join(format!("cmvs-key-cfg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        assert_eq!(
            KeyBindings::load_for_profile(&root, cvns).unwrap(),
            KeyBindings::default()
        );
        assert!(KeyBindings::load_for_profile(&root, cmvs).is_err());

        let cfg = "[KEY_FUNCTION_1]\nKEY_DATA_1=22\n[KEY_FUNCTION_8]\nKEY_DATA_2=24\n";
        std::fs::write(root.join("KEY.CFG"), cfg).unwrap();
        let loaded = KeyBindings::load_for_profile(&root, cmvs).unwrap();
        assert_eq!(loaded, KeyBindings::parse(cfg).unwrap());
        assert_eq!(loaded.codes()[0][0], 22);
        assert_eq!(loaded.codes()[7][1], 24);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn code_input_reports_f1_through_f12_rising_edges_without_hold_retrigger() {
        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();
        let mut input = ActionInput::default();

        let query_fn_key = |input: &mut ActionInput| {
            let (mut vm, pc) = vm_pending(0x34e, &[]);
            handle(&mut Call::new(&mut vm, pc, 0x34e), input).unwrap();
            vm.state().system_integer(0).unwrap()
        };

        // Ignored before 0x234d enables function-key reporting.
        let mut now = [false; 128];
        now[52] = true;
        codes.update(&bindings, &now, &mut input);
        assert_eq!(query_fn_key(&mut input), 0);
        now = [false; 128];
        codes.update(&bindings, &now, &mut input);

        // Enable reporting with 0x234d (command 0x34d).
        let (mut vm, pc) = vm_pending(0x34d, &[]);
        handle(&mut Call::new(&mut vm, pc, 0x34d), &mut input).unwrap();

        for bit in 0..12 {
            let code = 52 + bit;
            let expected = (bit + 1) as u32;

            // Rising edge reports 1..=12 for codes 52..64.
            now = [false; 128];
            now[code] = true;
            codes.update(&bindings, &now, &mut input);
            assert_eq!(query_fn_key(&mut input), expected);

            // Next frame while still held does not re-trigger.
            codes.update(&bindings, &now, &mut input);
            assert_eq!(query_fn_key(&mut input), 0);

            // Release also reports 0.
            now = [false; 128];
            codes.update(&bindings, &now, &mut input);
            assert_eq!(query_fn_key(&mut input), 0);
        }

        // With F1 already held, simultaneous rising edges on F4 and F9 report lowest new edge (F4),
        // then clear on the following held frame.
        now[52] = true;
        codes.update(&bindings, &now, &mut input);
        assert_eq!(query_fn_key(&mut input), 1);
        now[55] = true;
        now[60] = true;
        codes.update(&bindings, &now, &mut input);
        assert_eq!(query_fn_key(&mut input), 4);
        codes.update(&bindings, &now, &mut input);
        assert_eq!(query_fn_key(&mut input), 0);
    }
}

//! Engine +4664 pointer drag/flick recognizer (constructor 0x1400533d0). Scripts configure
//! it (0x2350..0x2364, plus six region records through 0x2366) and query the per-frame
//! hold result (0x2362). The frame update
//! 0x140054a60 runs mode 0 (0x140053930) or region drag mode 1 (0x1400541a0).
//! Mode 2 (0x140054570) remains a barrier. Fields stay named by offset.
use crate::{
    bytecode::DecodeError, evaluate::error, input::ActionInput, machine::Machine, vm::SystemResult,
};
use std::collections::BTreeMap;
mod drag;

/// Tracking fields reset by every setter that shares the tail of 0x1400548e0.
const RESET_MINUS_ONE: [u16; 3] = [44, 88, 172];
const RESET_ZERO: [u16; 11] = [48, 52, 72, 76, 80, 84, 92, 108, 120, 180, 184];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerGesture {
    fields: BTreeMap<u16, u32>,
}

impl Default for PointerGesture {
    /// Constructor values for every field the modeled commands and mode 0 touch: +32 = 64
    /// (distance threshold), +36 = 90, +96 = 700 (hold time), +112 = 1500 (rest time);
    /// the memset leaves the anchors +56..+68 and +100/+104 zero.
    fn default() -> Self {
        let mut fields = BTreeMap::from([
            (32, 64),
            (36, 90),
            (40, 4),
            (96, 700),
            (112, 1500),
            (116, 1),
        ]);
        for offset in [0, 4, 8, 12, 16, 20, 24, 28, 56, 60, 64, 68, 100, 104]
            .into_iter()
            .chain([176])
            .chain((124..=168).step_by(4))
            .chain(RESET_ZERO)
            .chain((188..=304).step_by(4))
        {
            fields.insert(offset, 0);
        }
        for offset in RESET_MINUS_ONE {
            fields.insert(offset, u32::MAX);
        }
        Self { fields }
    }
}

/// Mouse flag indices of `ActionInput::mouse`.
const LEFT: usize = 0;
const RIGHT: usize = 1;
const WHEEL_UP: usize = 2;
const WHEEL_DOWN: usize = 3;
/// `[released, held, pressed]` positions.
const RELEASED: usize = 0;
const HELD: usize = 1;
const PRESSED: usize = 2;

/// 0x140008ab0 with every coordinate multiplied by 100 (i32): the angle is
/// trunc(atan2(dy, dx) / π · −180) normalized to 0..359 as 360 − a, and the distance is
/// trunc(dx / cos θ), or |dy| when dx is 0. The CRT atan2/cos are approximated by Rust's.
/// The callers fold the angle with (|a| + 135) % 360 and divide the distance by 100.
fn measure(from: [i32; 2], to: [i32; 2]) -> (i32, i32) {
    let scaled = |v: i32| v.wrapping_mul(100);
    let dx = scaled(to[0]).wrapping_sub(scaled(from[0]));
    let dy = scaled(to[1]).wrapping_sub(scaled(from[1]));
    let theta = f64::from(dy).atan2(f64::from(dx));
    let mut angle = (theta / std::f64::consts::PI * -180.0) as i32;
    if angle < 0 {
        angle += 360;
    }
    angle = 360 - angle;
    if angle == 360 {
        angle = 0;
    }
    let distance = if dx != 0 {
        (f64::from(dx) / theta.cos()) as i32
    } else {
        dy.wrapping_abs()
    };
    ((angle.wrapping_abs() + 135) % 360, distance / 100)
}

impl PointerGesture {
    pub fn field(&self, offset: u16) -> Option<u32> {
        self.fields.get(&offset).copied()
    }
    pub fn fields(&self) -> &BTreeMap<u16, u32> {
        &self.fields
    }
    fn get(&self, offset: u16) -> i32 {
        self.fields.get(&offset).copied().unwrap_or(0) as i32
    }
    fn set(&mut self, offset: u16, value: i32) {
        self.fields.insert(offset, value as u32);
    }
    fn reset_tracking(&mut self) {
        for offset in RESET_MINUS_ONE {
            self.fields.insert(offset, u32::MAX);
        }
        for offset in RESET_ZERO {
            self.fields.insert(offset, 0);
        }
    }
    /// The inline reset after 0x140059560 in 0x140053930: the same fields as the setters'
    /// reset, with the qword at +80 also clearing +84.
    fn reset_after_consume(&mut self, input: &mut ActionInput) {
        input.clear_all_edges();
        self.reset_tracking();
    }
    /// The disabled branch: +44/+88 = -1, +48/+52/+72/+76/+84/+92/+120 = 0.
    fn idle(&mut self) {
        self.set(44, -1);
        for offset in [48, 52, 72, 76, 84, 92, 120] {
            self.set(offset, 0);
        }
        self.set(88, -1);
    }
    /// One side of 0x140053930: the left button with +24/+48/+72/+56 and directions 0..3,
    /// or the right button with +28/+52/+76/+64 and directions 4..7.
    fn track(&mut self, input: &mut ActionInput, side: usize, wheel: i32, previous: i32, now: u32) {
        let (active, wheel_hold, anchor, base) = if side == LEFT {
            (48, 72, 56, 0)
        } else {
            (52, 76, 64, 4)
        };
        let flags = input.mouse(side);
        let pointer = input.pointer();
        if self.get(active) == 0 {
            if flags[PRESSED] {
                self.set(anchor, pointer[0]);
                self.set(anchor + 4, pointer[1]);
                self.set(44, -1);
                self.set(active, 1);
                self.set(92, now as i32);
                self.set(100, pointer[0]);
                self.set(104, pointer[1]);
                self.set(108, now as i32);
            }
            return;
        }
        if self.get(wheel_hold) == 0 && wheel != 0 {
            self.set(wheel_hold, 1);
        }
        if self.get(wheel_hold) != 0 {
            if flags[RELEASED] {
                input.clear_all_edges();
                self.set(44, -1);
                self.set(active, 0);
                self.set(wheel_hold, 0);
            } else if flags[HELD] {
                self.set(80, wheel);
                input.clear_edges();
            } else {
                self.set(44, -1);
                self.set(active, 0);
                self.set(wheel_hold, 0);
                self.set(80, 0);
            }
        } else if flags[RELEASED] {
            let (angle, distance) = measure([self.get(anchor), self.get(anchor + 4)], pointer);
            if distance < self.get(32) {
                if previous >= 0 {
                    input.clear_all_edges();
                }
            } else {
                self.set(44, (angle / 90).clamp(0, 3) + base);
                input.clear_all_edges();
            }
            self.set(active, 0);
        } else if !flags[HELD] {
            self.set(44, -1);
            self.set(active, 0);
        }
    }
    /// Frame update 0x140054a60 after the input refresh, preceded by 0x140053510 (+44 = -1).
    /// Mode +4 = 0 runs 0x140053930: +88 (the 0x2362 result) is -1 unless a press tracked by
    /// +48/+52 has lasted +96 ms without the two-button chord (+84). It then holds 98/99, or
    /// 20/21 after a wheel step during the hold, or a direction 0..3/4..7 once the pointer is
    /// at least +32 from the anchor. A release ends tracking; a far release sets the flick
    /// direction +44. Flicks, chord releases and wheel holds clear input edges
    /// (0x140059560 / 0x140054bf0). Pointer rest (+112 ms within 3 pixels) moves the anchor;
    /// as in the original, +100 and +104 then both receive the pointer y. `now` is the
    /// research millisecond clock standing in for 0x140008bc0. Modes 1 and 2 are rejected;
    /// other modes do nothing.
    pub(crate) fn frame(&mut self, input: &mut ActionInput, now: u32) -> Result<(), &'static str> {
        self.set(44, -1);
        match self.get(4) {
            0 => {}
            1 => {
                self.frame_drag(input, now);
                return Ok(());
            }
            2 => return Err("pointer gesture mode 2 is not modeled"),
            _ => return Ok(()),
        }
        let previous = self.get(88);
        self.set(80, 0);
        self.set(88, -1);
        if self.get(0) == 0 {
            self.idle();
            return Ok(());
        }
        let mut wheel = i32::from(input.mouse(WHEEL_UP)[RELEASED]);
        if input.mouse(WHEEL_DOWN)[RELEASED] {
            wheel = -1;
        }
        if self.get(84) == 0
            && input.mouse(LEFT)[HELD] as u32 + input.mouse(RIGHT)[HELD] as u32 >= 2
        {
            self.set(84, 1);
        }
        if self.get(84) != 0 {
            let mut held = u32::from(input.mouse(LEFT)[HELD]);
            if input.mouse(LEFT)[RELEASED] {
                self.reset_after_consume(input);
            }
            held += u32::from(input.mouse(RIGHT)[HELD]);
            if input.mouse(RIGHT)[RELEASED] {
                self.reset_after_consume(input);
            }
            if wheel != 0 {
                self.reset_after_consume(input);
            }
            if held == 0 {
                self.set(84, 0);
            }
        }
        if [8, 12, 16, 20].iter().any(|&offset| self.get(offset) == 0) {
            self.idle();
            return Ok(());
        }
        if self.get(24) != 0 {
            self.track(input, LEFT, wheel, previous, now);
        }
        if self.get(28) != 0 {
            self.track(input, RIGHT, wheel, previous, now);
        }
        let tracking = self.get(48) != 0 || self.get(52) != 0;
        if tracking {
            let [x, y] = input.pointer();
            if x.wrapping_sub(self.get(100)).wrapping_abs() >= 3
                || y.wrapping_sub(self.get(104)).wrapping_abs() >= 3
            {
                self.set(100, x);
                self.set(104, y);
                self.set(108, now as i32);
            }
            if now.wrapping_sub(self.get(108) as u32) >= self.get(112) as u32 {
                for (active, anchor) in [(48, 56), (52, 64)] {
                    if self.get(active) != 0 {
                        self.set(anchor, x);
                        self.set(anchor + 4, y);
                        self.set(44, -1);
                    }
                }
                self.set(100, y);
                self.set(104, y);
                self.set(108, now as i32);
            }
        }
        if self.get(84) == 0
            && tracking
            && now.wrapping_sub(self.get(92) as u32) >= self.get(96) as u32
        {
            let pointer = input.pointer();
            for (active, wheel_hold, anchor, plain, wheeled, base) in
                [(48, 72, 56, 98, 20, 0), (52, 76, 64, 99, 21, 4)]
            {
                if self.get(active) == 0 {
                    continue;
                }
                self.set(88, plain);
                if self.get(wheel_hold) != 0 {
                    self.set(88, wheeled);
                } else {
                    let (angle, distance) =
                        measure([self.get(anchor), self.get(anchor + 4)], pointer);
                    if distance >= self.get(32) {
                        self.set(88, (angle / 90 + base).clamp(base, base + 3));
                    }
                }
            }
        } else {
            self.set(88, -1);
        }
        Ok(())
    }
    /// 0x2350..0x2364 setters (0x14007fb60..0x14007fe60) on the +4664 object, and 0x2362 /
    /// 0x14007fe20 -> 0x140053610: system result 0 = (SP-4 == 1 and +88 >= 0); pops 4.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if matches!(command, 0x367 | 0x368) {
            vm.complete_system(SystemResult(0x4000))?;
            if command == 0x367 {
                self.set(172, -1);
            } else {
                for (slot, offset) in [(0, 172), (4, 168), (5, 164), (6, 176)] {
                    vm.state_mut()
                        .set_system_integer(slot, self.get(offset) as u32)
                        .expect("result exists");
                }
            }
            return Ok(true);
        }
        if command == 0x360 {
            // 0x14007fd10 -> 0x14001cf30: system result 0 = +88 (the 0x2362 state).
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, self.get(88) as u32)
                .expect("result exists");
            return Ok(true);
        }
        if command == 0x353 {
            // 0x14007fce0 -> 0x1400534c0: system result 0 = the flick direction +44.
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, self.get(44) as u32)
                .expect("result exists");
            return Ok(true);
        }
        if command == 0x359 {
            // 0x14007fde0 -> 0x1400534d0: +80 is cleared when +72 and +76 are both set or
            // both clear; result 0 = +80, result 4 = bool(+76). With both clear the original
            // leaves its out word uninitialized; +80 is then 0 and Rust writes 0 there too.
            let (first, second) = (self.get(72), self.get(76));
            if (first != 0) == (second != 0) {
                self.set(80, 0);
            }
            vm.complete_system(SystemResult(0x4000))?;
            for (slot, value) in [(0, self.get(80)), (4, i32::from(second != 0))] {
                vm.state_mut()
                    .set_system_integer(slot, value as u32)
                    .expect("result exists");
            }
            return Ok(true);
        }
        if command == 0x365 {
            vm.complete_system(SystemResult(0x4000))?;
            let active = self.get(120) != 0;
            vm.state_mut()
                .set_system_integer(0, u32::from(active))
                .expect("result exists");
            if active {
                for (slot, offset) in [(1, 124), (2, 128), (4, 100), (5, 104)] {
                    vm.state_mut()
                        .set_system_integer(slot, self.get(offset) as u32)
                        .expect("result exists");
                }
            }
            vm.state_mut()
                .set_system_integer(6, self.get(176) as u32)
                .expect("result exists");
            return Ok(true);
        }
        if command == 0x366 {
            // 0x14007fef0 -> 0x1400536d0: six region records at +188, stride 20.
            // The slot comparison is unsigned; out-of-range slots are ignored. This
            // setter keeps raw rectangle words, does not reset tracking or query input.
            let mut args = [0; 6];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = vm
                    .stack()
                    .peek_u32(4 * (i + 1))
                    .map_err(|e| error(pc, 0x2366, e.0))?;
            }
            vm.complete_system(SystemResult(0x4018))?;
            if args[0] < 6 {
                let base = 188 + args[0] as u16 * 20;
                for (i, value) in [u32::from(args[1] != 0), args[2], args[3], args[4], args[5]]
                    .into_iter()
                    .enumerate()
                {
                    self.fields.insert(base + i as u16 * 4, value);
                }
            }
            return Ok(true);
        }
        let flag_field = match command {
            0x350 => Some(0),
            0x352 => Some(8),
            0x356 => Some(12),
            0x357 => Some(16),
            0x358 => Some(20),
            0x351 | 0x35a | 0x35b | 0x362 | 0x364 => None,
            _ => return Ok(false),
        };
        let op = command + 0x2000;
        let value = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let first = value(4)?;
        let second = if command == 0x351 { value(8)? } else { 0 };
        let flags = if command == 0x351 { 0x4008 } else { 0x4004 };
        vm.complete_system(SystemResult(flags))?;
        match (command, flag_field) {
            (_, Some(offset)) => {
                self.fields.insert(offset, u32::from(first != 0));
                self.reset_tracking();
            }
            (0x351, _) => {
                self.fields.insert(24, u32::from(first != 0));
                self.fields.insert(28, u32::from(second != 0));
                self.reset_tracking();
            }
            (0x362, _) => {
                let held = first == 1 && self.get(88) >= 0;
                vm.state_mut()
                    .set_system_integer(0, u32::from(held))
                    .expect("slot 0 exists");
            }
            (0x364, _) => {
                self.fields.insert(4, first);
                self.reset_tracking();
            }
            // 0x1400535f0 accepts 1..=90 and 0x1400535e0 non-negative values; neither resets.
            (0x35a, _) if (1..=90).contains(&first) => {
                self.fields.insert(36, first);
            }
            (0x35b, _) if first as i32 >= 0 => {
                self.fields.insert(32, first);
            }
            _ => {}
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{CodeInput, KeyBindings};

    /// Recognition enabled with left and right tracking, as the real path configures it.
    fn enabled() -> PointerGesture {
        let mut gesture = PointerGesture::default();
        for offset in [0, 8, 12, 16, 20, 24, 28] {
            gesture.set(offset, 1);
        }
        gesture.set(32, 48);
        gesture
    }
    /// One frame: input codes down this frame, pointer, then the gesture update.
    struct Frames {
        codes: CodeInput,
        bindings: KeyBindings,
        input: ActionInput,
        now: u32,
    }
    impl Frames {
        fn new() -> Self {
            // Action 0 bound to the left button, to observe edge clearing.
            let bindings = KeyBindings::parse("[KEY_FUNCTION_1]\nKEY_DATA_1=1\n").unwrap();
            Self {
                codes: CodeInput::default(),
                bindings,
                input: ActionInput::default(),
                now: 0,
            }
        }
        fn step(&mut self, gesture: &mut PointerGesture, down: &[usize], at: [i32; 2], ms: u32) {
            let mut now = [false; 128];
            for &code in down {
                now[code] = true;
            }
            self.codes.update(&self.bindings, &now, &mut self.input);
            self.input.set_pointer(at[0], at[1]);
            self.now = self.now.wrapping_add(ms);
            gesture.frame(&mut self.input, self.now).unwrap();
        }
    }

    #[test]
    fn measure_folds_screen_directions_and_truncates_distance() {
        assert_eq!(measure([0, 0], [0, -50]), (45, 50));
        assert_eq!(measure([0, 0], [50, 0]), (135, 50));
        assert_eq!(measure([0, 0], [0, 50]), (225, 50));
        assert_eq!(measure([0, 0], [-50, 0]), (315, 50));
    }

    #[test]
    fn a_short_click_never_reports_and_keeps_input() {
        let mut gesture = enabled();
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [100, 100], 16);
        assert_eq!((gesture.get(48), gesture.get(88)), (1, -1));
        f.step(&mut gesture, &[], [100, 100], 16);
        assert_eq!(
            (gesture.get(48), gesture.get(44), gesture.get(88)),
            (0, -1, -1)
        );
        // The release of the bound action survives for the VM.
        assert!(f.input.released(0) && f.input.mouse(LEFT)[RELEASED]);
    }

    #[test]
    fn a_held_press_reports_98_then_a_direction_once_far_enough() {
        let mut gesture = enabled();
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [100, 100], 16);
        for _ in 0..43 {
            f.step(&mut gesture, &[1], [100, 100], 16);
        }
        // 43 frames after the press: 688 ms < 700; the next frame reaches 704.
        assert_eq!(gesture.get(88), -1);
        f.step(&mut gesture, &[1], [100, 100], 16);
        assert_eq!(gesture.get(88), 98);
        f.step(&mut gesture, &[1], [100, 160], 16);
        assert_eq!(gesture.get(88), 2);
        // The right button reports 99 and directions 4..7.
        let mut right = enabled();
        let mut f = Frames::new();
        f.step(&mut right, &[3], [0, 0], 16);
        f.step(&mut right, &[3], [-60, 0], 700);
        assert_eq!(right.get(88), 7);
    }

    #[test]
    fn a_far_release_sets_the_flick_direction_and_clears_edges() {
        let mut gesture = enabled();
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [100, 100], 16);
        f.step(&mut gesture, &[1], [130, 100], 16);
        f.step(&mut gesture, &[], [160, 100], 16);
        assert_eq!(
            (gesture.get(44), gesture.get(48), gesture.get(88)),
            (1, 0, -1)
        );
        assert!(!f.input.released(0) && !f.input.mouse(LEFT)[RELEASED]);
    }

    #[test]
    fn the_hold_query_needs_argument_one_and_a_nonnegative_result() {
        // 0x2362 [SP-4 = 1], 0x2362 [SP-4 = 2] in a minimal PS2A image.
        let mut code = vec![];
        for arg in [1u32, 2] {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
            code.extend(0x2362u16.to_le_bytes());
        }
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
        let script = crate::ps3::Script::decode(&raw).unwrap();
        let mut vm = Machine::new(&script).unwrap();
        let mut gesture = enabled();
        gesture.set(88, 98);
        for expected in [1, 0] {
            while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
            assert!(gesture.try_handle(&mut vm).unwrap());
            assert_eq!(vm.state().system_integer(0), Some(expected));
        }
    }

    #[test]
    fn disabled_recognition_idles_and_other_modes_are_explicit() {
        let mut gesture = enabled();
        gesture.set(16, 0);
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [100, 100], 16);
        assert_eq!((gesture.get(48), gesture.get(88)), (0, -1));
        let mut chord = enabled();
        let mut f = Frames::new();
        f.step(&mut chord, &[1, 3], [0, 0], 16);
        assert_eq!(chord.get(84), 1);
        f.step(&mut chord, &[1, 3], [0, 0], 800);
        // The two-button chord suppresses hold reports.
        assert_eq!(chord.get(88), -1);
        let mut other = enabled();
        other.set(4, 2);
        assert!(other.frame(&mut ActionInput::default(), 0).is_err());
        other.set(4, 3);
        assert!(other.frame(&mut ActionInput::default(), 0).is_ok());
        assert_eq!(other.get(44), -1);
    }

    fn drag() -> PointerGesture {
        let mut gesture = enabled();
        gesture.set(4, 1);
        for (at, value) in [(188, 1), (192, 0), (196, 0), (200, 300), (204, 300)] {
            gesture.set(at, value);
        }
        gesture
    }

    #[test]
    fn region_drag_preserves_short_clicks_and_only_starts_on_the_selected_axis() {
        let mut gesture = drag();
        gesture.set(116, 1);
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [20, 30], 16);
        f.step(&mut gesture, &[1], [120, 33], 16);
        assert_eq!(gesture.get(120), 0);
        f.step(&mut gesture, &[], [120, 33], 16);
        assert!(f.input.released(0));
        assert_eq!((gesture.get(48), gesture.get(120)), (0, 0));
        f.step(&mut gesture, &[1], [20, 30], 16);
        f.step(&mut gesture, &[1], [20, 34], 16);
        assert_eq!(
            (gesture.get(120), gesture.get(124), gesture.get(128)),
            (1, 0, 0)
        );
        f.step(&mut gesture, &[1], [22, 54], 16);
        assert_eq!((gesture.get(124), gesture.get(128)), (2, 20));
        f.step(&mut gesture, &[], [22, 60], 16);
        assert_eq!(
            (gesture.get(48), gesture.get(120), gesture.get(172)),
            (0, 0, 1)
        );
        assert_eq!((gesture.get(168), gesture.get(164)), (26, 32));
        assert!(!f.input.released(0) && !f.input.mouse(LEFT)[RELEASED]);
    }

    #[test]
    fn drag_regions_choose_last_match_and_idle_release_invalidates_small_flick() {
        let mut gesture = drag();
        gesture.set(116, 0);
        for (at, value) in [(288, 1), (292, 0), (296, 0), (300, 200), (304, 200)] {
            gesture.set(at, value);
        }
        let mut f = Frames::new();
        f.step(&mut gesture, &[1], [400, 400], 16);
        assert_eq!(gesture.get(48), 0);
        f.step(&mut gesture, &[], [400, 400], 16);
        f.step(&mut gesture, &[1], [20, 30], 16);
        assert_eq!(gesture.get(176), 5);
        f.step(&mut gesture, &[1], [24, 30], 16);
        f.step(&mut gesture, &[1], [25, 30], 100);
        f.step(&mut gesture, &[], [25, 30], 16);
        assert_eq!(gesture.get(172), -1);
        assert_eq!(gesture.get(168), 0);
        assert!(!f.input.released(0));
    }

    #[test]
    fn drag_query_only_updates_coordinates_when_active_and_always_returns_region() {
        let code = [0x65, 0x23, 0x65, 0x23];
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(code);
        let mut raw = b"PS2A".to_vec();
        for n in [48u32, 0, 0, 1, 4, 0, 0, 0, 0, payload.len() as u32, 0] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        let script = crate::ps3::Script::decode(&raw).unwrap();
        let mut vm = Machine::new(&script).unwrap();
        let mut gesture = drag();
        assert_eq!(gesture.get(116), 1);
        for (at, value) in [(124, -3), (128, 25), (100, 20), (104, 30), (176, 5)] {
            gesture.set(at, value);
        }
        for i in 0..7 {
            vm.state_mut().set_system_integer(i, 100 + i).unwrap();
        }
        for active in [false, true] {
            gesture.set(120, i32::from(active));
            assert!(matches!(
                vm.step().unwrap(),
                crate::machine::Step::System { .. }
            ));
            assert!(gesture.try_handle(&mut vm).unwrap());
            let expected = if active {
                [1, (-3i32) as u32, 25, 103, 20, 30, 5]
            } else {
                [0, 101, 102, 103, 104, 105, 5]
            };
            for (i, value) in expected.into_iter().enumerate() {
                assert_eq!(vm.state().system_integer(i as u32), Some(value));
            }
            assert_eq!(vm.stack().len_bytes(), 0);
        }
    }
}

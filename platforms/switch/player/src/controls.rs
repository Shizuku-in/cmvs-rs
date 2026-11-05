//! Switch controller and touch policy, translated to the engine's input codes
//! (0x140056780: 1/3 mouse buttons, 4/5 wheel, 22 Enter, 23 Shift, 24 Ctrl, 25 Esc,
//! 33..36 arrows). Which game action a code triggers still comes from key.cfg.
//!
//! The host polls more often than the engine samples input, so `poll` runs every
//! loop iteration and `frame` once at each engine input boundary. A press that
//! begins and ends between two frames is latched into one down frame.
use cmvs_core::{
    input::{ActionInput, CodeInput, KeyBindings},
    viewport::Viewport,
};
use cmvs_native::{SwitchInput, button};

const MOUSE_LEFT: usize = 1;
const WHEEL_UP: usize = 4;
const WHEEL_DOWN: usize = 5;
/// Button, engine code. The D-pad uses keyboard navigation; the left stick moves the
/// pointer. A clicks at the pointer; Y is Enter for the navigation selection.
const BUTTONS: [(u32, usize); 10] = [
    (button::A, MOUSE_LEFT),
    (button::B, 3),
    (button::X, 25),
    (button::Y, 22),
    (button::ZL, 23),
    (button::ZR, 24),
    (button::UP, 33),
    (button::DOWN, 34),
    (button::LEFT, 35),
    (button::RIGHT, 36),
];
const STICK_DEAD_ZONE: f64 = 6000.0;
const STICK_MAX: f64 = 32767.0;
/// Full deflection crosses the canvas width in this many milliseconds.
const CURSOR_CROSSING_MS: f64 = 1200.0;
/// Longer poll gaps (loading, suspension) do not move the pointer further.
const MAX_STEP_MS: u32 = 100;
const WHEEL_DELAY_MS: u32 = 400;
const WHEEL_REPEAT_MS: u32 = 100;
/// + must be held this long to quit, so a stray press does not end the session.
pub const QUIT_HOLD_MS: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TouchState {
    Idle,
    /// The contact that started a gesture inside the game area.
    Primary(u32),
    /// Ignored until every contact lifts: a gesture began outside the game area or
    /// the primary contact lifted while others remained.
    Blocked,
}

pub struct Controls {
    size: [u32; 2],
    history: CodeInput,
    held: [bool; 128],
    latched: [bool; 128],
    cursor: [f64; 2],
    cursor_visible: bool,
    touch: TouchState,
    wheel: Option<(usize, u32)>,
    wheel_next: u32,
    quit_ms: u32,
    focused: bool,
    /// Buttons ignored until released: held across a focus change or system applet.
    blocked: u32,
}

impl Controls {
    /// The pointer starts at the centre of the `size` script canvas.
    pub fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            history: CodeInput::default(),
            held: [false; 128],
            latched: [false; 128],
            cursor: [f64::from(size[0] / 2), f64::from(size[1] / 2)],
            cursor_visible: true,
            touch: TouchState::Idle,
            wheel: None,
            wheel_next: 0,
            quit_ms: 0,
            focused: true,
            blocked: 0,
        }
    }

    /// Losing focus cancels held input without producing release clicks; on return,
    /// buttons and contacts still held from the system UI are ignored until released.
    pub fn focus(&mut self, focused: bool, input: &mut ActionInput) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        self.suppress_held(input);
    }

    /// After a modal system applet: cancels held input without release clicks and
    /// ignores whatever is still held (such as the button that closed it) until released.
    pub fn suppress_held(&mut self, input: &mut ActionInput) {
        self.history.clear(input);
        self.held.fill(false);
        self.latched.fill(false);
        self.touch = TouchState::Blocked;
        self.wheel = None;
        self.quit_ms = 0;
        self.blocked = u32::MAX;
    }

    /// `viewport` maps 1280x720 panel pixels to the script canvas; `dt_ms` is the time
    /// since the previous poll.
    pub fn poll(&mut self, sample: &SwitchInput, viewport: Viewport, dt_ms: u32) {
        if !self.focused {
            return;
        }
        let dt_ms = dt_ms.min(MAX_STEP_MS);
        self.blocked &= sample.buttons;
        let buttons = sample.buttons & !self.blocked;
        self.held.fill(false);
        for (mask, code) in BUTTONS {
            self.held[code] = buttons & mask != 0;
        }
        self.track_touch(sample, viewport);
        self.held[MOUSE_LEFT] |= matches!(self.touch, TouchState::Primary(_));
        for (latched, held) in self.latched.iter_mut().zip(self.held) {
            *latched |= held;
        }
        self.move_cursor(sample.sticks[0], dt_ms);
        self.scroll(buttons, sample.sticks[1][1], dt_ms);
        self.quit_ms = if buttons & button::PLUS != 0 {
            self.quit_ms.saturating_add(dt_ms)
        } else {
            0
        };
    }

    fn track_touch(&mut self, sample: &SwitchInput, viewport: Viewport) {
        let inside = |position: [u32; 2]| {
            viewport.to_source([f64::from(position[0]), f64::from(position[1])])
        };
        self.touch = match self.touch {
            TouchState::Idle => match sample.touches.first() {
                None => TouchState::Idle,
                Some(touch) => match inside(touch.position) {
                    Some(point) => {
                        self.set_cursor(point);
                        self.cursor_visible = false;
                        TouchState::Primary(touch.id)
                    }
                    None => TouchState::Blocked,
                },
            },
            TouchState::Primary(id) => match sample.touches.iter().find(|t| t.id == id) {
                Some(touch) => {
                    // Dragging into the letterbox keeps the last game-area position.
                    if let Some(point) = inside(touch.position) {
                        self.set_cursor(point);
                    }
                    TouchState::Primary(id)
                }
                None if sample.touches.is_empty() => TouchState::Idle,
                None => TouchState::Blocked,
            },
            TouchState::Blocked if sample.touches.is_empty() => TouchState::Idle,
            TouchState::Blocked => TouchState::Blocked,
        };
    }

    fn move_cursor(&mut self, stick: [i32; 2], dt_ms: u32) {
        let axis = |value: i32| {
            let value = f64::from(value).clamp(-STICK_MAX, STICK_MAX);
            let magnitude =
                ((value.abs() - STICK_DEAD_ZONE) / (STICK_MAX - STICK_DEAD_ZONE)).clamp(0.0, 1.0);
            // Quadratic response: fine aiming near the centre, fast at the edge.
            magnitude * magnitude * value.signum()
        };
        let [dx, dy] = [axis(stick[0]), -axis(stick[1])];
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let speed = f64::from(self.size[0]) / CURSOR_CROSSING_MS * f64::from(dt_ms);
        self.cursor = [
            (self.cursor[0] + dx * speed).clamp(0.0, f64::from(self.size[0] - 1)),
            (self.cursor[1] + dy * speed).clamp(0.0, f64::from(self.size[1] - 1)),
        ];
        self.cursor_visible = true;
    }

    /// L/R and the right stick send wheel notches: one at once, then repeating.
    fn scroll(&mut self, buttons: u32, stick: i32, dt_ms: u32) {
        let code = if buttons & button::L != 0 || f64::from(stick) > STICK_DEAD_ZONE * 2.0 {
            Some(WHEEL_UP)
        } else if buttons & button::R != 0 || f64::from(-stick) > STICK_DEAD_ZONE * 2.0 {
            Some(WHEEL_DOWN)
        } else {
            None
        };
        self.wheel = match (code, self.wheel) {
            (None, _) => None,
            (Some(code), Some((previous, held))) if previous == code => {
                let held = held.saturating_add(dt_ms);
                if held >= self.wheel_next {
                    self.latched[code] = true;
                    self.wheel_next = held + WHEEL_REPEAT_MS;
                }
                Some((code, held))
            }
            (Some(code), _) => {
                self.latched[code] = true;
                self.wheel_next = WHEEL_DELAY_MS;
                Some((code, 0))
            }
        };
    }

    /// Commits the input polled since the previous frame. Call exactly once per
    /// engine input boundary, never per VM budget slice, and poll first: a frame
    /// without a poll since the previous one reads as everything released.
    pub fn frame(&mut self, bindings: &KeyBindings, input: &mut ActionInput) {
        if !self.focused {
            return;
        }
        let codes = std::mem::replace(&mut self.latched, [false; 128]);
        self.history.update(bindings, &codes, input);
        let [x, y] = self.pointer();
        input.set_pointer(x, y);
    }

    /// Engine navigation moved the system cursor.
    pub fn warp(&mut self, point: [i32; 2]) {
        self.set_cursor(point);
    }

    fn set_cursor(&mut self, [x, y]: [i32; 2]) {
        self.cursor = [
            f64::from(x.clamp(0, self.size[0] as i32 - 1)),
            f64::from(y.clamp(0, self.size[1] as i32 - 1)),
        ];
    }

    pub fn pointer(&self) -> [i32; 2] {
        [self.cursor[0] as i32, self.cursor[1] as i32]
    }

    /// The drawn pointer hides after a touch until the stick moves it again.
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_ms >= QUIT_HOLD_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_native::Touch;

    fn bindings() -> KeyBindings {
        KeyBindings::parse(
            "[KEY_FUNCTION_01]\nKEY_DATA_01=1\nKEY_DATA_02=22\n[KEY_FUNCTION_09]\nKEY_DATA_01=24\n",
        )
        .unwrap()
    }
    fn view(size: [u32; 2]) -> Viewport {
        Viewport::new(size, [1280, 720], 1).unwrap()
    }
    fn sample(buttons: u32) -> SwitchInput {
        SwitchInput {
            focused: true,
            buttons,
            ..SwitchInput::default()
        }
    }
    fn touched(points: &[(u32, [u32; 2])]) -> SwitchInput {
        SwitchInput {
            touches: points
                .iter()
                .map(|&(id, position)| Touch { id, position })
                .collect(),
            ..sample(0)
        }
    }

    #[test]
    fn short_press_between_frames_is_one_down_frame_then_release() {
        let mut controls = Controls::new([1280, 720]);
        let mut input = ActionInput::default();
        let b = bindings();
        controls.poll(&sample(button::A), view([1280, 720]), 8);
        controls.poll(&sample(0), view([1280, 720]), 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [false, true, true]);
        assert!(input.pressed(0));
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [true, false, false]);
        // ZR is Ctrl, bound to action 8 here.
        for _ in 0..2 {
            controls.poll(&sample(button::ZR), view([1280, 720]), 8);
            controls.frame(&b, &mut input);
        }
        assert!(input.held(8) && !input.pressed(8));
    }

    #[test]
    fn touch_follows_the_primary_contact_and_ignores_letterbox_starts() {
        // An 800x600 canvas contained in 1280x720 is 959x720 after a 160 px side bar.
        let viewport = view([800, 600]);
        let mut controls = Controls::new([800, 600]);
        let mut input = ActionInput::default();
        let b = bindings();
        controls.poll(&touched(&[(7, [100, 300])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert!(!input.mouse(0)[1], "letterbox start is ignored");
        controls.poll(&touched(&[(7, [640, 360])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert!(!input.mouse(0)[1], "the blocked gesture stays blocked");

        controls.poll(&touched(&[]), viewport, 8);
        controls.poll(&touched(&[(8, [640, 360])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [false, true, true]);
        assert_eq!(input.pointer(), [400, 300]);
        assert!(!controls.cursor_visible());

        // A second finger does not take over; dragging into the bar keeps the last point.
        controls.poll(&touched(&[(8, [1118, 0]), (9, [400, 100])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.pointer(), [799, 0]);
        controls.poll(&touched(&[(8, [1270, 360]), (9, [400, 100])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.pointer(), [799, 0]);
        // Lifting the primary releases; the remaining finger cannot start a gesture.
        controls.poll(&touched(&[(9, [400, 100])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [true, false, false]);
        assert_eq!(input.pointer(), [799, 0]);
        controls.poll(&touched(&[(9, [640, 360])]), viewport, 8);
        controls.frame(&b, &mut input);
        assert!(!input.mouse(0)[1]);
    }

    #[test]
    fn stick_moves_the_pointer_by_time_with_a_dead_zone_and_clamping() {
        let viewport = view([1280, 720]);
        let mut controls = Controls::new([1280, 720]);
        let still = SwitchInput {
            sticks: [[5000, -5000], [0, 0]],
            ..sample(0)
        };
        controls.poll(&still, viewport, 16);
        assert_eq!(controls.pointer(), [640, 360]);
        let right = SwitchInput {
            sticks: [[32767, 0], [0, 0]],
            ..sample(0)
        };
        controls.poll(&right, viewport, 100);
        assert_eq!(controls.pointer(), [746, 360]);
        // A long stall moves no further than one capped step.
        controls.poll(&right, viewport, 5000);
        assert_eq!(controls.pointer(), [853, 360]);
        let down = SwitchInput {
            sticks: [[0, -32767], [0, 0]],
            ..sample(0)
        };
        for _ in 0..20 {
            controls.poll(&down, viewport, 100);
        }
        assert_eq!(controls.pointer(), [853, 719]);
        assert!(controls.cursor_visible());
    }

    #[test]
    fn wheel_notches_repeat_after_a_delay_and_are_single_frame_pulses() {
        let viewport = view([1280, 720]);
        let mut controls = Controls::new([1280, 720]);
        let mut input = ActionInput::default();
        let b = bindings();
        let mut notches = 0;
        for _ in 0..70 {
            controls.poll(&sample(button::R), viewport, 10);
            controls.frame(&b, &mut input);
            notches += usize::from(input.mouse(3)[2]);
            assert!(!(input.mouse(3)[2] && input.mouse(3)[0]));
        }
        // Immediately, at 400 ms, 500 ms and 600 ms of 700 ms held.
        assert_eq!(notches, 4);
        controls.poll(&sample(0), viewport, 10);
        controls.frame(&b, &mut input);
        let up = SwitchInput {
            sticks: [[0, 0], [0, 32767]],
            ..sample(0)
        };
        controls.poll(&up, viewport, 10);
        controls.frame(&b, &mut input);
        assert!(input.mouse(2)[2]);
    }

    #[test]
    fn plus_must_be_held_to_quit_and_focus_loss_cancels_without_clicks() {
        let viewport = view([1280, 720]);
        let mut controls = Controls::new([1280, 720]);
        let mut input = ActionInput::default();
        let b = bindings();
        for _ in 0..9 {
            controls.poll(&sample(button::PLUS), viewport, 100);
        }
        assert!(!controls.quit_requested());
        controls.poll(&sample(0), viewport, 100);
        controls.poll(&sample(button::PLUS), viewport, 100);
        assert!(!controls.quit_requested());
        for _ in 0..10 {
            controls.poll(&sample(button::PLUS), viewport, 100);
        }
        assert!(controls.quit_requested());

        controls.poll(&sample(button::A), viewport, 8);
        controls.frame(&b, &mut input);
        assert!(input.mouse(0)[1]);
        controls.focus(false, &mut input);
        controls.focus(true, &mut input);
        controls.poll(&sample(0), viewport, 8);
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [false, false, false]);
        assert!(!controls.quit_requested());
    }

    #[test]
    fn buttons_held_from_a_system_applet_wait_for_release() {
        let viewport = view([1280, 720]);
        let mut controls = Controls::new([1280, 720]);
        let mut input = ActionInput::default();
        let b = bindings();
        controls.suppress_held(&mut input);
        for _ in 0..3 {
            controls.poll(&sample(button::A | button::PLUS), viewport, 500);
            controls.frame(&b, &mut input);
            assert_eq!(input.mouse(0), [false, false, false]);
        }
        assert!(!controls.quit_requested());
        // Releasing A unblocks it alone; + stays ignored while still held.
        controls.poll(&sample(button::PLUS), viewport, 500);
        controls.frame(&b, &mut input);
        controls.poll(&sample(button::A | button::PLUS), viewport, 500);
        controls.frame(&b, &mut input);
        assert_eq!(input.mouse(0), [false, true, true]);
        assert!(!controls.quit_requested());
    }
}

//! CVNS color dialog drawn in the logical game canvas.
use super::dialog;
use super::dialog::{ACCEPT_BUTTON, CANCEL_BUTTON, INK, MUTED, Panel, Pointer, axes, confirmed};
use crate::input::ActionInput;

const TRACK_LEFT: usize = 48;
const TRACK_WIDTH: usize = 256;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Red,
    Green,
    Blue,
    First,
    Second,
    Cancel,
    Accept,
}

impl Control {
    const ALL: [Self; 7] = [
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::First,
        Self::Second,
        Self::Cancel,
        Self::Accept,
    ];

    fn rect(self) -> [usize; 4] {
        match self {
            Self::Red => [TRACK_LEFT, 65, TRACK_WIDTH, 23],
            Self::Green => [TRACK_LEFT, 103, TRACK_WIDTH, 23],
            Self::Blue => [TRACK_LEFT, 141, TRACK_WIDTH, 23],
            Self::First => [18, 184, 39, 32],
            Self::Second => [65, 184, 39, 32],
            Self::Cancel => CANCEL_BUTTON,
            Self::Accept => ACCEPT_BUTTON,
        }
    }

    fn at([x, y]: [usize; 2]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|control| dialog::contains(control.rect(), [x, y]))
    }

    fn channel(self) -> Option<usize> {
        (self as usize <= Self::Blue as usize).then_some(self as usize)
    }

    fn next(self, direction: isize) -> Self {
        Self::ALL[(self as isize + direction).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

pub(super) struct ColorPicker {
    swatches: [[u8; 3]; 2],
    color: [u8; 3],
    focus: Control,
    pointer: Pointer<Control>,
}

impl ColorPicker {
    pub(super) fn new(swatches: [u32; 2]) -> Self {
        Self {
            swatches: swatches.map(crate::scene::cvns::unpack_rgb),
            color: [0; 3],
            focus: Control::Red,
            pointer: Pointer::default(),
        }
    }

    pub(super) fn update(&mut self, input: &mut ActionInput, rect: [i64; 4]) -> dialog::Update {
        let before = (self.color, self.focus);
        let (local, inside) = dialog::hit_point(rect, input.pointer());
        let over = inside.then(|| Control::at(local)).flatten();
        let [released, held, pressed] = input.mouse(0);
        if pressed && let Some(control) = over {
            self.focus = control;
        }
        let dragged = self.pointer.pressed;
        let clicked = self.pointer.update(over, input.mouse(0));
        let dragged = if pressed { over } else { dragged };
        if (held || released)
            && let Some(channel) = dragged.and_then(Control::channel)
            && input.pointer()[0] >= 0
        {
            self.color[channel] = local[0].saturating_sub(TRACK_LEFT).min(255) as u8;
        }
        if let Some(control) = clicked.filter(|control| control.channel().is_none()) {
            return self.activate(control);
        }
        if self.pointer.cancelled(input) {
            return dialog::Update::Close(dialog::Answer::Color(None));
        }
        let [horizontal, vertical] = axes(input);
        if vertical != 0 {
            self.focus = self.focus.next(isize::from(vertical));
        }
        if let Some(channel) = self.focus.channel() {
            let delta = horizontal + input.repeat_horizontal();
            self.color[channel] = self.color[channel].saturating_add_signed(delta);
        } else if horizontal != 0 {
            self.focus = self.focus.next(isize::from(horizontal));
        }
        if confirmed(input) {
            return self.activate(self.focus);
        }
        dialog::Update::from_change(before != (self.color, self.focus))
    }

    fn activate(&mut self, control: Control) -> dialog::Update {
        match control {
            Control::Cancel => dialog::Update::Close(dialog::Answer::Color(None)),
            Control::First => {
                self.color = self.swatches[0];
                dialog::Update::Redraw
            }
            Control::Second => {
                self.color = self.swatches[1];
                dialog::Update::Redraw
            }
            _ => {
                let [r, g, b] = self.color;
                dialog::Update::Close(dialog::Answer::Color(Some(u32::from_be_bytes([
                    0, r, g, b,
                ]))))
            }
        }
    }

    pub(super) fn describe(&self, page: &mut dialog::DialogPage) {
        let hex = |[r, g, b]: [u8; 3]| format!("#{r:02X}{g:02X}{b:02X}");
        page.text([18, 14, 364, 28], format!("Color {}", hex(self.color)));
        for control in Control::ALL {
            let label = match control {
                Control::Red => "Red".into(),
                Control::Green => "Green".into(),
                Control::Blue => "Blue".into(),
                Control::First => hex(self.swatches[0]),
                Control::Second => hex(self.swatches[1]),
                Control::Cancel => "Cancel".into(),
                Control::Accept => "OK".into(),
            };
            page.control(
                control.rect(),
                label,
                control.channel().map(|i| self.color[i]),
            );
        }
    }

    pub(super) fn render(&self) -> Panel {
        let mut panel = Panel::new();
        panel.text([18, 18], b"COLOR", INK);
        panel.fill([245, 15, 30, 24], self.color);
        panel.outline([245, 15, 30, 24], MUTED);
        let [r, g, b] = self.color;
        panel.text([290, 18], format!("#{r:02X}{g:02X}{b:02X}").as_bytes(), INK);

        for (channel, control) in Control::ALL[..3].iter().copied().enumerate() {
            let [x, y, w, h] = control.rect();
            panel.text([22, y + 4], &b"RGB"[channel..=channel], INK);
            for value in 0..w {
                let mut color = self.color;
                color[channel] = value as u8;
                panel.fill([x + value, y, 1, h], color);
            }
            let mark = [x + usize::from(self.color[channel]) - 2, y - 2, 5, h + 4];
            panel.outline(mark, INK);
            panel.text(
                [326, y + 4],
                format!("{:03}", self.color[channel]).as_bytes(),
                MUTED,
            );
        }
        for (control, color) in [
            (Control::First, self.swatches[0]),
            (Control::Second, self.swatches[1]),
        ] {
            panel.fill(control.rect(), color);
            panel.outline(control.rect(), MUTED);
        }
        panel.button(Control::Cancel.rect(), b"CANCEL", false);
        panel.button(Control::Accept.rect(), b"OK", true);
        panel.focus(self.focus.rect());
        panel
    }
}

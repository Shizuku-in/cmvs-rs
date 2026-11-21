//! CVNS exit confirmation and script-supplied alerts.
use super::dialog::{
    self, ACCEPT_BUTTON, Answer, CANCEL_BUTTON, INK, Panel, Pointer, Update, draw_line, wrap_text,
};
use crate::{
    TextEncoding,
    input::ActionInput,
    message::GlyphRasterizer,
    platform::{MessageDialogKind, MessageDialogRequest},
};

const LINES_VISIBLE: usize = 5;
const TEXT_LEFT: usize = 18;
const TEXT_WIDTH: usize = 334;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Cancel,
    Accept,
    Up,
    Down,
}

impl Button {
    fn rect(self) -> [usize; 4] {
        match self {
            Self::Cancel => CANCEL_BUTTON,
            Self::Accept => ACCEPT_BUTTON,
            Self::Up => [353, 59, 29, 32],
            Self::Down => [353, 133, 29, 32],
        }
    }
}

pub(super) struct MessageDialog {
    kind: MessageDialogKind,
    caption: Vec<u16>,
    lines: Vec<Vec<u16>>,
    top: usize,
    focus: Button,
    pointer: Pointer<Button>,
}

impl MessageDialog {
    pub(super) fn new(
        request: MessageDialogRequest,
        encoding: TextEncoding,
        is_exit: bool,
        has_font: bool,
    ) -> Self {
        let (caption, text) = if is_exit && !has_font {
            (b"EXIT GAME".as_slice(), b"QUIT THE GAME?".as_slice())
        } else {
            (request.caption.as_slice(), request.text.as_slice())
        };
        let mut caption = wrap_text(caption, encoding, if has_font { 360 } else { TEXT_WIDTH });
        if caption.len() > 1 {
            // Keep the title bar to one line.
            let end = caption[0].len().saturating_sub(3);
            caption[0].truncate(end);
            caption[0].extend([u16::from(b'.'); 3]);
        }
        Self {
            kind: request.kind,
            caption: caption.swap_remove(0),
            lines: wrap_text(text, encoding, TEXT_WIDTH),
            top: 0,
            focus: if request.kind == MessageDialogKind::Confirm {
                Button::Cancel
            } else {
                Button::Accept
            },
            pointer: Pointer::default(),
        }
    }

    fn has_cancel(&self) -> bool {
        self.kind == MessageDialogKind::Confirm
    }

    fn buttons(&self) -> [(Button, bool); 4] {
        [
            (Button::Cancel, self.has_cancel()),
            (Button::Accept, true),
            (Button::Up, self.top > 0),
            (Button::Down, self.top + LINES_VISIBLE < self.lines.len()),
        ]
    }

    pub(super) fn describe(&self, page: &mut dialog::DialogPage, decode: impl Fn(u16) -> char) {
        page.text(
            [18, 14, 364, 24],
            self.caption.iter().copied().map(&decode).collect(),
        );
        let body = self
            .lines
            .iter()
            .skip(self.top)
            .take(LINES_VISIBLE)
            .map(|line| line.iter().copied().map(&decode).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        page.text([TEXT_LEFT, 56, TEXT_WIDTH, 115], body);
        for (button, enabled) in self.buttons() {
            if enabled {
                let label = match button {
                    Button::Up => "Scroll up",
                    Button::Down => "Scroll down",
                    Button::Cancel => "Cancel",
                    Button::Accept if self.has_cancel() => "Yes",
                    Button::Accept => "OK",
                };
                page.control(button.rect(), label, None);
            }
        }
    }

    pub(super) fn update(&mut self, input: &mut ActionInput, rect: [i64; 4]) -> Update {
        let before = (self.top, self.focus);
        let (point, inside) = dialog::hit_point(rect, input.pointer());
        let bottom = self.lines.len().saturating_sub(LINES_VISIBLE);
        let over = self.buttons().into_iter().find_map(|(button, enabled)| {
            (enabled && inside && dialog::contains(button.rect(), point)).then_some(button)
        });
        let mouse = input.mouse(0);
        if mouse[2]
            && let Some(button @ (Button::Accept | Button::Cancel)) = over
        {
            self.focus = button;
        }
        match self.pointer.update(over, mouse) {
            Some(Button::Cancel) => return Update::Close(Answer::Message(false)),
            Some(Button::Accept) => return Update::Close(Answer::Message(true)),
            Some(Button::Up) => self.top -= 1,
            Some(Button::Down) => self.top += 1,
            None => {}
        }
        if self.pointer.cancelled(input) {
            // Acknowledge-only alerts have the same result for Escape and OK.
            return Update::Close(Answer::Message(!self.has_cancel()));
        }
        let [horizontal, vertical] = dialog::axes(input);
        let wheel = i8::from(input.cvns_wait_interrupt(2, true))
            - i8::from(input.cvns_wait_interrupt(1, true));
        self.top = self
            .top
            .saturating_add_signed(isize::from(vertical + wheel))
            .min(bottom);
        if self.has_cancel() {
            self.focus = match horizontal {
                -1 => Button::Cancel,
                1 => Button::Accept,
                _ => self.focus,
            };
        }
        if dialog::confirmed(input) {
            return Update::Close(Answer::Message(self.focus == Button::Accept));
        }
        Update::from_change(before != (self.top, self.focus))
    }

    pub(super) fn render(
        &self,
        mut font: Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    ) -> Panel {
        let mut panel = Panel::new();
        draw_line(&mut panel, &mut font, [TEXT_LEFT, 14], &self.caption, INK);
        let body_y = 56 + LINES_VISIBLE.saturating_sub(self.lines.len()) * 23 / 2;
        for (line, glyphs) in self
            .lines
            .iter()
            .skip(self.top)
            .take(LINES_VISIBLE)
            .enumerate()
        {
            draw_line(
                &mut panel,
                &mut font,
                [TEXT_LEFT, body_y + line * 23],
                glyphs,
                INK,
            );
        }
        if self.top > 0 {
            panel.button(Button::Up.rect(), b"^", false);
        }
        if self.top + LINES_VISIBLE < self.lines.len() {
            panel.button(Button::Down.rect(), b"V", false);
        }
        if self.has_cancel() {
            panel.button(Button::Cancel.rect(), b"CANCEL", false);
        }
        panel.button(
            Button::Accept.rect(),
            if self.has_cancel() { b"YES" } else { b"OK" },
            true,
        );
        panel.focus(self.focus.rect());
        panel
    }
}

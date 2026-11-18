//! CVNS exit confirmation and script-supplied alerts.
use super::dialog::{self, Answer, FOCUS, INK, Panel, Pointer, Update, WIDTH};
use crate::{
    TextEncoding,
    input::ActionInput,
    message::{GlyphRasterizer, RecordedGlyph},
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
            Self::Cancel => [188, 184, 92, 32],
            Self::Accept => [292, 184, 90, 32],
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
        let mut caption = wrap(caption, encoding, if has_font { 360 } else { TEXT_WIDTH });
        if caption.len() > 1 {
            // Keep the title bar to one line.
            let end = caption[0].len().saturating_sub(3);
            caption[0].truncate(end);
            caption[0].extend([u16::from(b'.'); 3]);
        }
        Self {
            kind: request.kind,
            caption: caption.swap_remove(0),
            lines: wrap(text, encoding, TEXT_WIDTH),
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

    pub(super) fn update(&mut self, input: &mut ActionInput, rect: [i64; 4]) -> Update {
        let before = (self.top, self.focus);
        let (point, inside) = dialog::hit_point(rect, input.pointer());
        let bottom = self.lines.len().saturating_sub(LINES_VISIBLE);
        let over = [
            (Button::Cancel, self.has_cancel()),
            (Button::Accept, true),
            (Button::Up, self.top > 0),
            (Button::Down, self.top < bottom),
        ]
        .into_iter()
        .find_map(|(button, enabled)| {
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
        panel.fill([0, 0, WIDTH, 3], FOCUS);
        panel.fill([18, 48, 364, 1], [63, 78, 100]);
        draw_line(&mut panel, &mut font, 14, &self.caption, INK);
        let body_y = 56 + LINES_VISIBLE.saturating_sub(self.lines.len()) * 23 / 2;
        for (line, glyphs) in self
            .lines
            .iter()
            .skip(self.top)
            .take(LINES_VISIBLE)
            .enumerate()
        {
            draw_line(&mut panel, &mut font, body_y + line * 23, glyphs, INK);
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

fn advance(code: u16) -> usize {
    if code <= 0x7f { 12 } else { 20 }
}

fn wrap(bytes: &[u8], encoding: TextEncoding, width: usize) -> Vec<Vec<u16>> {
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut x = 0;
    let mut bytes = bytes.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'\r' && bytes.peek() == Some(&b'\n') {
            continue;
        }
        if matches!(byte, b'\r' | b'\n') {
            lines.push(std::mem::take(&mut line));
            x = 0;
            continue;
        }
        let code = if let Some(len) = encoding.wide_len(byte) {
            let mut sequence = [byte; 4];
            let mut available = 1;
            for b in &mut sequence[1..len] {
                let Some(next) = bytes.next_if(|&next| !matches!(next, b'\r' | b'\n')) else {
                    break;
                };
                *b = next;
                available += 1;
            }
            if available == len {
                encoding
                    .wide_code(&sequence[..len])
                    .unwrap_or(u16::from(b'?'))
            } else {
                u16::from(b'?')
            }
        } else if !byte.is_ascii() {
            u16::from_be_bytes([0xff, byte])
        } else if byte.is_ascii_control() {
            u16::from(b' ')
        } else {
            u16::from(byte)
        };
        let advance = advance(code);
        if x + advance > width {
            lines.push(std::mem::take(&mut line));
            x = 0;
        }
        line.push(code);
        x += advance;
    }
    lines.push(line);
    lines
}

fn draw_line(
    panel: &mut Panel,
    font: &mut Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    y: usize,
    codes: &[u16],
    ink: [u8; 3],
) {
    const SIZE: u32 = 20;
    let [r, g, b] = ink;
    let mut glyph = RecordedGlyph {
        snapshot_indices: None,
        face: std::sync::Arc::from(crate::settings::DEFAULT_FONT_FACE),
        bold: false,
        revision_alpha: false,
        code: 0,
        position: [0, 0],
        size: SIZE,
        colors: [u32::from_be_bytes([0, r, g, b]); 2],
        gradient: 0,
        outline: [0; 2],
        delay_ms: 0,
        elapsed_ms: 0,
        duration_ms: 0,
        active: false,
        shown: true,
        opacity: 255,
        reveal_mode: 0,
        reveal_width: SIZE + 4,
    };
    let mut x = TEXT_LEFT;
    for &code in codes {
        glyph.code = code;
        if let Some(image) = font
            .as_deref_mut()
            .and_then(|font| font.rasterize_glyph(&glyph).ok())
        {
            panel.blit_glyph([x - 2, y], &image);
        } else {
            panel.text([x, y + 4], &[u8::try_from(code).unwrap_or(b'?')], ink);
        }
        x += advance(code);
    }
}

//! Shared canvas and VM suspension for script dialogs.
use super::{
    Runtime,
    color_picker::ColorPicker,
    message_dialog::MessageDialog,
    text_dialog::{CommentEdit, TextDialog},
};
use crate::{
    FormatError, TextEncoding,
    dispatch::{RuntimeCall, store_color_result},
    input::ActionInput,
    message::{GlyphRasterizer, RecordedGlyph},
    platform::PlatformBackend,
    raster::RgbaImage,
    scene::ObjectImage,
};

pub(super) const WIDTH: usize = 400;
pub(super) const HEIGHT: usize = 234;
pub(super) const INK: [u8; 3] = [238, 242, 249];
pub(super) const MUTED: [u8; 3] = [163, 178, 199];
pub(super) const FOCUS: [u8; 3] = [127, 187, 255];

pub(super) const CANCEL_BUTTON: [usize; 4] = [188, 184, 92, 32];
pub(super) const ACCEPT_BUTTON: [usize; 4] = [292, 184, 90, 32];

pub(super) enum Content {
    Color(ColorPicker),
    Message(MessageDialog),
    Text(Box<TextDialog>),
}

pub(super) enum Update {
    Idle,
    Redraw,
    Close(Answer),
}

impl Update {
    pub(super) fn from_change(changed: bool) -> Self {
        if changed { Self::Redraw } else { Self::Idle }
    }
}

pub(super) enum Answer {
    Color(Option<u32>),
    Message(bool),
    Text(Option<Vec<u8>>),
}

pub(super) struct Dialog {
    request: RuntimeCall,
    content: Content,
    rect: [i64; 4],
}

/// Visible text and pointer targets of a modal dialog in logical canvas coordinates.
#[derive(Default)]
pub struct DialogPage {
    pub text: Vec<([i32; 4], String)>,
    pub buttons: Vec<DialogButton>,
    rect: [i64; 4],
}

pub struct DialogButton {
    pub rect: [i32; 4],
    pub label: String,
    /// Position on a color channel's horizontal track, from left to right.
    pub slider: Option<u8>,
}

impl DialogPage {
    fn canvas_rect(&self, [x, y, w, h]: [usize; 4]) -> [i32; 4] {
        let [left, top, width, height] = self.rect;
        // Invert hit_point's floor division at both edges.
        let px = |v| (left + (v as i64 * width + WIDTH as i64 - 1) / WIDTH as i64) as i32;
        let py = |v| (top + (v as i64 * height + HEIGHT as i64 - 1) / HEIGHT as i64) as i32;
        [px(x), py(y), px(x + w) - px(x), py(y + h) - py(y)]
    }

    pub(super) fn text(&mut self, rect: [usize; 4], value: String) {
        if !value.trim().is_empty() {
            self.text.push((self.canvas_rect(rect), value));
        }
    }

    pub(super) fn control(
        &mut self,
        rect: [usize; 4],
        label: impl Into<String>,
        slider: Option<u8>,
    ) {
        let rect = self.canvas_rect(rect);
        if rect[2] > 0 && rect[3] > 0 {
            self.buttons.push(DialogButton {
                rect,
                label: label.into(),
                slider,
            });
        }
    }
}

impl Dialog {
    fn image(
        &self,
        font: Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    ) -> Result<ObjectImage, FormatError> {
        let panel = match &self.content {
            Content::Color(picker) => picker.render(),
            Content::Message(message) => message.render(font),
            Content::Text(text) => text.render(font),
        };
        Ok(ObjectImage {
            pixels: RgbaImage::new(WIDTH as u32, HEIGHT as u32, panel.0)?,
            filter: None,
            name: Vec::new(),
            alpha: false,
            surface: false,
        })
    }
}

impl Runtime {
    /// Enable the color picker for a host that presents and supplies input for each frame.
    pub fn enable_in_game_color_picker(&mut self) {
        self.in_game_color_picker = true;
    }

    /// Enable canvas dialogs, optionally using the player's font for script text.
    pub fn enable_in_game_dialogs(&mut self, font: Option<Box<dyn GlyphRasterizer + Send>>) {
        self.enable_in_game_color_picker();
        self.in_game_message_dialogs = true;
        self.in_game_text_dialogs = true;
        self.dialog_font = font;
    }

    pub(super) fn open_dialog(
        &mut self,
        request: RuntimeCall,
        content: Content,
        backend: &impl PlatformBackend,
    ) -> Result<(), FormatError> {
        let [sw, sh] = self.screen_size().map(i64::from);
        let mut w = sw.min(WIDTH as i64);
        let mut h = sh.min(HEIGHT as i64);
        if w * (HEIGHT as i64) < h * (WIDTH as i64) {
            h = (w * HEIGHT as i64 / WIDTH as i64).max(1);
        } else {
            w = (h * WIDTH as i64 / HEIGHT as i64).max(1);
        }
        let rect = [(sw - w) / 2, (sh - h) / 2, w, h];
        let dialog = Dialog {
            request,
            content,
            rect,
        };
        let image = dialog.image(self.dialog_font.as_deref_mut())?;
        if let Some(movie) = self.movies.player_image() {
            let pixels = crate::raster::movie_canvas(
                movie,
                self.screen_size(),
                backend.render_mode().unwrap_or(0) == 1,
            )?;
            self.scene.modal_movie = Some(ObjectImage {
                pixels,
                ..image.clone()
            });
        }
        self.scene.modal_panel = Some((rect, image));
        self.dialog = Some(dialog);
        clear_edges(&mut self.input);
        self.frame_delta = 0;
        Ok(())
    }

    /// Read the current modal panel using the host's game-font decoder.
    pub fn dialog_page(&self, decode: impl Fn(u16) -> char) -> Option<DialogPage> {
        let dialog = self.dialog.as_ref()?;
        let mut page = DialogPage {
            rect: dialog.rect,
            ..Default::default()
        };
        match &dialog.content {
            Content::Color(picker) => picker.describe(&mut page),
            Content::Message(message) => message.describe(&mut page, decode),
            Content::Text(text) => text.describe(&mut page, decode),
        }
        Some(page)
    }

    /// Logical canvas rectangle of the focused inline comment field.
    pub fn inline_comment_rect(&self) -> Option<[i64; 4]> {
        let dialog = self.dialog.as_ref()?;
        match &dialog.content {
            Content::Text(text) if text.inline_focused() => Some(text.field_rect(dialog.rect)),
            _ => None,
        }
    }

    /// Logical canvas rectangle of the field awaiting a platform keyboard.
    pub fn keyboard_comment_rect(&self) -> Option<[i64; 4]> {
        let dialog = self.dialog.as_ref()?;
        match &dialog.content {
            Content::Text(text) if text.waiting_for_keyboard() => {
                Some(text.field_rect(dialog.rect))
            }
            _ => None,
        }
    }

    /// Deliver a keyboard or IME edit to the open comment field.
    pub fn edit_comment(&mut self, edit: CommentEdit) -> bool {
        if let Some(Dialog {
            content: Content::Text(text),
            ..
        }) = &mut self.dialog
        {
            return text.edit(edit);
        }
        false
    }

    pub(super) fn advance_dialog(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), super::RuntimeError> {
        let dialog = self.dialog.as_mut().expect("open dialog");
        let update = match &mut dialog.content {
            Content::Color(picker) => picker.update(&mut self.input, dialog.rect),
            Content::Message(message) => message.update(&mut self.input, dialog.rect),
            Content::Text(text) => text.update(&mut self.input, dialog.rect, backend),
        };
        match update {
            Update::Close(result) => {
                match result {
                    Answer::Color(color) => {
                        dialog.request.complete(&mut self.machine, 0x4008)?;
                        store_color_result(self.machine.state_mut(), color);
                    }
                    Answer::Text(answer) => {
                        let Content::Text(text) = &dialog.content else {
                            unreachable!()
                        };
                        let binding = &text.binding;
                        let result = binding.result(&self.machine, answer, false, |why| {
                            dialog.request.error(why)
                        })?;
                        dialog.request.complete(&mut self.machine, 0x4018)?;
                        binding.store(&mut self.machine, result);
                    }
                    Answer::Message(accepted) => {
                        let flags = if dialog.request.command() == 0x5b0 {
                            0x4000
                        } else {
                            0x400c
                        };
                        dialog.request.complete(&mut self.machine, flags)?;
                        self.machine
                            .state_mut()
                            .set_system_integer(0, u32::from(accepted))
                            .expect("slot zero exists");
                    }
                }
                self.clear_dialog();
                clear_edges(&mut self.input);
                self.resume_run = true;
            }
            Update::Redraw => {
                let image = dialog.image(self.dialog_font.as_deref_mut())?;
                self.scene.modal_panel = Some((dialog.rect, image));
            }
            Update::Idle => {}
        }
        Ok(())
    }

    pub(super) fn clear_dialog(&mut self) {
        self.dialog = None;
        self.scene.modal_panel = None;
        self.scene.modal_movie = None;
    }

    pub(super) fn discard_dialog(&mut self, backend: &mut impl PlatformBackend) {
        if self.keyboard_comment_rect().is_some() {
            backend.cancel_text_dialog();
        }
        if self.dialog.is_some() {
            self.clear_dialog();
            clear_edges(&mut self.input);
        }
    }
}

fn clear_edges(input: &mut ActionInput) {
    input.clear_all_edges();
    input.cvns_wait_interrupt(0xff, true);
}

pub(super) fn axes(input: &mut ActionInput) -> [i8; 2] {
    [
        i8::from(input.cvns_wait_interrupt(1 << 11, true))
            - i8::from(input.cvns_wait_interrupt(1 << 10, true)),
        i8::from(input.cvns_wait_interrupt(1 << 9, true))
            - i8::from(input.cvns_wait_interrupt(1 << 8, true)),
    ]
}

pub(super) fn confirmed(input: &mut ActionInput) -> bool {
    input.cvns_wait_interrupt((1 << 5) | (1 << 6), true)
}

pub(super) fn hit_point([x, y, w, h]: [i64; 4], pointer: [i32; 2]) -> ([usize; 2], bool) {
    let [px, py] = pointer.map(i64::from);
    (
        [
            ((px - x) * WIDTH as i64 / w).clamp(0, WIDTH as i64 - 1) as usize,
            ((py - y) * HEIGHT as i64 / h).clamp(0, HEIGHT as i64 - 1) as usize,
        ],
        (x..x + w).contains(&px) && (y..y + h).contains(&py),
    )
}

pub(super) fn contains([x, y, w, h]: [usize; 4], [px, py]: [usize; 2]) -> bool {
    (x..x + w).contains(&px) && (y..y + h).contains(&py)
}

pub(super) struct Pointer<T> {
    pub(super) pressed: Option<T>,
    right_pressed: bool,
}

impl<T> Default for Pointer<T> {
    fn default() -> Self {
        Self {
            pressed: None,
            right_pressed: false,
        }
    }
}

impl<T: Copy + PartialEq> Pointer<T> {
    pub(super) fn cancelled(&mut self, input: &mut ActionInput) -> bool {
        let [released, held, pressed] = input.mouse(1);
        let was_pressed = self.right_pressed;
        self.right_pressed = held && (pressed || was_pressed);
        (released && was_pressed) || input.cvns_wait_interrupt(1 << 4, true)
    }

    pub(super) fn update(
        &mut self,
        over: Option<T>,
        [released, held, pressed]: [bool; 3],
    ) -> Option<T> {
        if pressed {
            self.pressed = over;
        }
        if released {
            return self.pressed.take().filter(|&control| Some(control) == over);
        }
        if !held && !pressed {
            // Focus loss clears buttons without producing a release.
            self.pressed = None;
        }
        None
    }
}
pub(super) struct Panel(Vec<u8>);
impl Panel {
    pub(super) fn new() -> Self {
        let mut panel = Self([23, 32, 48, 255].repeat(WIDTH * HEIGHT));
        panel.fill([0, 0, WIDTH, 3], FOCUS);
        panel.fill([18, 48, 364, 1], [63, 78, 100]);
        panel
    }

    pub(super) fn fill(&mut self, [x, y, w, h]: [usize; 4], [r, g, b]: [u8; 3]) {
        for row in y..y + h {
            let start = (row * WIDTH + x) * 4;
            for pixel in self.0[start..start + w * 4].as_chunks_mut::<4>().0 {
                *pixel = [r, g, b, 255];
            }
        }
    }

    pub(super) fn outline(&mut self, [x, y, w, h]: [usize; 4], color: [u8; 3]) {
        self.fill([x, y, w, 1], color);
        self.fill([x, y + h - 1, w, 1], color);
        self.fill([x, y, 1, h], color);
        self.fill([x + w - 1, y, 1, h], color);
    }

    pub(super) fn button(&mut self, rect: [usize; 4], label: &[u8], primary: bool) {
        let [x, y, w, h] = rect;
        let (background, ink) = if primary {
            ([150, 199, 255], [17, 29, 55])
        } else {
            ([41, 53, 72], INK)
        };
        self.fill(rect, background);
        self.text(
            [x + (w - label.len() * 12 + 2) / 2, y + (h - 14) / 2],
            label,
            ink,
        );
    }

    pub(super) fn focus(&mut self, [x, y, w, h]: [usize; 4]) {
        self.outline([x - 3, y - 3, w + 6, h + 6], FOCUS);
    }

    pub(super) fn blit_glyph(&mut self, [x, y]: [usize; 2], glyph: &RgbaImage) {
        for (i, src) in glyph.pixels().as_chunks::<4>().0.iter().enumerate() {
            let stride = glyph.width() as usize;
            let [px, py] = [x + i % stride, y + i / stride];
            if px >= WIDTH || py >= HEIGHT || src[3] == 0 {
                continue;
            }
            let index = (py * WIDTH + px) * 4;
            let alpha = u32::from(src[3]);
            for (dst, &src) in self.0[index..index + 3].iter_mut().zip(src) {
                *dst =
                    ((u32::from(src) * alpha + u32::from(*dst) * (255 - alpha) + 127) / 255) as u8;
            }
        }
    }

    pub(super) fn text(&mut self, [x, y]: [usize; 2], bytes: &[u8], color: [u8; 3]) {
        for (index, &code) in bytes.iter().enumerate() {
            for (row, bits) in label_glyph(code).into_iter().enumerate() {
                for column in 0..5 {
                    if bits & (16 >> column) != 0 {
                        self.fill([x + index * 12 + column * 2, y + row * 2, 2, 2], color);
                    }
                }
            }
        }
    }
}

// The few monochrome glyphs needed for the picker are independent of game fonts and encodings.
fn label_glyph(code: u8) -> [u8; 7] {
    match code.to_ascii_uppercase() {
        b'#' => [10, 10, 31, 10, 31, 10, 10],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'G' => [14, 17, 16, 23, 17, 17, 15],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'N' => [17, 25, 21, 19, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'I' => [14, 4, 4, 4, 4, 4, 14],
        b'J' => [7, 2, 2, 2, 18, 18, 12],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'P' => [30, 17, 17, 30, 16, 16, 16],
        b'Q' => [14, 17, 17, 17, 21, 18, 13],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'U' => [17, 17, 17, 17, 17, 17, 14],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'W' => [17, 17, 17, 21, 21, 21, 10],
        b'X' => [17, 17, 10, 4, 10, 17, 17],
        b'Y' => [17, 17, 10, 4, 4, 4, 4],
        b'Z' => [31, 1, 2, 4, 8, 16, 31],
        b'^' => [4, 10, 17, 0, 0, 0, 0],
        b'.' => [0, 0, 0, 0, 0, 12, 12],
        b',' => [0, 0, 0, 0, 12, 12, 8],
        b'!' => [4, 4, 4, 4, 4, 0, 4],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b':' => [0, 12, 12, 0, 12, 12, 0],
        b'/' => [1, 1, 2, 4, 8, 16, 16],
        b' ' => [0; 7],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

pub(super) fn glyph_advance(code: u16) -> usize {
    if code <= 0x7f { 12 } else { 20 }
}

pub(super) fn wrap_text(bytes: &[u8], encoding: TextEncoding, width: usize) -> Vec<Vec<u16>> {
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
        let advance = glyph_advance(code);
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

pub(super) fn draw_line(
    panel: &mut Panel,
    font: &mut Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    [mut x, y]: [usize; 2],
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
        x += glyph_advance(code);
    }
}

//! Save-comment editor in the game canvas.
use super::dialog::{
    self, ACCEPT_BUTTON, Answer, CANCEL_BUTTON, FOCUS, HEIGHT, INK, MUTED, Panel, Pointer, Update,
    WIDTH, draw_line, glyph_advance, wrap_text,
};
use crate::{
    TextEncoding,
    dispatch::VmTextDialog,
    input::ActionInput,
    message::GlyphRasterizer,
    platform::{PlatformBackend, TextDialogRequest, TextDialogResult},
};

const FIELD: [usize; 4] = [18, 119, 364, 38];
const FIELD_ORIGIN: [usize; 2] = [28, 127];
const FIELD_TEXT_WIDTH: usize = FIELD[2] - 20;
const PROMPT_ROWS: usize = 2;

/// An edit delivered by a host with inline keyboard and IME support.
/// Text byte strings use the game's encoding.
pub enum CommentEdit {
    Insert(Vec<u8>),
    Preedit(Vec<u8>),
    Invalid,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    SelectAll,
    Tab { backwards: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Field,
    Cancel,
    Accept,
    PromptUp,
    PromptDown,
}

impl Control {
    const ALL: [Self; 5] = [
        Self::Field,
        Self::Cancel,
        Self::Accept,
        Self::PromptUp,
        Self::PromptDown,
    ];

    fn rect(self) -> [usize; 4] {
        match self {
            Self::Field => FIELD,
            Self::Cancel => CANCEL_BUTTON,
            Self::Accept => ACCEPT_BUTTON,
            Self::PromptUp => [353, 56, 29, 22],
            Self::PromptDown => [353, 83, 29, 22],
        }
    }
}

pub(super) struct TextDialog {
    pub(super) binding: VmTextDialog,
    encoding: TextEncoding,
    inline: bool,
    focus: Control,
    pointer: Pointer<Control>,
    draft: Vec<u8>,
    caret: usize,
    select_all: bool,
    preedit: Vec<u8>,
    keyboard: Option<TextDialogRequest>,
    changed: bool,
    error: Option<&'static [u8]>,
    prompt: Vec<Vec<u16>>,
    prompt_top: usize,
}

struct FieldLayout {
    codes: Vec<u16>,
    first: usize,
    before_width: usize,
    preedit_width: usize,
}

impl TextDialog {
    pub(super) fn new(mut binding: VmTextDialog, encoding: TextEncoding, inline: bool) -> Self {
        remove_full_width_notice(&mut binding.request.prompt, encoding);
        let draft = binding.request.initial.clone();
        let mut prompt = wrap_text(&binding.request.prompt, encoding, 362);
        if prompt.len() > PROMPT_ROWS {
            prompt = wrap_text(&binding.request.prompt, encoding, 328);
        }
        Self {
            binding,
            encoding,
            inline,
            focus: Control::Field,
            pointer: Pointer::default(),
            caret: draft.len(),
            select_all: inline && !draft.is_empty(),
            draft,
            preedit: Vec::new(),
            keyboard: None,
            changed: false,
            error: None,
            prompt,
            prompt_top: 0,
        }
    }

    pub(super) fn inline_focused(&self) -> bool {
        self.inline && self.focus == Control::Field
    }

    pub(super) fn waiting_for_keyboard(&self) -> bool {
        self.keyboard.is_some()
    }

    pub(super) fn field_rect(&self, [x, y, w, h]: [i64; 4]) -> [i64; 4] {
        [
            x + FIELD[0] as i64 * w / WIDTH as i64,
            y + FIELD[1] as i64 * h / HEIGHT as i64,
            FIELD[2] as i64 * w / WIDTH as i64,
            FIELD[3] as i64 * h / HEIGHT as i64,
        ]
    }

    fn set_focus(&mut self, focus: Control) {
        if self.preedit.is_empty() && self.focus != focus {
            self.focus = focus;
            self.select_all = false;
            self.changed = true;
        }
    }

    fn controls(&self) -> &'static [Control] {
        if self.prompt.len() > PROMPT_ROWS {
            &Control::ALL
        } else {
            &Control::ALL[..3]
        }
    }

    fn move_focus(&mut self, direction: i8) {
        let controls = self.controls();
        self.set_focus(
            controls[(self.focus as i8 + direction).rem_euclid(controls.len() as i8) as usize],
        );
    }

    fn scroll_prompt(&mut self, rows: isize) {
        let top = self
            .prompt_top
            .saturating_add_signed(rows)
            .min(self.prompt.len().saturating_sub(PROMPT_ROWS));
        self.changed |= top != self.prompt_top;
        self.prompt_top = top;
    }

    pub(super) fn edit(&mut self, edit: CommentEdit) -> bool {
        if let CommentEdit::Tab { backwards } = edit {
            if self.inline {
                self.move_focus(if backwards { -1 } else { 1 });
            }
            return self.inline;
        }
        if !self.inline_focused() {
            return false;
        }
        self.changed = true;
        if let CommentEdit::Preedit(bytes) = edit {
            self.preedit = bytes.into_iter().take(255).collect();
            return true;
        }
        self.preedit.clear();
        let previous = character_boundaries(&self.draft, self.encoding)
            .take_while(|&i| i < self.caret)
            .last()
            .unwrap_or(0);
        let next = character_boundaries(&self.draft, self.encoding)
            .find(|&i| i > self.caret)
            .unwrap_or(self.draft.len());
        let selection = if self.select_all {
            0..self.draft.len()
        } else {
            self.caret..self.caret
        };
        match edit {
            CommentEdit::Invalid => self.error = Some(b"UNSUPPORTED CHARACTERS"),
            CommentEdit::Insert(bytes) => self.replace(selection, &bytes),
            CommentEdit::Backspace => {
                let range = if self.select_all {
                    selection
                } else {
                    previous..self.caret
                };
                self.replace(range, &[]);
            }
            CommentEdit::Delete => {
                let range = if self.select_all {
                    selection
                } else {
                    self.caret..next
                };
                self.replace(range, &[]);
            }
            CommentEdit::Left | CommentEdit::Home => {
                self.caret = if self.select_all || matches!(edit, CommentEdit::Home) {
                    0
                } else {
                    previous
                };
                self.select_all = false;
            }
            CommentEdit::Right | CommentEdit::End => {
                self.caret = if self.select_all || matches!(edit, CommentEdit::End) {
                    self.draft.len()
                } else {
                    next
                };
                self.select_all = false;
            }
            CommentEdit::SelectAll => self.select_all = !self.draft.is_empty(),
            CommentEdit::Preedit(_) | CommentEdit::Tab { .. } => unreachable!(),
        }
        true
    }

    fn replace(&mut self, range: std::ops::Range<usize>, bytes: &[u8]) {
        let inserted = match comment_glyph_count(bytes, self.encoding) {
            Ok(count) => count,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let count = |bytes| character_boundaries(bytes, self.encoding).count() - 1;
        let previous_chars = count(&self.draft);
        let chars = previous_chars - count(&self.draft[range.clone()]) + inserted;
        let len = self.draft.len() - range.len() + bytes.len();
        let request = &self.binding.request;
        let too_many_bytes = len > request.byte_limit() as usize && len >= self.draft.len();
        let too_many_chars =
            chars > request.character_limit(self.encoding) as usize && chars >= previous_chars;
        // Allow shortening prefilled comments written with a larger limit.
        if too_many_bytes || too_many_chars {
            self.error = Some(b"COMMENT IS TOO LONG");
            return;
        }
        self.caret = range.start + bytes.len();
        self.draft.splice(range, bytes.iter().copied());
        self.select_all = false;
        self.error = None;
    }

    fn activate(&mut self, control: Control) -> Update {
        match control {
            Control::Field if !self.inline => {
                self.keyboard = Some(TextDialogRequest {
                    initial: self.draft.clone(),
                    ..self.binding.request.clone()
                });
                self.error = None;
                Update::Redraw
            }
            Control::Cancel => Update::Close(Answer::Text(None)),
            Control::PromptUp | Control::PromptDown => {
                self.scroll_prompt(if control == Control::PromptUp { -1 } else { 1 });
                Update::from_change(std::mem::take(&mut self.changed))
            }
            Control::Field | Control::Accept => {
                if !self.preedit.is_empty() {
                    return Update::from_change(std::mem::take(&mut self.changed));
                }
                // A platform keyboard can submit on blur during the OK click.
                // Require a corrected submission or explicit keyboard cancellation.
                if !self.inline && self.error.is_some() {
                    return Update::from_change(std::mem::take(&mut self.changed));
                }
                let request = &self.binding.request;
                // Empty submissions keep the initial comment in VmTextDialog::result.
                let result = if self.draft.is_empty() {
                    &request.initial
                } else {
                    &self.draft
                };
                let chars = match comment_glyph_count(result, self.encoding) {
                    Ok(count) => count,
                    Err(error) => {
                        self.error = Some(error);
                        return Update::Redraw;
                    }
                };
                if result != &request.initial
                    && (result.len() > request.byte_limit() as usize
                        || chars > request.character_limit(self.encoding) as usize)
                {
                    self.error = Some(b"COMMENT IS TOO LONG");
                    return Update::Redraw;
                }
                Update::Close(Answer::Text(Some(self.draft.clone())))
            }
        }
    }

    pub(super) fn update(
        &mut self,
        input: &mut ActionInput,
        rect: [i64; 4],
        backend: &mut impl PlatformBackend,
    ) -> Update {
        if let Some(request) = &self.keyboard {
            let answer = backend.poll_text_dialog(request);
            if matches!(answer, Ok(TextDialogResult::Pending)) {
                return Update::Idle;
            }
            self.keyboard = None;
            match answer {
                Ok(TextDialogResult::Accepted(bytes)) => {
                    self.replace(0..self.draft.len(), &bytes);
                }
                Ok(TextDialogResult::Cancelled) => {}
                _ => self.error = Some(b"COULD NOT ENTER COMMENT TEXT"),
            }
            dialog::confirmed(input);
            self.changed = true;
        }

        let (point, inside) = dialog::hit_point(rect, input.pointer());
        let over = self
            .controls()
            .iter()
            .copied()
            .find(|button| inside && dialog::contains(button.rect(), point))
            // Ignore clicks begun during composition even if it ends before mouseup.
            .filter(|&button| self.preedit.is_empty() || button == Control::Cancel);
        let mouse = input.mouse(0);
        if mouse[2]
            && let Some(button) = over
        {
            if self.inline && button == Control::Field {
                self.place_caret(point[0]);
            }
            self.set_focus(button);
        }
        if let Some(button) = self.pointer.update(over, mouse)
            && !(self.inline && button == Control::Field)
        {
            return self.activate(button);
        }
        if self.pointer.cancelled(input) {
            return self.activate(Control::Cancel);
        }
        let [horizontal, vertical] = dialog::axes(input);
        let direction = if vertical != 0 { vertical } else { horizontal };
        if direction != 0 {
            self.move_focus(direction);
        }
        let wheel = isize::from(input.cvns_wait_interrupt(2, true))
            - isize::from(input.cvns_wait_interrupt(1, true));
        self.scroll_prompt(wheel);
        if dialog::confirmed(input) {
            return self.activate(self.focus);
        }
        Update::from_change(std::mem::take(&mut self.changed))
    }

    pub(super) fn describe(&self, page: &mut dialog::DialogPage, decode: impl Fn(u16) -> char) {
        let string = |codes: &[u16]| codes.iter().copied().map(&decode).collect::<String>();
        let title = wrap_text(&self.binding.request.title, self.encoding, 362);
        let title = if self.binding.request.title.is_empty() {
            "Edit comment".into()
        } else {
            string(&title[0])
        };
        page.text([18, 14, 364, 24], title);
        let prompt = self
            .prompt
            .iter()
            .skip(self.prompt_top)
            .take(PROMPT_ROWS)
            .map(|line| string(line))
            .collect::<Vec<_>>()
            .join("\n");
        page.text([18, 57, 328, 46], prompt);
        if let Some(error) = self.error {
            page.text(
                [18, 165, 364, 16],
                String::from_utf8_lossy(error).into_owned(),
            );
        }
        for &control in self.controls() {
            let label = match control {
                Control::Field => {
                    let draft = string(&self.field_layout().codes);
                    if draft.is_empty() {
                        "Enter comment".into()
                    } else {
                        draft
                    }
                }
                Control::Cancel => "Cancel".into(),
                Control::Accept => "OK".into(),
                Control::PromptUp => "Scroll up".into(),
                Control::PromptDown => "Scroll down".into(),
            };
            page.control(control.rect(), label, None);
        }
    }

    pub(super) fn render(
        &self,
        mut font: Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    ) -> Panel {
        let mut panel = Panel::new();
        let title = &self.binding.request.title;
        let title = if title.is_empty() {
            b"EDIT COMMENT".as_slice()
        } else {
            title
        };
        draw_line(
            &mut panel,
            &mut font,
            [18, 14],
            &wrap_text(title, self.encoding, 362)[0],
            INK,
        );
        for (row, line) in self
            .prompt
            .iter()
            .skip(self.prompt_top)
            .take(PROMPT_ROWS)
            .enumerate()
        {
            draw_line(&mut panel, &mut font, [18, 57 + row * 23], line, INK);
        }
        if self.prompt.len() > PROMPT_ROWS {
            panel.button(Control::PromptUp.rect(), b"^", false);
            panel.button(Control::PromptDown.rect(), b"V", false);
        }
        panel.fill(FIELD, [13, 22, 36]);
        panel.outline(FIELD, MUTED);
        if self.draft.is_empty() && self.preedit.is_empty() && !self.inline_focused() {
            panel.text([28, 131], b"ENTER COMMENT", MUTED);
        } else {
            self.draw_field(&mut panel, &mut font);
        }
        if let Some(error) = self.error {
            panel.text([18, 165], error, [255, 177, 153]);
        }
        panel.button(CANCEL_BUTTON, b"CANCEL", false);
        panel.button(ACCEPT_BUTTON, b"OK", true);
        panel.focus(self.focus.rect());
        panel
    }

    fn place_caret(&mut self, x: usize) {
        let FieldLayout { codes, first, .. } = self.field_layout();
        let mut x = x.saturating_sub(FIELD_ORIGIN[0]);
        let mut index = first;
        for code in codes {
            let advance = glyph_advance(code);
            if x < advance / 2 {
                break;
            }
            index += 1;
            x = x.saturating_sub(advance);
        }
        self.caret = character_boundaries(&self.draft, self.encoding)
            .nth(index)
            .unwrap_or(self.draft.len());
        self.select_all = false;
        self.changed = true;
    }

    fn field_layout(&self) -> FieldLayout {
        let glyphs = |bytes| wrap_text(bytes, self.encoding, usize::MAX).remove(0);
        let (before, after) = if self.select_all && !self.preedit.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            (
                glyphs(&self.draft[..self.caret]),
                glyphs(&self.draft[self.caret..]),
            )
        };
        let preedit = glyphs(&self.preedit);
        let preedit_width = line_width(&preedit);
        let mut first = 0;
        let mut before_width = 0;
        let mut codes: Vec<_> = if self.inline_focused() {
            let max_before = FIELD_TEXT_WIDTH - 1 - preedit_width.min(FIELD_TEXT_WIDTH - 20);
            before_width = line_width(&before);
            while before_width > max_before {
                before_width -= glyph_advance(before[first]);
                first += 1;
            }
            before[first..]
                .iter()
                .chain(&preedit)
                .chain(&after)
                .copied()
                .collect()
        } else {
            glyphs(&self.draft)
        };
        let mut used = 0;
        let count = codes
            .iter()
            .take_while(|&&code| {
                used += glyph_advance(code);
                used <= FIELD_TEXT_WIDTH
            })
            .count();
        codes.truncate(count);
        FieldLayout {
            codes,
            first,
            before_width,
            preedit_width: preedit_width.min(FIELD_TEXT_WIDTH - before_width),
        }
    }

    fn draw_field(
        &self,
        panel: &mut Panel,
        font: &mut Option<&mut (dyn GlyphRasterizer + Send + 'static)>,
    ) {
        let FieldLayout {
            codes,
            before_width,
            preedit_width,
            ..
        } = self.field_layout();
        let [x, y] = FIELD_ORIGIN;
        if self.select_all && self.preedit.is_empty() && !codes.is_empty() {
            panel.fill([x - 2, y - 2, line_width(&codes) + 2, 25], [50, 76, 112]);
        }
        draw_line(panel, font, [x, y], &codes, INK);
        if self.inline_focused() && !self.select_all {
            panel.fill([x + before_width, y + 1, 1, 21], FOCUS);
        }
        if self.inline_focused() && preedit_width > 0 {
            panel.fill([x + before_width, y + 21, preedit_width, 1], FOCUS);
        }
    }
}

fn line_width(codes: &[u16]) -> usize {
    codes.iter().copied().map(glyph_advance).sum()
}

fn comment_glyph_count(bytes: &[u8], encoding: TextEncoding) -> Result<usize, &'static [u8]> {
    const UNSUPPORTED: &[u8] = b"UNSUPPORTED CHARACTERS";
    let mut i = 0;
    let mut count = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if matches!(byte, b'{' | b'\\') {
            return Err(b"NO OPEN BRACE OR BACKSLASH");
        }
        // The host GBK codec encodes the euro sign as 0x80, but the VM would
        // consume the following character as its trail byte.
        if byte.is_ascii_control() || (encoding == TextEncoding::Gbk && byte == 0x80) {
            return Err(UNSUPPORTED);
        }
        let wide = encoding
            .wide_at(bytes, i, "truncated comment glyph")
            .map_err(|_| UNSUPPORTED)?;
        i += wide.map_or(1, |(_, len)| len);
        count += 1;
    }
    Ok(count)
}

fn character_boundaries(bytes: &[u8], encoding: TextEncoding) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors(Some(0), move |&offset| {
        let first = *bytes.get(offset)?;
        let len = encoding.wide_len(first).unwrap_or(1);
        Some((offset + len).min(bytes.len()))
    })
}

// Japanese scripts may include "full-width only" directly before their character limit.
fn remove_full_width_notice(prompt: &mut Vec<u8>, encoding: TextEncoding) {
    let notice: &[u8] = match encoding {
        TextEncoding::ShiftJis => b"\x91\x53\x8a\x70\x82\xcc\x82\xdd",
        TextEncoding::Gbk => b"\xc8\xab\xbd\xc7\xa4\xce\xa4\xdf",
        TextEncoding::Utf8 => "全角のみ".as_bytes(),
    };
    let start = character_boundaries(prompt, encoding).find(|&i| prompt[i..].starts_with(notice));
    if let Some(start) = start {
        prompt.drain(start..start + notice.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_glyphs_follow_the_game_scanner_at_character_boundaries() {
        for (encoding, bytes) in [
            (TextEncoding::ShiftJis, b"chapter {1}".as_slice()),
            (TextEncoding::ShiftJis, b"a\\n"),
            (TextEncoding::Gbk, b"\x80"),
            (TextEncoding::Gbk, b"\x80A"),
            (TextEncoding::Utf8, "😀".as_bytes()),
        ] {
            assert!(comment_glyph_count(bytes, encoding).is_err());
        }
        assert_eq!(
            comment_glyph_count(b"/1} \x83\x5c\x83\x7b", TextEncoding::ShiftJis),
            Ok(6)
        );
        assert_eq!(
            comment_glyph_count("記録1".as_bytes(), TextEncoding::Utf8),
            Ok(3)
        );
    }

    #[test]
    fn comment_prompt_keeps_the_script_limit_when_removing_the_width_notice() {
        for (encoding, notice) in [
            (
                TextEncoding::ShiftJis,
                b"\x91\x53\x8a\x70\x82\xcc\x82\xdd".as_slice(),
            ),
            (TextEncoding::Gbk, b"\xc8\xab\xbd\xc7\xa4\xce\xa4\xdf"),
            (TextEncoding::Utf8, "全角のみ".as_bytes()),
        ] {
            let mut prompt = [b"(".as_slice(), notice, b"32)"].concat();
            remove_full_width_notice(&mut prompt, encoding);
            assert_eq!(prompt, b"(32)");
        }
    }
}

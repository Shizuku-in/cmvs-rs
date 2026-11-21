//! Accessible content from a completed logical frame, before viewport scaling.
use cmvs_core::{
    TextEncoding,
    render::message::{AUXILIARY_TEXTS, MESSAGE_WINDOWS, TEXT_OBJECTS},
    runtime::{Runtime, controls},
};
use cmvs_font::font::character;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Choice {
    id: [usize; 2],
    rect: [i32; 4],
    label: String,
}

impl Choice {
    fn contains(&self, [x, y]: [i32; 2]) -> bool {
        let [left, top, w, h] = self.rect;
        (left..left + w).contains(&x) && (top..top + h).contains(&y)
    }
}

#[derive(PartialEq, Eq, Serialize)]
struct TextBlock {
    id: usize,
    value: String,
    rect: [i32; 4],
}

#[derive(Default, PartialEq, Eq, Serialize)]
pub(crate) struct Page {
    #[serde(skip)]
    script: Vec<u8>,
    text: Vec<TextBlock>,
    choices: Vec<Choice>,
}

impl Page {
    pub fn read(run: &Runtime, encoding: TextEncoding) -> Self {
        let mut page = Self::default();
        if run.movies().has_player() {
            return page;
        }
        page.script = run.machine().current_name().to_vec();
        let size = run.screen_size().map(|v| v as i32);
        for slot in 0..controls::GROUPS {
            if let Some(group) = run.buttons().group(slot)
                && !run
                    .scene()
                    .object(group.object)
                    .is_some_and(|obj| obj.shown)
            {
                continue;
            }
            if let Some(group) = controls::group(run, slot) {
                for i in 0..group.len() {
                    if group.enabled(i)
                        && let Some(rect) = group.pointer_rect(i)
                    {
                        page.add_choice([slot, i], rect, size);
                    }
                }
            }
        }
        let messages = run.messages();
        for slot in 0..MESSAGE_WINDOWS {
            if let Some(win) = messages.window(slot).filter(|w| w.visible && !w.hidden) {
                let Some(text) = messages.text(win.text) else {
                    continue;
                };
                if text.logical_size.iter().any(|&v| v <= 0) {
                    continue;
                }
                let device = |n: i32, axis: usize| {
                    (i64::from(n) * i64::from(size[axis]) / i64::from(text.logical_size[axis]))
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                };
                for (i, b) in win.buttons.iter().enumerate().filter(|(_, b)| b.enabled) {
                    let [x, y, w, h] = b.hit.map(i32::from);
                    let x = win.position[0].saturating_add(x);
                    let y = win.position[1].saturating_add(y);
                    let left = device(x, 0);
                    let top = device(y, 1);
                    let right = device(x.saturating_add(w), 0);
                    let bottom = device(y.saturating_add(h), 1);
                    page.add_choice(
                        [controls::GROUPS + slot, i],
                        [
                            left,
                            top,
                            right.saturating_sub(left),
                            bottom.saturating_sub(top),
                        ],
                        size,
                    );
                }
            }
        }

        if run.scene().cvns().is_none() {
            for (i, &rect) in run.input().hit_rects().iter().enumerate() {
                page.add_choice([controls::GROUPS + MESSAGE_WINDOWS, i], rect, size);
            }
        }

        for slot in 0..TEXT_OBJECTS {
            if let Some(text) = messages
                .text(slot)
                .filter(|t| t.visible && t.logical_size[0] > 0)
            {
                let logical = size[0] as f32 / text.logical_size[0] as f32;
                let scale = f32::from_bits(text.scale_bits);
                if scale <= 0.0 {
                    continue;
                }
                page.add_glyphs(
                    slot,
                    encoding,
                    size,
                    text.glyphs
                        .iter()
                        .filter(|g| g.shown && g.opacity != 0)
                        .map(|g| {
                            let center = std::array::from_fn(|i| {
                                ((text.position[i] as f32
                                    + (g.position[i] as f32 + g.size as f32 / 2.0) * scale)
                                    * logical) as i32
                            });
                            (g.code, center, (g.size as f32 * scale * logical) as i32)
                        }),
                );
            }
        }
        for slot in 0..AUXILIARY_TEXTS {
            if let Some(t) = messages.auxiliary_text(slot)
                && let Ok(origin) = t.glyph_origin()
            {
                page.add_glyphs(
                    TEXT_OBJECTS + slot,
                    encoding,
                    size,
                    t.attached_glyphs().map(|g| {
                        let center = std::array::from_fn(|i| {
                            origin[i]
                                .saturating_add(g.position[i])
                                .saturating_add((g.size / 2) as i32)
                        });
                        (g.code, center, g.size as i32)
                    }),
                );
            }
        }
        page
    }

    fn add_choice(&mut self, id: [usize; 2], [x, y, w, h]: [i32; 4], screen: [i32; 2]) {
        let left = x.max(0);
        let top = y.max(0);
        let right = x.saturating_add(w).min(screen[0]);
        let bottom = y.saturating_add(h).min(screen[1]);
        let rect = [
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ];
        if right > left
            && bottom > top
            && rect != [0, 0, screen[0], screen[1]]
            && !self.choices.iter().any(|choice| choice.rect == rect)
        {
            self.choices.push(Choice {
                id,
                rect,
                label: String::new(),
            });
        }
    }

    // Associate glyphs spatially: two branches may share a text object or the same baseline.
    fn add_glyphs(
        &mut self,
        id: usize,
        encoding: TextEncoding,
        screen: [i32; 2],
        glyphs: impl Iterator<Item = (u16, [i32; 2], i32)>,
    ) {
        let mut text = String::new();
        let mut last = None;
        let mut lo = screen;
        let mut hi = [0; 2];
        for (code, at, size) in glyphs {
            if !(0..screen[0]).contains(&at[0]) || !(0..screen[1]).contains(&at[1]) {
                continue;
            }
            let ch = character(encoding, code).unwrap_or(char::REPLACEMENT_CHARACTER);
            if let Some(choice) = self.choices.iter_mut().rev().find(|c| c.contains(at)) {
                if last != Some(choice.id) && !choice.label.is_empty() {
                    choice.label.push(' ');
                }
                choice.label.push(ch);
                last = Some(choice.id);
            } else {
                text.push(ch);
                for axis in 0..2 {
                    let start = at[axis].saturating_sub(size / 2);
                    lo[axis] = lo[axis].min(start.max(0));
                    hi[axis] = hi[axis].max(start.saturating_add(size.max(1)).min(screen[axis]));
                }
                last = None;
            }
        }
        if !text.trim().is_empty() {
            self.text.push(TextBlock {
                id,
                value: text,
                rect: [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]],
            });
        }
    }
}

// Validate the displayed choice again between hover, press, and release. A changed page
// must never turn a VoiceOver activation into a click on the next screen.
#[derive(Default)]
pub(crate) struct Reader {
    pub page: Page,
    pub revision: u64,
    pending: Option<(u64, usize, Click)>,
}

enum Click {
    Hover,
    Press,
    Release,
}

impl Reader {
    pub fn update(&mut self, page: Page) {
        if self.revision == 0 || page != self.page {
            self.page = page;
            self.revision += 1;
        }
    }

    pub fn choose(&mut self, revision: u64, index: usize) {
        if revision == self.revision && index < self.page.choices.len() && self.pending.is_none() {
            self.pending = Some((revision, index, Click::Hover));
        }
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn sample(
        &mut self,
        current: impl FnOnce() -> Page,
        input: &mut cmvs_host::InputState,
    ) -> bool {
        let Some((revision, index, stage)) = self.pending.take() else {
            return false;
        };
        if revision != self.revision || current() != self.page {
            input.cancel_primary_button();
            return true;
        }
        let [x, y, w, h] = self.page.choices[index].rect;
        input.set_pointer(x + w / 2, y + h / 2);
        match stage {
            Click::Hover => {
                input.cancel_primary_button();
                self.pending = Some((revision, index, Click::Press));
            }
            Click::Press => {
                input.pulse(1).expect("valid primary button");
                self.pending = Some((revision, index, Click::Release));
            }
            Click::Release => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::host::input::{ActionInput, KeyBindings};
    use cmvs_host::InputState;

    fn options() -> Page {
        let mut page = Page::default();
        let size = [80, 60];
        page.add_choice([0, 0], [-10, 30, 40, 30], size);
        page.add_choice([0, 1], [50, 30, 40, 30], size);
        page.add_glyphs(
            0,
            TextEncoding::ShiftJis,
            size,
            [
                (0x82a0, [0, 1], 2),
                (0x82a2, [4, 10], 2), // wrapped dialogue
                (0x41, [15, 40], 2),
                (0x42, [65, 40], 2), // adjacent choices
            ]
            .into_iter(),
        );
        page
    }

    #[test]
    fn labels_follow_visible_bounds_and_use_the_game_encoding() {
        let p = options();
        assert_eq!(p.text[0].value, "あい");
        assert_eq!(p.choices[0].rect, [0, 30, 30, 30]);
        assert_eq!(p.choices[0].label, "A");
        assert_eq!(p.choices[1].rect, [50, 30, 30, 30]);
        assert_eq!(p.choices[1].label, "B");
    }

    #[test]
    fn activation_samples_the_selected_button_and_cancels_on_page_change() {
        let mut reader = Reader::default();
        let mut input = InputState::default();
        let mut actions = ActionInput::default();
        let keys = KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=1\n").unwrap();
        reader.update(options());
        reader.choose(reader.revision, 1);
        for (pressed, released) in [(false, false), (true, false), (false, true)] {
            assert!(reader.sample(options, &mut input));
            input.sample(&keys, &mut actions);
            assert_eq!(actions.pointer(), [65, 45]);
            assert_eq!(
                (actions.pressed(0), actions.released(0)),
                (pressed, released)
            );
        }

        reader.choose(reader.revision, 0);
        reader.sample(options, &mut input);
        reader.sample(Page::default, &mut input);
        input.sample(&keys, &mut actions);
        assert!(!actions.pressed(0));
        assert!(!reader.sample(options, &mut input));
        let stale = reader.revision;
        reader.update(Page::default());
        reader.choose(stale, 0);
        assert!(!reader.sample(options, &mut input));
    }
}

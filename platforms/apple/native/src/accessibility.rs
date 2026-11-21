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
    #[serde(skip_serializing_if = "Option::is_none")]
    slider: Option<u8>,
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

fn text_scale(scale_bits: u32) -> Option<f32> {
    let scale = f32::from_bits(scale_bits);
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

#[derive(Default, PartialEq, Eq, Serialize)]
pub(crate) struct Page {
    #[serde(skip)]
    script: Vec<u8>,
    modal: bool,
    text: Vec<TextBlock>,
    choices: Vec<Choice>,
}

impl Page {
    pub fn read(run: &Runtime, encoding: TextEncoding) -> Self {
        let mut page = Self::default();
        if let Some(dialog) =
            run.dialog_page(|code| character(encoding, code).unwrap_or(char::REPLACEMENT_CHARACTER))
        {
            page.modal = true;
            page.script = run.machine().current_name().to_vec();
            for (id, (rect, value)) in dialog.text.into_iter().enumerate() {
                page.text.push(TextBlock { id, rect, value });
            }
            for (id, button) in dialog.buttons.into_iter().enumerate() {
                page.choices.push(Choice {
                    id: [controls::GROUPS + MESSAGE_WINDOWS + 1, id],
                    rect: button.rect,
                    label: button.label,
                    slider: button.slider,
                });
            }
            return page;
        }
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
                let Some(scale) = text_scale(text.scale_bits) else {
                    continue;
                };
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
                slider: None,
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

type ChoiceKey = ([usize; 2], [i32; 4]);

impl Page {
    fn choice_keys(&self) -> Vec<ChoiceKey> {
        self.choices.iter().map(|c| (c.id, c.rect)).collect()
    }

    fn same_target(&self, current: &Self, index: usize) -> bool {
        self.script == current.script
            && self
                .choices
                .get(index)
                .zip(current.choices.get(index))
                .is_some_and(|(a, b)| a.id == b.id && a.rect == b.rect)
    }

    fn same_choices(&self, current: &Self, expected: &[ChoiceKey]) -> bool {
        self.script == current.script
            && current.choices.len() == expected.len()
            && current
                .choices
                .iter()
                .zip(expected)
                .all(|(c, &(id, rect))| c.id == id && c.rect == rect)
    }

    fn matches_stage(&self, current: &Self, index: usize, stage: &Click) -> bool {
        match stage {
            Click::Hover => self == current,
            Click::Press if self.modal => self == current,
            Click::Press => self.same_target(current, index),
            Click::Release(expected) => self.same_choices(current, expected),
        }
    }
}

// Validate the displayed choice again between hover, press, and release. A changed page
// must never turn a VoiceOver activation into a click on the next screen, while allowing
// hover-induced description text updates on the same choice.
#[derive(Default)]
pub(crate) struct Reader {
    pub page: Page,
    pub revision: u64,
    pending: Option<Activation>,
}

struct Activation {
    revision: u64,
    index: usize,
    point: [i32; 2],
    stage: Click,
}

enum Click {
    Hover,
    Press,
    Release(Vec<ChoiceKey>),
}

impl Reader {
    pub fn update(&mut self, page: Page) {
        if self.revision == 0 || page != self.page {
            if let Some(action) = &self.pending
                && self.page.matches_stage(&page, action.index, &action.stage)
            {
                return;
            }
            self.page = page;
            self.revision += 1;
        }
    }

    pub fn choose(&mut self, revision: u64, index: usize, slider: Option<u8>) {
        let Some(choice) = self.page.choices.get(index) else {
            return;
        };
        if revision != self.revision || self.pending.is_some() {
            return;
        }
        let [x, y, w, h] = choice.rect;
        let dx = match (choice.slider, slider) {
            (None, None) => w / 2,
            (Some(current), Some(value)) => {
                let scaled = i64::from(w - 1) * i64::from(value);
                ((scaled + if value > current { 254 } else { 0 }) / 255) as i32
            }
            _ => return,
        };
        self.pending = Some(Activation {
            revision,
            index,
            point: [x + dx, y + h / 2],
            stage: Click::Hover,
        });
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn sample(
        &mut self,
        current: impl FnOnce() -> Page,
        input: &mut cmvs_host::InputState,
    ) -> bool {
        let Some(mut action) = self.pending.take() else {
            return false;
        };
        let current = current();
        if action.revision != self.revision
            || !self
                .page
                .matches_stage(&current, action.index, &action.stage)
        {
            input.cancel_primary_button();
            return true;
        }
        let [x, y] = action.point;
        input.set_pointer(x, y);
        action.stage = match action.stage {
            Click::Hover => {
                input.cancel_primary_button();
                Click::Press
            }
            Click::Press => {
                input.pulse(1).expect("valid primary button");
                Click::Release(current.choice_keys())
            }
            Click::Release(_) => return true,
        };
        self.pending = Some(action);
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
        reader.choose(reader.revision, 1, None);
        for (pressed, released) in [(false, false), (true, false), (false, true)] {
            assert!(reader.sample(options, &mut input));
            input.sample(&keys, &mut actions);
            assert_eq!(actions.pointer(), [65, 45]);
            assert_eq!(
                (actions.pressed(0), actions.released(0)),
                (pressed, released)
            );
        }

        reader.choose(reader.revision, 0, None);
        reader.sample(options, &mut input);
        reader.sample(Page::default, &mut input);
        input.sample(&keys, &mut actions);
        assert!(!actions.pressed(0));
        assert!(!reader.sample(options, &mut input));
        let stale = reader.revision;
        reader.update(Page::default());
        reader.choose(stale, 0, None);
        assert!(!reader.sample(options, &mut input));
    }

    #[test]
    fn replacement_label_before_hover_cancels_activation() {
        let mut reader = Reader::default();
        let mut input = InputState::default();
        reader.update(options());
        reader.choose(reader.revision, 1, None);
        let replacement = || {
            let mut page = options();
            page.choices[1].label = "Another option".into();
            page
        };
        assert!(reader.sample(replacement, &mut input));
        assert!(!reader.sample(replacement, &mut input));
    }

    #[test]
    fn hover_text_change_preserves_activation_and_press_page_change_cancels_release() {
        let mut reader = Reader::default();
        let mut input = InputState::default();
        let mut actions = ActionInput::default();
        let keys = KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=1\n").unwrap();

        let hovered = || {
            let mut page = options();
            page.text[0].value = "Hover description".into();
            page.choices[1].label = "B (hover)".into();
            page
        };

        reader.update(options());
        reader.choose(reader.revision, 1, None);

        // Frame 1: Hover moves pointer without pressing.
        assert!(reader.sample(options, &mut input));
        input.sample(&keys, &mut actions);
        assert_eq!((actions.pressed(0), actions.released(0)), (false, false));

        // An interleaved accessibility poll after hover changes text/label must not break Press.
        reader.update(hovered());
        assert_eq!(reader.revision, 1);

        // Frame 2: Press succeeds even though hover changed text and choice label.
        assert!(reader.sample(hovered, &mut input));
        input.sample(&keys, &mut actions);
        assert_eq!((actions.pressed(0), actions.released(0)), (true, false));

        // Frame 3: Release completes when choices/script remain intact.
        reader.update(hovered());
        assert!(reader.sample(hovered, &mut input));
        input.sample(&keys, &mut actions);
        assert_eq!((actions.pressed(0), actions.released(0)), (false, true));
        assert!(!reader.sample(hovered, &mut input));

        // Once pending is cleared, the next update publishes the hovered text revision.
        reader.update(hovered());
        assert_eq!(reader.revision, 2);

        // If Press changes the available choices, Release is cancelled to protect the next screen.
        reader.choose(reader.revision, 0, None);
        assert!(reader.sample(hovered, &mut input));
        input.sample(&keys, &mut actions);
        assert!(reader.sample(hovered, &mut input));
        input.sample(&keys, &mut actions);
        assert_eq!((actions.pressed(0), actions.released(0)), (true, false));
        assert!(reader.sample(Page::default, &mut input));
        input.sample(&keys, &mut actions);
        assert!(!actions.released(0));
    }

    #[test]
    fn text_scale_rejects_non_finite_and_non_positive_values() {
        assert_eq!(text_scale(1.5f32.to_bits()), Some(1.5));
        for bits in [
            0.0f32.to_bits(),
            (-0.0f32).to_bits(),
            (-1.0f32).to_bits(),
            f32::NAN.to_bits(),
            0x7fc0_1234,
            f32::INFINITY.to_bits(),
            f32::NEG_INFINITY.to_bits(),
        ] {
            assert_eq!(text_scale(bits), None);
        }
    }
}

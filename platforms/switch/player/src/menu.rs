//! Game selection screen, shown when more than one game is installed.
use crate::{
    SCREEN,
    canvas::{BRIGHT, Canvas, DIM, HIGHLIGHT, MARGIN, TEXT},
    library::GameEntry,
};
use ab_glyph::FontVec;
use cmvs_core::raster::RgbaImage;
use cmvs_native::{SwitchInput, button};

/// Rows shown at once.
pub const ROWS: usize = 10;
const LIST_TOP: i32 = 136;
const ROW_HEIGHT: i32 = 52;
const STICK_THRESHOLD: i32 = 16000;
const REPEAT_DELAY_MS: u32 = 400;
const REPEAT_MS: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Idle,
    /// The selection or scroll changed; redraw.
    Moved,
    Launch(usize),
    Exit,
}

pub struct Menu {
    pub selected: usize,
    /// First visible row.
    pub top: usize,
    len: usize,
    /// Buttons held at the previous sample; presses need a fresh edge.
    held: u32,
    /// Direction held for repeats, with the time until the next step.
    repeat: Option<(i32, u32)>,
    /// The touch that began on a row, and that row.
    touch: Option<(u32, Option<usize>)>,
    /// Touches already down when the menu opened are ignored until released.
    stale_touches: Vec<u32>,
}

impl Menu {
    /// `initial` is the current sample: anything already held must be released first.
    pub fn new(selected: usize, len: usize, initial: &SwitchInput) -> Self {
        let mut menu = Self {
            selected: selected.min(len.saturating_sub(1)),
            top: 0,
            len,
            held: initial.buttons | direction_bits(initial),
            repeat: None,
            touch: None,
            stale_touches: initial.touches.iter().map(|t| t.id).collect(),
        };
        menu.scroll_to_selection();
        menu
    }

    fn scroll_to_selection(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
    }

    fn select(&mut self, index: usize) -> MenuAction {
        let index = index.min(self.len.saturating_sub(1));
        if index == self.selected {
            return MenuAction::Idle;
        }
        self.selected = index;
        self.scroll_to_selection();
        MenuAction::Moved
    }

    fn step(&mut self, by: i32) -> MenuAction {
        let target = (self.selected as i64 + i64::from(by)).clamp(0, self.len as i64 - 1);
        self.select(target as usize)
    }

    pub fn poll(&mut self, sample: &SwitchInput, dt_ms: u32) -> MenuAction {
        if !sample.focused {
            // Input from HOME or an overlay never reaches the menu.
            self.held = sample.buttons | direction_bits(sample);
            self.repeat = None;
            self.touch = None;
            self.stale_touches = sample.touches.iter().map(|t| t.id).collect();
            return MenuAction::Idle;
        }
        let now = sample.buttons | direction_bits(sample);
        let pressed = now & !self.held;
        self.held = now;
        if pressed & button::PLUS != 0 {
            return MenuAction::Exit;
        }
        if pressed & button::A != 0 {
            return MenuAction::Launch(self.selected);
        }
        let mut action = MenuAction::Idle;
        if pressed & button::L != 0 {
            action = self.step(-(ROWS as i32));
        } else if pressed & button::R != 0 {
            action = self.step(ROWS as i32);
        }
        let direction = match (now & button::UP != 0, now & button::DOWN != 0) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        match (direction, self.repeat) {
            (0, _) => self.repeat = None,
            (d, Some((held, wait))) if d == held => {
                if wait <= dt_ms {
                    self.repeat = Some((d, REPEAT_MS - (dt_ms - wait).min(REPEAT_MS - 1)));
                    action = self.step(d);
                } else {
                    self.repeat = Some((d, wait - dt_ms));
                }
            }
            (d, _) => {
                self.repeat = Some((d, REPEAT_DELAY_MS));
                action = self.step(d);
            }
        }
        match self.track_touch(sample) {
            MenuAction::Idle => action,
            touch => touch,
        }
    }

    /// A tap selects a row; tapping the selected row starts it.
    fn track_touch(&mut self, sample: &SwitchInput) -> MenuAction {
        self.stale_touches
            .retain(|id| sample.touches.iter().any(|t| t.id == *id));
        let current = sample
            .touches
            .iter()
            .find(|t| !self.stale_touches.contains(&t.id));
        match (self.touch, current) {
            (None, Some(touch)) => {
                self.touch = Some((touch.id, self.row_at(touch.position)));
                MenuAction::Idle
            }
            (Some((id, _)), Some(touch)) if touch.id == id => MenuAction::Idle,
            (Some((_, row)), _) => {
                self.touch = None;
                let released = sample.touches.is_empty();
                match row {
                    Some(row) if released && row == self.selected => MenuAction::Launch(row),
                    Some(row) if released => self.select(row),
                    _ => MenuAction::Idle,
                }
            }
            (None, None) => MenuAction::Idle,
        }
    }

    fn row_at(&self, [x, y]: [u32; 2]) -> Option<usize> {
        let (x, y) = (x as i32, y as i32);
        if x < MARGIN || x >= SCREEN[0] as i32 - MARGIN || y < LIST_TOP {
            return None;
        }
        let row = ((y - LIST_TOP) / ROW_HEIGHT) as usize;
        (row < ROWS && self.top + row < self.len).then_some(self.top + row)
    }
}

/// Up/down from the D-pad or either stick.
fn direction_bits(sample: &SwitchInput) -> u32 {
    let mut bits = sample.buttons & (button::UP | button::DOWN);
    for [_, y] in sample.sticks {
        if y > STICK_THRESHOLD {
            bits |= button::UP;
        } else if y < -STICK_THRESHOLD {
            bits |= button::DOWN;
        }
    }
    bits
}

/// Renders the screen; `loading` replaces the controls hint once a game is chosen.
pub fn draw(menu: &Menu, games: &[GameEntry], font: &FontVec, loading: bool) -> RgbaImage {
    let mut canvas = Canvas::new();
    let width = SCREEN[0] as i32 - 2 * MARGIN;
    canvas.text(
        font,
        44.0,
        [MARGIN as f32, 92.0],
        width as f32,
        "CMVS",
        BRIGHT,
    );
    let count = format!("{} / {}", menu.selected + 1, games.len());
    let count_width = Canvas::text_width(font, 26.0, &count);
    canvas.text(
        font,
        26.0,
        [(SCREEN[0] as i32 - MARGIN) as f32 - count_width, 92.0],
        count_width + 1.0,
        &count,
        DIM,
    );
    for (row, game) in games.iter().enumerate().skip(menu.top).take(ROWS) {
        let y = LIST_TOP + (row - menu.top) as i32 * ROW_HEIGHT;
        let selected = row == menu.selected;
        if selected {
            canvas.fill([MARGIN, y, width, ROW_HEIGHT - 4], HIGHLIGHT);
        }
        canvas.text(
            font,
            30.0,
            [(MARGIN + 20) as f32, (y + 36) as f32],
            (width - 40) as f32,
            &game.name,
            if selected { BRIGHT } else { TEXT },
        );
    }
    if menu.top > 0 {
        canvas.fill([SCREEN[0] as i32 / 2 - 12, LIST_TOP - 10, 24, 3], DIM);
    }
    if menu.top + ROWS < games.len() {
        let y = LIST_TOP + ROWS as i32 * ROW_HEIGHT + 4;
        canvas.fill([SCREEN[0] as i32 / 2 - 12, y, 24, 3], DIM);
    }
    let hint = if loading {
        format!("Loading {}…", games[menu.selected].name)
    } else {
        "A: Start    L / R: Page    +: Exit".into()
    };
    canvas.text(font, 24.0, [MARGIN as f32, 692.0], width as f32, &hint, DIM);
    canvas.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_native::Touch;

    fn sample(buttons: u32) -> SwitchInput {
        SwitchInput {
            focused: true,
            buttons,
            ..SwitchInput::default()
        }
    }
    fn touch(id: u32, position: [u32; 2]) -> SwitchInput {
        SwitchInput {
            focused: true,
            touches: vec![Touch { id, position }],
            ..SwitchInput::default()
        }
    }

    #[test]
    fn held_buttons_need_release_and_directions_repeat_after_a_delay() {
        let mut menu = Menu::new(0, 25, &sample(button::A | button::PLUS));
        assert_eq!(
            menu.poll(&sample(button::A | button::PLUS), 16),
            MenuAction::Idle
        );
        assert_eq!(menu.poll(&sample(0), 16), MenuAction::Idle);

        assert_eq!(menu.poll(&sample(button::DOWN), 16), MenuAction::Moved);
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.poll(&sample(button::DOWN), 300), MenuAction::Idle);
        assert_eq!(menu.poll(&sample(button::DOWN), 100), MenuAction::Moved);
        assert_eq!(menu.poll(&sample(button::DOWN), 100), MenuAction::Moved);
        assert_eq!(menu.selected, 3);
        let stick = SwitchInput {
            sticks: [[0, 30000], [0, 0]],
            ..sample(0)
        };
        assert_eq!(menu.poll(&stick, 16), MenuAction::Moved);
        assert_eq!(menu.selected, 2);

        assert_eq!(menu.poll(&sample(button::R), 16), MenuAction::Moved);
        assert_eq!((menu.selected, menu.top), (12, 3));
        menu.poll(&sample(0), 16);
        assert_eq!(menu.poll(&sample(button::R), 16), MenuAction::Moved);
        menu.poll(&sample(0), 16);
        assert_eq!(menu.poll(&sample(button::R), 16), MenuAction::Moved);
        assert_eq!((menu.selected, menu.top), (24, 15));
        menu.poll(&sample(0), 16);
        assert_eq!(menu.poll(&sample(button::A), 16), MenuAction::Launch(24));
        menu.poll(&sample(0), 16);
        assert_eq!(menu.poll(&sample(button::PLUS), 16), MenuAction::Exit);
    }

    #[test]
    fn taps_select_then_start_and_unfocused_input_is_ignored() {
        let mut menu = Menu::new(0, 3, &touch(7, [200, LIST_TOP as u32 + 60]));
        // The touch held while the menu opened does nothing.
        assert_eq!(
            menu.poll(&touch(7, [200, LIST_TOP as u32 + 60]), 16),
            MenuAction::Idle
        );
        assert_eq!(menu.poll(&sample(0), 16), MenuAction::Idle);
        let second_row = [200, LIST_TOP as u32 + ROW_HEIGHT as u32 + 10];
        assert_eq!(menu.poll(&touch(8, second_row), 16), MenuAction::Idle);
        assert_eq!(menu.poll(&sample(0), 16), MenuAction::Moved);
        assert_eq!(menu.selected, 1);
        menu.poll(&touch(9, second_row), 16);
        assert_eq!(menu.poll(&sample(0), 16), MenuAction::Launch(1));
        // Rows past the list and letterbox margins are not targets.
        menu.poll(
            &touch(10, [200, LIST_TOP as u32 + 5 * ROW_HEIGHT as u32]),
            16,
        );
        assert_eq!(menu.poll(&sample(0), 16), MenuAction::Idle);

        let unfocused = SwitchInput {
            focused: false,
            ..sample(button::A)
        };
        assert_eq!(menu.poll(&unfocused, 16), MenuAction::Idle);
        assert_eq!(menu.poll(&sample(button::A), 16), MenuAction::Idle);
    }
}

/// Writes a menu preview: `CMVS_MENU_FONT=<font> CMVS_MENU_PNG=<out.png> cargo test -p
/// cmvs-switch menu_preview -- --ignored`.
#[cfg(test)]
#[test]
#[ignore = "needs a font file; writes a PNG for visual review"]
fn menu_preview() {
    let font = std::fs::read(std::env::var("CMVS_MENU_FONT").expect("CMVS_MENU_FONT")).unwrap();
    let font = FontVec::try_from_vec_and_index(font, 0).unwrap();
    let names = [
        "2016-07-29_AMATSUTSUMI",
        "aoitori",
        "Asunana",
        "ハピメアFD",
        "A very long directory name that certainly does not fit on a single row of the menu",
    ];
    let games: Vec<_> = (0..14)
        .map(|i| GameEntry {
            name: format!("{} {}", names[i % names.len()], i + 1),
            root: std::path::PathBuf::new(),
        })
        .collect();
    let initial = SwitchInput {
        focused: true,
        ..SwitchInput::default()
    };
    let menu = Menu::new(11, games.len(), &initial);
    let image = draw(&menu, &games, &font, false);
    std::fs::write(
        std::env::var("CMVS_MENU_PNG").expect("CMVS_MENU_PNG"),
        image.to_png(),
    )
    .unwrap();
}

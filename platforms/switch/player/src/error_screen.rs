//! Full-screen error page shown before leaving a failed game.
use crate::{
    SCREEN,
    canvas::{Canvas, DIM, MARGIN, TEXT, wrap},
};
use ab_glyph::FontVec;
use cmvs_core::raster::RgbaImage;
use cmvs_native::{SwitchInput, button};

const TITLE: [u8; 3] = [255, 120, 110];
const TEXT_PX: f32 = 24.0;
const LINE_HEIGHT: i32 = 34;
const TEXT_TOP: i32 = 172;
const TEXT_BOTTOM: i32 = 612;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorChoice {
    /// Back to the game list.
    Back,
    Exit,
}

/// A leaves for the game list when there is one, + exits; buttons held when the page
/// opens need a fresh press.
pub struct ErrorPrompt {
    held: u32,
    can_return: bool,
}
impl ErrorPrompt {
    pub fn new(initial: &SwitchInput, can_return: bool) -> Self {
        Self {
            held: initial.buttons,
            can_return,
        }
    }
    pub fn poll(&mut self, sample: &SwitchInput) -> Option<ErrorChoice> {
        let pressed = sample.buttons & !self.held;
        self.held = sample.buttons;
        if !sample.focused {
            return None;
        }
        if pressed & button::PLUS != 0 || (pressed & button::A != 0 && !self.can_return) {
            Some(ErrorChoice::Exit)
        } else if pressed & button::A != 0 {
            Some(ErrorChoice::Back)
        } else {
            None
        }
    }
}

/// `game` names the failed game, when one was running.
pub fn draw(
    font: &FontVec,
    game: Option<&str>,
    message: &str,
    log: &str,
    can_return: bool,
) -> RgbaImage {
    let mut canvas = Canvas::new();
    let width = (SCREEN[0] as i32 - 2 * MARGIN) as f32;
    let title = match game {
        Some(game) => format!("Error: {game}"),
        None => "Error".into(),
    };
    canvas.text(font, 40.0, [MARGIN as f32, 104.0], width, &title, TITLE);
    let lines = wrap(message, width, |line| {
        Canvas::text_width(font, TEXT_PX, line)
    });
    let rows = ((TEXT_BOTTOM - TEXT_TOP) / LINE_HEIGHT) as usize;
    for (row, line) in lines.iter().take(rows).enumerate() {
        let shown = if row + 1 == rows && lines.len() > rows {
            "…"
        } else {
            line.as_str()
        };
        let y = TEXT_TOP + row as i32 * LINE_HEIGHT;
        canvas.text(font, TEXT_PX, [MARGIN as f32, y as f32], width, shown, TEXT);
    }
    canvas.text(
        font,
        22.0,
        [MARGIN as f32, 652.0],
        width,
        &format!("Full message: {log}"),
        DIM,
    );
    let hint = if can_return {
        "A: Back to list    +: Exit"
    } else {
        "A / +: Exit"
    };
    canvas.text(font, 24.0, [MARGIN as f32, 692.0], width, hint, DIM);
    canvas.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(buttons: u32) -> SwitchInput {
        SwitchInput {
            focused: true,
            buttons,
            ..SwitchInput::default()
        }
    }
    #[test]
    fn held_buttons_are_ignored_and_a_returns_only_with_a_list() {
        let mut prompt = ErrorPrompt::new(&sample(button::A), true);
        assert_eq!(prompt.poll(&sample(button::A)), None);
        assert_eq!(prompt.poll(&sample(0)), None);
        assert_eq!(prompt.poll(&sample(button::A)), Some(ErrorChoice::Back));
        assert_eq!(prompt.poll(&sample(button::PLUS)), Some(ErrorChoice::Exit));

        let mut single = ErrorPrompt::new(&sample(0), false);
        let unfocused = SwitchInput {
            focused: false,
            ..sample(button::A)
        };
        assert_eq!(single.poll(&unfocused), None);
        assert_eq!(single.poll(&sample(button::A)), None);
        assert_eq!(single.poll(&sample(0)), None);
        assert_eq!(single.poll(&sample(button::A)), Some(ErrorChoice::Exit));
    }
}

/// Writes an error page preview: `CMVS_MENU_FONT=<font> CMVS_ERROR_PNG=<out.png> cargo
/// test -p cmvs-switch error_preview -- --ignored`.
#[cfg(test)]
#[test]
#[ignore = "needs a font file; writes a PNG for visual review"]
fn error_preview() {
    let font = std::fs::read(std::env::var("CMVS_MENU_FONT").expect("CMVS_MENU_FONT")).unwrap();
    let font = FontVec::try_from_vec_and_index(font, 0).unwrap();
    let message = "2017-11-24_AOITORI: unsupported scene command 0x2311 with arguments that \
        are longer than one line of the error page, so it wraps\nat main_story / 0x1f3a after \
        48213 steps\nゲームのスクリプトを読み込めませんでした";
    let image = draw(
        &font,
        Some("2017-11-24_AOITORI"),
        message,
        "switch/cmvs-rs/error.log",
        true,
    );
    std::fs::write(
        std::env::var("CMVS_ERROR_PNG").expect("CMVS_ERROR_PNG"),
        image.to_png(),
    )
    .unwrap();
}

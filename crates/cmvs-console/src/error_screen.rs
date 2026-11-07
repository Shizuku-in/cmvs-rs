//! Full-screen error page shown before leaving a failed game.
use crate::{
    ButtonMap, Console,
    canvas::{Canvas, DIM, MARGIN, TEXT, wrap},
};
use ab_glyph::FontVec;
use cmvs_core::raster::RgbaImage;
use cmvs_native::PadInput;

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

/// Confirm leaves for the game list when there is one, exit leaves the program;
/// buttons held when the page opens need a fresh press.
pub struct ErrorPrompt {
    confirm: u32,
    exit: u32,
    held: u32,
    can_return: bool,
}
impl ErrorPrompt {
    pub fn new(initial: &PadInput, can_return: bool, buttons: &ButtonMap) -> Self {
        Self {
            confirm: buttons.confirm,
            exit: buttons.exit,
            held: initial.buttons,
            can_return,
        }
    }
    pub fn poll(&mut self, sample: &PadInput) -> Option<ErrorChoice> {
        let pressed = sample.buttons & !self.held;
        self.held = sample.buttons;
        if !sample.focused {
            return None;
        }
        let confirmed = pressed & self.confirm != 0;
        if pressed & self.exit != 0 || (confirmed && !self.can_return) {
            Some(ErrorChoice::Exit)
        } else if confirmed {
            Some(ErrorChoice::Back)
        } else {
            None
        }
    }
}

/// `game` names the failed game, when one was running.
pub fn draw(
    console: &Console,
    font: &FontVec,
    game: Option<&str>,
    message: &str,
    log: &str,
    can_return: bool,
) -> RgbaImage {
    let mut canvas = Canvas::new(console.screen);
    let width = canvas.width() - 2.0 * MARGIN as f32;
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
    let labels = console.labels;
    let hint = if can_return {
        format!("{}: Back to list    {}: Exit", labels.confirm, labels.exit)
    } else {
        format!("{} / {}: Exit", labels.confirm, labels.exit)
    };
    canvas.text(font, 24.0, [MARGIN as f32, 692.0], width, &hint, DIM);
    canvas.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_pad::{
        button::{A, PLUS},
        console,
    };
    fn sample(buttons: u32) -> PadInput {
        PadInput {
            focused: true,
            buttons,
            ..PadInput::default()
        }
    }
    #[test]
    fn held_buttons_are_ignored_and_a_returns_only_with_a_list() {
        let mut prompt = ErrorPrompt::new(&sample(A), true, &console([1280, 720]).buttons);
        assert_eq!(prompt.poll(&sample(A)), None);
        assert_eq!(prompt.poll(&sample(0)), None);
        assert_eq!(prompt.poll(&sample(A)), Some(ErrorChoice::Back));
        assert_eq!(prompt.poll(&sample(PLUS)), Some(ErrorChoice::Exit));

        let mut single = ErrorPrompt::new(&sample(0), false, &console([1280, 720]).buttons);
        let unfocused = PadInput {
            focused: false,
            ..sample(A)
        };
        assert_eq!(single.poll(&unfocused), None);
        assert_eq!(single.poll(&sample(A)), None);
        assert_eq!(single.poll(&sample(0)), None);
        assert_eq!(single.poll(&sample(A)), Some(ErrorChoice::Exit));
    }
}

/// Writes an error page preview: `CMVS_MENU_FONT=<font> CMVS_ERROR_PNG=<out.png> cargo
/// test -p cmvs-console error_preview -- --ignored`; `CMVS_MENU_SCREEN=vita` draws it at
/// 960x544.
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
        &crate::test_pad::console(crate::test_pad::preview_screen()),
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

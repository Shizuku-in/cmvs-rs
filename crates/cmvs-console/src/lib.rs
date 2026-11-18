//! Host policy shared by the game-console players (Switch, PlayStation Vita): the
//! controller-to-engine mapping, the game list, the error page, CPU-drawn host pages,
//! the game text encoding and its comment dialog, and GPU scene compositing with its texture
//! cache. Each console describes its screen and buttons with a [`Console`]; input
//! arrives as [`cmvs_native::PadInput`] samples.
pub mod audio;
pub mod backend;
pub mod canvas;
pub mod controls;
pub mod encoding;
pub mod error_screen;
pub mod fs;
pub use fs::ConsoleFs;
pub mod gpu;
pub mod library;
pub mod menu;
pub mod text_input;

use std::{error::Error, fmt, path::Path, time::Duration};

/// Converts a frame interval to milliseconds saturated to `u32`.
pub fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// Renders the current movie frame onto the logical canvas if playing, otherwise
/// software-composites the scene.
pub fn render_software(
    startup: &cmvs_core::runtime::Runtime,
    font: &mut cmvs_font::font::FontRenderer,
    linear: bool,
) -> Result<cmvs_core::render::raster::RgbaImage, Box<dyn Error + Send + Sync>> {
    cmvs_host::compose_cpu_frame(startup, Some(font), linear)
}

/// Binds user font directory, installs glyph metrics on the runtime, and falls back to the
/// built-in font face if the configured face is absent.
pub fn configure_game_font(
    startup: &mut cmvs_core::runtime::Runtime,
    font: &mut cmvs_font::font::FontRenderer,
    user_dir: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    cmvs_host::bind_font(startup, font, Some(user_dir)).map(|_| ())
}

/// Reads and parses `root/key.cfg` when `profile.uses_key_cfg()` is true, or returns empty
/// bindings for CVNS builds that use fixed virtual key codes.
pub fn load_key_bindings(
    root: &Path,
    profile: &cmvs_core::profile::EngineProfile,
) -> Result<cmvs_core::host::input::KeyBindings, Box<dyn Error + Send + Sync>> {
    cmvs_core::host::input::KeyBindings::load_for_profile_with_fs(&ConsoleFs, root, profile)
}

/// Formats a runtime tick failure with script name, PC, and step count context.
pub fn format_tick_error(
    startup: &cmvs_core::runtime::Runtime,
    error: impl fmt::Display,
) -> String {
    let vm = startup.machine();
    format!(
        "{error}\nat {} / 0x{:x} after {} steps",
        vm.current_name().escape_ascii(),
        vm.pc(),
        startup.steps()
    )
}

/// What each console button means to the shared pages and the in-game controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonMap {
    /// Starts a game and leaves an error page.
    pub confirm: u32,
    /// Leaves the program from a page; held in game, leaves the game.
    pub exit: u32,
    /// List navigation.
    pub up: u32,
    pub down: u32,
    pub page_up: u32,
    pub page_down: u32,
    /// In game: held buttons and the engine input codes they send.
    pub codes: Vec<(u32, usize)>,
    /// In game: buttons sending wheel notches (repeating while held).
    pub wheel_up: u32,
    pub wheel_down: u32,
}

/// Button names in the on-screen hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labels {
    pub confirm: &'static str,
    pub exit: &'static str,
    /// The page buttons together, such as "L / R".
    pub pages: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Console {
    /// Screen size in pixels; touch positions use the same space.
    pub screen: [u32; 2],
    pub buttons: ButtonMap,
    pub labels: Labels,
}

/// Switch-like buttons (A confirm, + exit, L/R pages) for the shared page tests.
#[cfg(test)]
pub(crate) mod test_pad {
    use super::{ButtonMap, Console, Labels};

    pub mod button {
        pub const PLUS: u32 = 1;
        pub const A: u32 = 2;
        pub const L: u32 = 4;
        pub const R: u32 = 8;
        pub const UP: u32 = 16;
        pub const DOWN: u32 = 32;
    }

    pub fn console(screen: [u32; 2]) -> Console {
        Console {
            screen,
            buttons: ButtonMap {
                confirm: button::A,
                exit: button::PLUS,
                up: button::UP,
                down: button::DOWN,
                page_up: button::L,
                page_down: button::R,
                codes: Vec::new(),
                wheel_up: 0,
                wheel_down: 0,
            },
            labels: Labels {
                confirm: "A",
                exit: "+",
                pages: "L / R",
            },
        }
    }

    /// `CMVS_MENU_SCREEN=vita` previews pages at 960x544 instead of 1280x720.
    pub fn preview_screen() -> [u32; 2] {
        match std::env::var("CMVS_MENU_SCREEN").as_deref() {
            Ok("vita") => [960, 544],
            _ => [1280, 720],
        }
    }
}

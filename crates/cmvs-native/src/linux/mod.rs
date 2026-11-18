//! Linux system services: Fontconfig discovery and Zenity selection.
mod font_dialog;
mod text_dialog;
pub use text_dialog::edit_text;
mod fontconfig;

pub use font_dialog::choose_font;
pub use fontconfig::font_directories;

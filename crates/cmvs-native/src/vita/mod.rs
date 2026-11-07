//! PlayStation Vita integration over platforms/psvita/runtime (vita2d, SceCtrl,
//! SceTouch, SceAudioOut, common dialogs). Button bits are portable; every system call
//! is compiled for the Vita only.
#[cfg(target_os = "vita")]
mod audio;
#[cfg(target_os = "vita")]
mod dialog;
mod input;
#[cfg(target_os = "vita")]
mod system;
#[cfg_attr(not(target_os = "vita"), allow(dead_code))]
mod text;
#[cfg(target_os = "vita")]
mod window;

#[cfg(target_os = "vita")]
pub use audio::VitaAudioOut;
#[cfg(target_os = "vita")]
pub use dialog::{ime_dialog, show_message};
pub use input::button;
#[cfg(target_os = "vita")]
pub use system::{prefer_core, set_max_clocks};
#[cfg(target_os = "vita")]
pub use window::{VitaWindow, confirm_is_cross};

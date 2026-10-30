//! Windows system dialogs used by the desktop host. Other platforms reserve their own
//! implementations; this crate compiles to nothing there.
#[cfg(windows)]
pub mod font_dialog;

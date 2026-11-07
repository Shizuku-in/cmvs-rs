//! Portable controller samples shared by the console hosts, so their input policy can
//! be tested off-device. Button bits are per console: see `switch_button` and
//! `vita_button`.

/// One touch-screen contact in screen pixels; `id` is stable while held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    pub id: u32,
    pub position: [u32; 2],
}

/// One sample of system state and input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PadInput {
    /// The system asked the application to exit.
    pub exit: bool,
    /// The application is in the foreground; otherwise system UI covers it.
    pub focused: bool,
    /// Held buttons, as the console's button bits.
    pub buttons: u32,
    /// Left and right stick positions, each axis -32767..=32767 with +y up.
    pub sticks: [[i32; 2]; 2],
    pub touches: Vec<Touch>,
}

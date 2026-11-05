//! Portable controller samples, so hosts can test their input policy off-device.

/// Held buttons of the combined standard controller (D-pad only, not the sticks).
pub mod button {
    pub const PLUS: u32 = 1 << 0;
    pub const A: u32 = 1 << 1;
    pub const B: u32 = 1 << 2;
    pub const X: u32 = 1 << 3;
    pub const Y: u32 = 1 << 4;
    pub const L: u32 = 1 << 5;
    pub const R: u32 = 1 << 6;
    pub const ZL: u32 = 1 << 7;
    pub const ZR: u32 = 1 << 8;
    pub const MINUS: u32 = 1 << 9;
    pub const UP: u32 = 1 << 10;
    pub const DOWN: u32 = 1 << 11;
    pub const LEFT: u32 = 1 << 12;
    pub const RIGHT: u32 = 1 << 13;
    pub const STICK_L: u32 = 1 << 14;
    pub const STICK_R: u32 = 1 << 15;
}

/// One touch-screen contact in 1280x720 panel pixels; `id` is stable while held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    pub id: u32,
    pub position: [u32; 2],
}

/// One sample of applet state and input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SwitchInput {
    /// The system asked the application to exit (HOME close or applet shutdown).
    pub exit: bool,
    /// The application is in the foreground; otherwise HOME or an overlay is shown.
    pub focused: bool,
    /// Bits from [`button`].
    pub buttons: u32,
    /// Left and right stick positions, each axis -32767..=32767 with +y up.
    pub sticks: [[i32; 2]; 2],
    pub touches: Vec<Touch>,
}

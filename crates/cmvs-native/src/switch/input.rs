//! Switch button bits in [`crate::PadInput::buttons`].

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

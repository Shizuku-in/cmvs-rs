//! Vita button bits in [`crate::PadInput::buttons`].

/// Held buttons (D-pad only, not the sticks). L and R are the shoulder buttons.
pub mod button {
    pub const CROSS: u32 = 1 << 0;
    pub const CIRCLE: u32 = 1 << 1;
    pub const SQUARE: u32 = 1 << 2;
    pub const TRIANGLE: u32 = 1 << 3;
    pub const L: u32 = 1 << 4;
    pub const R: u32 = 1 << 5;
    pub const SELECT: u32 = 1 << 6;
    pub const START: u32 = 1 << 7;
    pub const UP: u32 = 1 << 8;
    pub const DOWN: u32 = 1 << 9;
    pub const LEFT: u32 = 1 << 10;
    pub const RIGHT: u32 = 1 << 11;
}

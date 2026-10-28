//! Draw-state frame animation; addresses refer to the documented binary hash.
/// Frame entries in the 0x510-byte animation object (0x140026060 zeroes them all).
pub const ANIMATION_FRAMES: usize = 64;

/// One 20-byte entry at animation +16 + 20 * index, written by 0x1400261b0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationFrame {
    /// Entry +0: the frame 0x1400262b0 moves to next; negative ends the animation and -2
    /// also marks the object for disposal (0x14003d0c0 sets +14500).
    pub next: i32,
    /// Entry +4..+14: source x, y, width, height and origin x, y copied into draw words 2..8
    /// (origin only when non-negative).
    pub words: [i16; 6],
    /// Entry +16: milliseconds before moving on, compared unsigned with the elapsed time.
    pub duration: u32,
}

/// The draw state +136 frame animation (0x21cf). Header words: +0 playing, +4 start frame,
/// +8 current frame, +12 elapsed milliseconds; all zero after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub playing: bool,
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
    pub frames: [AnimationFrame; ANIMATION_FRAMES],
}
impl Animation {
    /// 0x140026200 (0x21d1): play from `frame`, copying it into the draw words.
    pub(crate) fn start(&mut self, frame: usize, words: &mut [u32; 22]) {
        self.playing = true;
        self.start = frame as u32;
        self.current = frame as u32;
        self.elapsed = 0;
        self.frames[frame].apply(words);
    }
    /// 0x1400262b0 for a playing animation: add the frame delta (at most 200 ms), then
    /// while the current frame's duration has elapsed either end with its negative `next`
    /// or move to `next`, subtracting the duration and copying the frame into the draw
    /// words. A cycle of zero-length frames never exits in the original; Rust moves at most
    /// ANIMATION_FRAMES times per update.
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        if !self.playing {
            return None;
        }
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        for _ in 0..ANIMATION_FRAMES {
            let frame = self.frames[self.current as usize];
            if frame.duration > self.elapsed {
                return None;
            }
            if frame.next < 0 {
                return Some(frame.next);
            }
            self.elapsed -= frame.duration;
            self.current = frame.next as u32;
            self.frames[self.current as usize].apply(words);
        }
        None
    }
}
impl AnimationFrame {
    /// Sign-extended source rectangle into draw words 2..6; origin words 6/7 only when the
    /// stored value is non-negative.
    fn apply(&self, words: &mut [u32; 22]) {
        for (word, value) in words[2..6].iter_mut().zip(&self.words[..4]) {
            *word = i32::from(*value) as u32;
        }
        for (word, value) in words[6..8].iter_mut().zip(&self.words[4..]) {
            if *value >= 0 {
                *word = i32::from(*value) as u32;
            }
        }
    }
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            playing: false,
            start: 0,
            current: 0,
            elapsed: 0,
            frames: [AnimationFrame::default(); ANIMATION_FRAMES],
        }
    }
}

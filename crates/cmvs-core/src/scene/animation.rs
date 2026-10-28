//! Draw-state frame animation, shared by window and scene commands; addresses are hash-bound.
use super::SceneHost;
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};
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

/// The draw state +136 frame animation (0x21cf/0x21ef). Header words: +0 playing, +4 start frame,
/// +8 current frame, +12 elapsed milliseconds. Only playing/entries are initialized by
/// the original constructor. Inactive header placeholders are never read before start().
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
    /// 0x140026110: refresh the current frame, including the non-negative origin rule.
    fn stop(&mut self, words: &mut [u32; 22]) {
        if self.playing {
            self.playing = false;
            self.frames[self.current as usize].apply(words);
        }
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

impl SceneHost {
    pub(super) fn try_frame_animation(
        &mut self,
        vm: &mut Machine<'_>,
    ) -> Result<bool, DecodeError> {
        let Some((pc, command @ 0x1ef..=0x1f3)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x1f0 => 11,
            0x1f1 => 3,
            _ => 2,
        };
        let mut args = [0; 11];
        for (i, a) in args.iter_mut().take(count).enumerate() {
            *a = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let slot = u8::try_from(args[0])
            .ok()
            .filter(|s| self.objects.contains_key(s))
            .ok_or_else(|| fail("frame animation needs an existing parent in slots 0..255"))?;
        let parent = &self.objects[&slot];
        let child = if (args[1] as i32) < 0 {
            None
        } else {
            Some(
                u16::try_from(args[1])
                    .ok()
                    .filter(|id| *id < 768 && parent.children.contains_key(id))
                    .ok_or_else(|| fail("frame animation child outside 0..767 or missing"))?,
            )
        };
        let object = child.map_or(parent, |id| &parent.children[&id]);
        let draw = object
            .draw
            .as_ref()
            .ok_or_else(|| fail("frame animation target has no draw state"))?;
        if draw.animation.is_some() {
            if matches!(command, 0x1f0 | 0x1f1) && args[2] >= ANIMATION_FRAMES as u32 {
                return Err(fail("animation frame outside 0..63"));
            }
            if command == 0x1f0 && (args[3] as i32) >= ANIMATION_FRAMES as i32 {
                return Err(fail("next animation frame outside 0..63"));
            }
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let parent = self.objects.get_mut(&slot).expect("validated parent");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("validated child"),
            None => parent,
        };
        let draw = object.draw.as_mut().expect("validated draw");
        match command {
            0x1ef => draw.animation = Some(Box::default()),
            0x1f0 => {
                if let Some(a) = draw.animation.as_mut() {
                    a.frames[args[2] as usize] = AnimationFrame {
                        next: args[3] as i32,
                        duration: args[4],
                        words: std::array::from_fn(|i| args[5 + i] as i16),
                    };
                }
            }
            0x1f1 => {
                if let Some(a) = draw.animation.as_mut() {
                    a.start(args[2] as usize, &mut draw.words);
                }
            }
            0x1f2 => {
                if let Some(mut a) = draw.animation.take() {
                    a.stop(&mut draw.words);
                }
            }
            _ => {
                let value = draw.animation.as_ref().map_or(u32::MAX, |a| {
                    if a.playing { a.current } else { u32::MAX - 1 }
                });
                vm.state_mut()
                    .set_system_integer(0, value)
                    .expect("result exists");
            }
        }
        Ok(true)
    }
}

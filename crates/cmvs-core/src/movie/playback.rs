//! 0x2309 -> 0x1400385a0, mode 0 update 0x140039330; hash-bound in vm-semantics.md.
use super::{MOVIE_SLOTS, MovieHost};
use crate::{FormatError, evaluate::error, machine::Machine, scene::SceneHost, vm::SystemResult};
use std::error::Error;
pub const MOVIE_SEGMENTS: usize = 16;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MovieSegment {
    pub looping: bool,
    pub start: u32,
    pub end: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoviePlayback {
    pub segments: [MovieSegment; MOVIE_SEGMENTS],
    pub current: usize,
    pub pending: Option<usize>,
    pub active: bool,
    pub paused: bool,
    pub elapsed: u32,
    pub last_tick: u32,
}
impl MoviePlayback {
    pub(super) fn new(frame: u32, end: u32) -> Self {
        let mut segments = [MovieSegment::default(); MOVIE_SEGMENTS];
        segments[0] = MovieSegment {
            looping: false,
            start: frame,
            end,
        };
        Self {
            segments,
            current: 0,
            pending: None,
            active: false,
            paused: false,
            elapsed: 0,
            last_tick: 0,
        }
    }
    fn advance(&mut self, now: u32, frame: u32, rate: u32, time: u32) -> Result<u32, FormatError> {
        if !self.active || self.paused {
            return Ok(frame);
        }
        self.elapsed = self
            .elapsed
            .wrapping_add(now.wrapping_sub(self.last_tick).min(1000));
        self.last_tick = now;
        let steps = self.elapsed.wrapping_mul(rate) / 1000 / time;
        if steps > 65536 {
            return Err(FormatError("scene movie frame steps exceed research limit"));
        }
        self.elapsed = self
            .elapsed
            .wrapping_sub(1000u32.wrapping_mul(steps).wrapping_mul(time) / rate);
        let mut target = frame;
        for _ in 0..steps {
            let key = self.segments[self.current];
            if target < key.end {
                target = target.wrapping_add(1);
            } else if let Some(next) = self.pending.take() {
                self.current = next;
                let start = self.segments[next].start;
                target = if start == u32::MAX || start == target {
                    target.wrapping_add(1)
                } else {
                    start
                };
            } else if key.looping {
                target = key.start;
            } else {
                self.active = false;
                self.paused = true;
                break;
            }
        }
        Ok(target)
    }
}
impl MovieHost {
    /// SP-4 player, SP-8 key, SP-12 start, SP-16 end (-1 = last), SP-20 loop.
    /// Empty player: pop 20 bytes without reading loop. No result writes or immediate seek.
    pub fn try_start_scene(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x309)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, 0x2309, why);
        let arg = |at| vm.stack().peek_u32(at).map_err(|e| fail(e.0));
        let (slot, key, start, end) = (arg(4)?, arg(8)?, arg(12)?, arg(16)?);
        if slot as usize >= MOVIE_SLOTS {
            return Err(fail("movie slot outside 0..2 sets an unmodeled flag").into());
        }
        let Some(movie) = &self.slots[slot as usize] else {
            vm.complete_system(SystemResult(0x4014))?;
            return Ok(true);
        };
        if key as usize >= MOVIE_SEGMENTS {
            return Err(fail("movie segment outside 0..15").into());
        }
        let segment = MovieSegment {
            looping: arg(20)? != 0,
            start,
            end: if end == u32::MAX {
                movie.index.max_sequence() - 1
            } else {
                end
            },
        };
        vm.complete_system(SystemResult(0x4014))?;
        let p = &mut self.slots[slot as usize]
            .as_mut()
            .expect("validated")
            .playback;
        p.segments[key as usize] = segment;
        if p.active {
            p.pending = Some(key as usize);
        } else {
            p.current = key as usize;
            p.pending = None;
            p.active = true;
            p.elapsed = 0;
            p.last_tick = self.scene_clock;
        }
        Ok(true)
    }
    /// Main loop 0x14000e400 updates slots before VM/scene effects. Decode the final
    /// target only; each player's failed update retains its time/frame/image for retry.
    pub fn advance_scene(
        &mut self,
        delta: u32,
        scene: &mut SceneHost,
    ) -> Result<(), Box<dyn Error>> {
        self.scene_clock = self.scene_clock.wrapping_add(delta);
        for movie in self.slots.iter_mut().flatten() {
            let mut playback = movie.playback.clone();
            let target = playback.advance(
                self.scene_clock,
                movie.frame,
                movie.index.rate(),
                movie.index.rate_time(),
            )?;
            if target != movie.frame {
                if scene
                    .object(movie.object_slot)
                    .is_none_or(|o| o.image.as_ref().is_none_or(|i| i.name != movie.name))
                {
                    return Err("scene movie bound object was destroyed or replaced".into());
                }
                let pixels = movie
                    .index
                    .decode(&movie.data, target, movie.base.as_ref())?;
                scene
                    .object_mut(movie.object_slot)
                    .expect("validated")
                    .image
                    .as_mut()
                    .expect("validated")
                    .pixels = pixels;
            }
            movie.frame = target;
            movie.playback = playback;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_clock_wrap_clamp_and_fractional_remainder_match_unsigned_arithmetic() {
        let mut p = MoviePlayback::new(0, 100);
        p.active = true;
        p.last_tick = u32::MAX - 15;
        assert_eq!(p.advance(51, 0, 15, 1).unwrap(), 1);
        assert_eq!(p.elapsed, 1);
        assert_eq!(p.advance(2051, 1, 15, 1).unwrap(), 16);
        assert_eq!(p.elapsed, 1);
    }
    #[test]
    fn queued_minus_one_and_equal_start_both_continue_with_the_next_frame() {
        for start in [u32::MAX, 3] {
            let mut p = MoviePlayback::new(0, 3);
            p.active = true;
            p.pending = Some(1);
            p.segments[1] = MovieSegment {
                looping: false,
                start,
                end: 7,
            };
            assert_eq!(p.advance(67, 3, 15, 1).unwrap(), 4);
            assert_eq!(p.current, 1);
            assert_eq!(p.pending, None);
        }
    }
}

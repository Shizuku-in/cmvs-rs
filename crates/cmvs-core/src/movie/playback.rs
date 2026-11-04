//! 0x2309 -> 0x1400385a0, mode 0 update 0x140039330; hash-bound in vm-semantics.md.
use super::{MOVIE_SLOTS, MovieHost};
use crate::{
    FormatError, bytecode::DecodeError, dispatch::Call, evaluate::error, machine::Machine,
    scene::SceneHost,
};
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
    pub mode: u32,
    pub stop_frame: u32,
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
            mode: 0,
            stop_frame: 0,
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
        self.elapsed = self.elapsed.wrapping_add(if self.mode == 10 {
            now.wrapping_sub(self.last_tick)
        } else {
            now.wrapping_sub(self.last_tick).min(1000)
        });
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
                if self.mode != 10 || self.stop_frame == 0 || target != self.stop_frame {
                    target = target.wrapping_add(1);
                }
            } else if self.mode == 10 {
                self.active = false;
                self.paused = true;
                break;
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
    /// HAROUSE 0x450830/0x4508b0: mode setter and raw segment/frame query.
    pub(crate) fn legacy_control(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let slot = call.argument(4, "player slot")? as usize;
        if slot >= MOVIE_SLOTS {
            return Err(call.error("movie slot outside 0..2 sets an unmodeled flag"));
        }
        if matches!(call.command, 0x30d | 0x31d) {
            if self.slots[slot].is_some() {
                let mode = call.argument(
                    8,
                    if call.command == 0x30d {
                        "playback mode"
                    } else {
                        "stop frame"
                    },
                )?;
                if call.command == 0x30d && !matches!(mode, 0 | 10) {
                    return Err(call.error("movie playback mode is not implemented"));
                }
                call.complete(0x4008)?;
                let playback = &mut self.slots[slot].as_mut().expect("validated").playback;
                if call.command == 0x30d {
                    playback.mode = mode;
                } else {
                    playback.stop_frame = mode;
                }
            } else {
                call.ignore_argument(8, "unused playback mode")?;
                call.complete(0x4008)?;
            }
        } else {
            let (segment, frame) = self.slots[slot].as_ref().map_or((u32::MAX, 0), |movie| {
                (movie.playback.current as u32, movie.frame)
            });
            call.complete(0x4004)?;
            let state = call.vm.state_mut();
            state.set_system_integer(0, segment).expect("slot exists");
            state.set_system_integer(4, frame).expect("slot exists");
        }
        Ok(())
    }

    /// HAROUSE 0x4509f0/0x450a70: seek and clear pause, or apply a queued segment.
    pub(crate) fn seek_scene(
        &mut self,
        call: &mut Call<'_, '_>,
        scene: &mut SceneHost,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "player slot")? as usize;
        let requested = if call.command == 0x31b {
            Some(call.argument(8, "target frame")?)
        } else {
            None
        };
        let consumed = if requested.is_some() { 0x4008 } else { 0x4004 };
        if slot >= MOVIE_SLOTS {
            return Err(call
                .error("movie slot outside 0..2 sets an unmodeled flag")
                .into());
        }
        let Some(movie) = self.slots[slot].as_ref() else {
            call.complete(consumed)?;
            call.vm
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot exists");
            return Ok(());
        };
        let pending = if requested.is_none() {
            movie.playback.pending
        } else {
            None
        };
        let target = match (requested, pending) {
            (Some(target), _) => target,
            (None, Some(key)) => match movie.playback.segments[key].start {
                u32::MAX => movie.frame.wrapping_add(1),
                start => start,
            },
            (None, None) => {
                call.complete(consumed)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 0)
                    .expect("slot exists");
                return Ok(());
            }
        };
        let pixels = if target != movie.frame {
            if target >= movie.index.max_sequence() {
                return Err(call
                    .error("out-of-range movie seek is not implemented")
                    .into());
            }
            if scene
                .object(movie.object_slot)
                .is_none_or(|o| o.image.as_ref().is_none_or(|i| i.name != movie.name))
            {
                return Err(call
                    .error("scene movie bound object was destroyed or replaced")
                    .into());
            }
            Some(
                movie
                    .index
                    .decode(&movie.data, target, movie.base.as_ref())?,
            )
        } else {
            None
        };
        call.complete(consumed)?;
        let movie = self.slots[slot].as_mut().expect("validated");
        if let Some(pixels) = pixels {
            scene
                .object_mut(movie.object_slot)
                .expect("validated")
                .image
                .as_mut()
                .expect("validated")
                .pixels = pixels;
        }
        movie.frame = target;
        if let Some(key) = pending {
            movie.playback.current = key;
            movie.playback.pending = None;
        } else {
            movie.playback.paused = false;
        }
        call.vm
            .state_mut()
            .set_system_integer(0, 1)
            .expect("slot exists");
        Ok(())
    }

    /// SP-4 player, SP-8 key, SP-12 start, SP-16 end (-1 = last), SP-20 loop.
    /// Empty player: pop 20 bytes without reading loop. No result writes or immediate seek.
    pub fn try_start_scene(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x309)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle_start_scene(&mut Call::new(vm, pc, 0x309))?;
        Ok(true)
    }
    pub(crate) fn handle_start_scene(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<(), Box<dyn Error>> {
        let pc = call.pc;
        let fail = |why| error(pc, 0x2309, why);
        let (slot, key, start, end) = (
            call.argument(4, "player slot")?,
            call.argument(8, "segment")?,
            call.argument(12, "start frame")?,
            call.argument(16, "end frame")?,
        );
        if slot as usize >= MOVIE_SLOTS {
            return Err(fail("movie slot outside 0..2 sets an unmodeled flag").into());
        }
        let Some(movie) = &self.slots[slot as usize] else {
            call.ignore_argument(20, "unused loop")?;
            call.complete(0x4014)?;
            return Ok(());
        };
        if key as usize >= MOVIE_SEGMENTS {
            return Err(fail("movie segment outside 0..15").into());
        }
        let segment = MovieSegment {
            looping: call.argument(20, "loop")? != 0,
            start,
            end: if end == u32::MAX {
                movie.index.max_sequence() - 1
            } else {
                end
            },
        };
        call.complete(0x4014)?;
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
        Ok(())
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
    fn mode_ten_uses_unclamped_time_and_stops_without_loop_or_pending_transition() {
        let mut p = MoviePlayback::new(0, 30);
        p.mode = 10;
        p.active = true;
        p.pending = Some(1);
        p.segments[0].looping = true;
        assert_eq!(p.advance(1500, 0, 15, 1).unwrap(), 22);
        assert_eq!(p.elapsed, 34);
        assert_eq!(p.advance(2100, 22, 15, 1).unwrap(), 30);
        assert!(!p.active);
        assert!(p.paused);
        assert_eq!(p.pending, Some(1));
        assert_eq!(p.current, 0);
        let old = p.clone();
        assert_eq!(p.advance(5000, 30, 15, 1).unwrap(), 30);
        assert_eq!(p, old);
    }
    #[test]
    fn mode_ten_holds_at_stop_frame_until_cleared_without_pausing() {
        let mut p = MoviePlayback::new(0, 30);
        p.mode = 10;
        p.stop_frame = 2;
        p.active = true;
        assert_eq!(p.advance(1000, 0, 15, 1).unwrap(), 2);
        assert!(p.active);
        assert!(!p.paused);
        assert_eq!(p.advance(2000, 2, 15, 1).unwrap(), 2);
        p.stop_frame = 0;
        assert_eq!(p.advance(2067, 2, 15, 1).unwrap(), 3);
    }
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

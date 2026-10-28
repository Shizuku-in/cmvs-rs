//! Draw-state +112 scale track, 0x21e0..0x21e4; addresses are hash-bound in vm-semantics.
use super::SceneHost;
use crate::{
    bytecode::DecodeError,
    evaluate::{error, truncate},
    machine::Machine,
    vm::SystemResult,
};

pub const SCALE_KEYS: usize = 8;
/// 20-byte record at +24: next, target X/Y float bits, mode, unsigned duration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScaleKey {
    pub next: i32,
    pub xy: [u32; 2],
    pub mode: u32,
    pub duration: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalePlayback {
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
}
/// 0xb8-byte object: constructor initializes base and records, start initializes playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleMotion {
    pub base: [u32; 2],
    pub playback: Option<ScalePlayback>,
    pub keys: [ScaleKey; SCALE_KEYS],
}
impl ScaleMotion {
    /// 0x140024c70: linear f32 interpolation or quantized quarter-sine mode 1.
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        let play = self.playback.as_mut()?;
        play.elapsed = play.elapsed.wrapping_add(delta.min(200));
        let key = self.keys[play.current as usize];
        if key.duration > play.elapsed {
            let elapsed = play.elapsed as f32;
            let duration = key.duration as f32;
            for (i, word) in words[18..20].iter_mut().enumerate() {
                let base = f32::from_bits(self.base[i]);
                let difference = f32::from_bits(key.xy[i]) - base;
                let offset = if key.mode as u8 == 1 {
                    let degrees = (elapsed * 90.0) / duration;
                    let radians = (degrees / 180.0) * std::f32::consts::PI;
                    let value = (f64::from(radians).sin() * f64::from(difference)) as f32;
                    truncate(value * 100.0) as f32 / 100.0
                } else {
                    (difference * elapsed) / duration
                };
                *word = (offset + base).to_bits();
            }
            return None;
        }
        // The original can loop forever through zero-duration cycles; use the existing cap.
        for _ in 0..super::ANIMATION_FRAMES {
            let key = self.keys[play.current as usize];
            self.base = key.xy;
            words[18..20].copy_from_slice(&key.xy);
            if key.next < 0 {
                return Some(key.next);
            }
            play.elapsed -= key.duration;
            play.current = key.next as u32;
            if self.keys[play.current as usize].duration > play.elapsed {
                return None;
            }
        }
        None
    }
}
impl SceneHost {
    pub(super) fn try_scale_motion(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ 0x1e0..=0x1e4)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x1e0 => 4,
            0x1e1 => 8,
            0x1e2 => 3,
            _ => 2,
        };
        let mut args = [0; 8];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let slot = u8::try_from(args[0])
            .ok()
            .filter(|s| self.objects.contains_key(s))
            .ok_or_else(|| fail("scale motion needs an existing parent in slots 0..255"))?;
        let parent = &self.objects[&slot];
        let child = if (args[1] as i32) < 0 {
            None
        } else {
            Some(
                u16::try_from(args[1])
                    .ok()
                    .filter(|id| *id < 768 && parent.children.contains_key(id))
                    .ok_or_else(|| fail("scale motion child outside 0..767 or missing"))?,
            )
        };
        let object = child.map_or(parent, |id| &parent.children[&id]);
        let draw = object
            .draw
            .as_ref()
            .ok_or_else(|| fail("scale motion target has no draw state"))?;
        if draw.scale_motion.is_some() {
            if matches!(command, 0x1e1 | 0x1e2) && args[2] >= SCALE_KEYS as u32 {
                return Err(fail("scale key outside 0..7"));
            }
            if command == 0x1e1 {
                if (args[3] as i32) >= SCALE_KEYS as i32 {
                    return Err(fail("next scale key outside 0..7"));
                }
                if !matches!(args[4] as u8, 0 | 1) {
                    return Err(fail("scale motion mode not implemented"));
                }
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
            0x1e0 => {
                draw.scale_motion = Some(Box::new(ScaleMotion {
                    base: [args[2], args[3]],
                    playback: None,
                    keys: [ScaleKey::default(); SCALE_KEYS],
                }))
            }
            0x1e1 => {
                if let Some(m) = draw.scale_motion.as_mut() {
                    m.keys[args[2] as usize] = ScaleKey {
                        next: args[3] as i32,
                        mode: args[4],
                        duration: args[5],
                        xy: [args[6], args[7]],
                    };
                }
            }
            0x1e2 => {
                if let Some(m) = draw.scale_motion.as_mut() {
                    m.playback = Some(ScalePlayback {
                        start: args[2],
                        current: args[2],
                        elapsed: 0,
                    });
                }
            }
            0x1e3 => {
                if let Some(m) = draw.scale_motion.take()
                    && let Some(play) = m.playback
                {
                    draw.words[18..20].copy_from_slice(&m.keys[play.current as usize].xy);
                }
            }
            _ => {
                let result = draw.scale_motion.as_ref().map_or(u32::MAX, |m| {
                    m.playback.as_ref().map_or(u32::MAX - 1, |p| p.current)
                });
                vm.state_mut()
                    .set_system_integer(0, result)
                    .expect("result exists");
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsigned_duration_elapsed_wrap_and_zero_duration_cycle_are_bounded() {
        let mut motion = ScaleMotion {
            base: [0; 2],
            playback: Some(ScalePlayback {
                start: 0,
                current: 0,
                elapsed: u32::MAX - 4,
            }),
            keys: [ScaleKey {
                next: 0,
                xy: [1.0f32.to_bits(); 2],
                mode: 0,
                duration: u32::MAX,
            }; 8],
        };
        let mut words = [0; 22];
        assert_eq!(motion.advance(10, &mut words), None);
        assert_eq!(motion.playback.as_ref().unwrap().elapsed, 5);
        assert_eq!(words[18], (5.0f32 / u32::MAX as f32).to_bits());
        motion.keys[0].duration = 0;
        assert_eq!(motion.advance(10, &mut words), None);
        assert_eq!(&words[18..20], &[1.0f32.to_bits(); 2]);
    }
}

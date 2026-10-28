//! Draw state +152, 0x21f9..0x21fd; addresses refer to the documented binary hash.
//! Linear mode 0, absolute quadratic ease-out mode 1 and periodic Y modes 0x38/0x39.
use super::SceneHost;
use crate::{
    bytecode::DecodeError,
    evaluate::{error, truncate},
    machine::Machine,
    vm::SystemResult,
};

pub const WORLD_MOTION_KEYS: usize = 8;

/// 24-byte entry at +28 + 24 * index, written by 0x1400254e0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotionKey {
    pub next: i32,
    /// Target XYZ as original f32 bits. Relative mode still snaps to these on completion.
    pub xyz: [u32; 3],
    /// Low byte selects interpolation; bit 0x8000 selects relative linear interpolation.
    pub mode: u32,
    pub duration: u32,
}

/// Header +4..+24, initialized only by 0x140025530; absent after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionPlayback {
    pub base: [u32; 3],
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
}

/// 0xdc-byte track, constructor 0x1400253e0 clears playing and eight entries only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldMotion {
    pub playback: Option<MotionPlayback>,
    pub keys: [MotionKey; WORLD_MOTION_KEYS],
}

impl WorldMotion {
    pub(crate) fn supports_mode(mode: u32) -> bool {
        matches!(mode as u8, 0 | 0x38 | 0x39) || mode as u8 == 1 && mode & 0x8000 == 0
    }

    /// 0x140025570 after opacity/frame animation: clamp delta to 200 ms, wrap elapsed,
    /// interpolate only before the first boundary, then snap while crossing entries.
    /// A new segment with remaining time is not interpolated until the following frame.
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        let playback = self.playback.as_mut()?;
        playback.elapsed = playback.elapsed.wrapping_add(delta.min(200));
        let key = self.keys[playback.current as usize];
        if key.duration > playback.elapsed {
            // cvtsi2ss uses zero-extended rdi/r8, not the decompiler's signed int casts.
            let elapsed = playback.elapsed as f32;
            let duration = key.duration as f32;
            if matches!(key.mode as u8, 0x38 | 0x39) {
                // 0x140025bac -> 0x140025b56: unsigned wrapping product and remainder,
                // f32 degrees/amplitude; 0x140025310 uses f64 sin then f32 hundredths.
                let phase = playback.elapsed.wrapping_mul(key.mode >> 28) % key.duration;
                let degrees = (phase as f32 * 180.0) / duration;
                let radians = (degrees / 180.0) * std::f32::consts::PI;
                let amplitude = ((key.mode >> 16) & 0xfff) as f32 / 100.0;
                let value = (f64::from(radians).sin() * f64::from(amplitude)) as f32;
                let offset = truncate(value * 100.0) as f32 / 100.0;
                words[11] = playback.base[0];
                let base_y = f32::from_bits(playback.base[1]);
                words[12] = if key.mode as u8 == 0x38 {
                    (base_y - offset).to_bits()
                } else {
                    // 0x140025bcd -> 0x140025ad6 -> 0x140025b0b.
                    (offset + base_y).to_bits()
                };
                words[13] = playback.base[2];
                return None;
            }
            for (i, word) in words[11..14].iter_mut().enumerate() {
                let base = f32::from_bits(playback.base[i]);
                let target = f32::from_bits(key.xyz[i]);
                if key.mode as u8 == 1 {
                    // 0x140025810: f32 subtraction, f64 easing/product, cvttsd2si,
                    // then f32 integer conversion/division/subtraction.
                    let remaining = 1.0 - f64::from(elapsed) / f64::from(duration);
                    let value = (f64::from(target - base) * (remaining * remaining)) * 100.0;
                    let integer = if value.is_nan()
                        || !(-2_147_483_648.0..2_147_483_648.0).contains(&value)
                    {
                        i32::MIN
                    } else {
                        value as i32
                    };
                    *word = (target - (integer as f32 / 100.0)).to_bits();
                    continue;
                }
                let step = if key.mode & 0x8000 != 0 {
                    (elapsed * target) / duration
                } else {
                    ((target - base) * elapsed) / duration
                };
                *word = ((truncate(step * 100.0) as f32 / 100.0) + base).to_bits();
            }
            return None;
        }
        // Same bounded-loop policy as frame animation; the original can spin forever.
        for _ in 0..super::ANIMATION_FRAMES {
            let key = self.keys[playback.current as usize];
            playback.base = key.xyz;
            words[11..14].copy_from_slice(&key.xyz);
            if key.next < 0 {
                return Some(key.next);
            }
            playback.elapsed -= key.duration;
            playback.current = key.next as u32;
            if self.keys[playback.current as usize].duration > playback.elapsed {
                return None;
            }
        }
        None
    }
}

impl SceneHost {
    pub(super) fn try_world_motion(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ 0x1f9..=0x1fd)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x1fa => 9,
            0x1fb => 3,
            _ => 2,
        };
        let mut args = [0; 9];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let slot = u8::try_from(args[0])
            .ok()
            .filter(|s| self.objects.contains_key(s))
            .ok_or_else(|| fail("world motion needs an existing object in slots 0..255"))?;
        let parent = &self.objects[&slot];
        let child = if (args[1] as i32) < 0 {
            None
        } else {
            Some(
                u16::try_from(args[1])
                    .ok()
                    .filter(|id| *id < 768 && parent.children.contains_key(id))
                    .ok_or_else(|| fail("world motion child outside 0..767 or missing"))?,
            )
        };
        let object = match child {
            Some(id) => &parent.children[&id],
            None => parent,
        };
        let draw = object
            .draw
            .as_ref()
            .ok_or_else(|| fail("world motion target has no draw state"))?;
        if draw.world_motion.is_some() {
            if matches!(command, 0x1fa | 0x1fb) && args[2] >= WORLD_MOTION_KEYS as u32 {
                return Err(fail("world motion key outside 0..7"));
            }
            if command == 0x1fa {
                if (args[3] as i32) >= WORLD_MOTION_KEYS as i32 {
                    return Err(fail("world motion next key outside 0..7"));
                }
                if !WorldMotion::supports_mode(args[4]) {
                    return Err(fail("world motion interpolation mode is not implemented"));
                }
            }
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let parent = self.objects.get_mut(&slot).expect("validated object");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("validated child"),
            None => parent,
        };
        let draw = object.draw.as_mut().expect("validated draw");
        match command {
            0x1f9 => draw.world_motion = Some(Box::default()),
            0x1fa => {
                if let Some(motion) = draw.world_motion.as_mut() {
                    motion.keys[args[2] as usize] = MotionKey {
                        next: args[3] as i32,
                        xyz: [args[6], args[7], args[8]],
                        mode: args[4],
                        duration: args[5],
                    };
                }
            }
            0x1fb => {
                if let Some(motion) = draw.world_motion.as_mut() {
                    motion.playback = Some(MotionPlayback {
                        base: draw.words[11..14].try_into().expect("XYZ"),
                        start: args[2],
                        current: args[2],
                        elapsed: 0,
                    });
                }
            }
            0x1fc => {
                if let Some(motion) = draw.world_motion.take()
                    && let Some(playback) = motion.playback
                {
                    draw.words[11..14].copy_from_slice(&motion.keys[playback.current as usize].xyz);
                }
            }
            _ => {
                let result = match &draw.world_motion {
                    None => u32::MAX,
                    Some(motion) => motion.playback.as_ref().map_or(u32::MAX - 1, |p| p.current),
                };
                vm.state_mut()
                    .set_system_integer(0, result)
                    .expect("result 0 exists");
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing() -> WorldMotion {
        WorldMotion {
            playback: Some(MotionPlayback {
                base: [0; 3],
                start: 0,
                current: 0,
                elapsed: 0,
            }),
            ..WorldMotion::default()
        }
    }

    #[test]
    fn periodic_y_wraps_product_and_preserves_other_axes_as_raw_bits() {
        let mut motion = playing();
        let base = [0x7fc01234, 10f32.to_bits(), 0x80000000];
        motion.playback.as_mut().unwrap().base = base;
        motion.playback.as_mut().unwrap().elapsed = 0x20000064;
        motion.keys[0] = MotionKey {
            next: -1,
            xyz: [0; 3],
            mode: 0x80fa0038,
            duration: 3_200_000_000,
        };
        let mut words = [0; 22];
        motion.advance(0, &mut words);
        // Product wraps to 800, so the offset truncates to zero hundredths.
        assert_eq!(&words[11..14], &base);
        motion.playback.as_mut().unwrap().elapsed = 25;
        motion.keys[0].duration = 400;
        motion.advance(0, &mut words);
        assert_eq!(words[11], base[0]);
        assert_eq!(words[12], (7.5f32).to_bits());
        assert_eq!(words[13], base[2]);
        motion.playback.as_mut().unwrap().elapsed = 399;
        assert_eq!(motion.advance(1, &mut words), Some(-1));
        assert_eq!(&words[11..14], &[0; 3]);
    }

    #[test]
    fn large_unsigned_elapsed_and_duration_convert_as_zero_extended_integers() {
        let mut motion = playing();
        motion.playback.as_mut().unwrap().elapsed = 0x80000000;
        motion.keys[0] = MotionKey {
            next: -1,
            xyz: [10f32.to_bits(), (-10f32).to_bits(), 0],
            mode: 0,
            duration: u32::MAX,
        };
        let mut words = [0; 22];
        assert_eq!(motion.advance(0, &mut words), None);
        assert_eq!(
            words[11..14]
                .iter()
                .copied()
                .map(f32::from_bits)
                .collect::<Vec<_>>(),
            [5.0, -5.0, 0.0]
        );
        motion.playback.as_mut().unwrap().elapsed = u32::MAX - 10;
        motion.advance(20, &mut words);
        assert_eq!(motion.playback.as_ref().unwrap().elapsed, 9);
    }

    #[test]
    fn zero_duration_cycles_are_bounded_and_terminal_nan_bits_are_preserved() {
        let mut motion = playing();
        let mut words = [0; 22];
        assert_eq!(motion.advance(0, &mut words), None);
        assert_eq!(motion.playback.as_ref().unwrap().current, 0);
        motion.keys[0].next = -1;
        motion.keys[0].xyz = [0x7fc01234, 0x80000000, 0x7f800000];
        assert_eq!(motion.advance(0, &mut words), Some(-1));
        assert_eq!(&words[11..14], &[0x7fc01234, 0x80000000, 0x7f800000]);
        for mode in [0, 1] {
            let mut motion = playing();
            motion.keys[0] = MotionKey {
                next: -1,
                xyz: [f32::INFINITY.to_bits(); 3],
                mode,
                duration: 100,
            };
            assert_eq!(motion.advance(50, &mut words), None);
            let expected = if mode == 0 {
                (-21474836.0f32).to_bits()
            } else {
                f32::INFINITY.to_bits()
            };
            assert_eq!(&words[11..14], &[expected; 3]);
        }
    }
}

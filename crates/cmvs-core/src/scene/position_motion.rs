//! Integer XY motion at DrawState +144 (0x21f4..0x21f8), bound to the documented binary.
use super::SceneHost;
use crate::{bytecode::DecodeError, dispatch::Call, evaluate::error};
pub const POSITION_KEYS: usize = 16;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PositionKey {
    pub next: i32,
    /// Packed signed 16-bit positions; setters keep the low halves of their inputs.
    pub xy: [i16; 2],
    pub mode: u32,
    pub duration: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionPlayback {
    pub base: [i16; 2],
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
}
/// 0x140024f00 clears only the active flag and sixteen 16-byte keys. Header +4..+16
/// is unknown until play, represented by None rather than invented zero values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PositionMotion {
    pub playback: Option<PositionPlayback>,
    pub keys: [PositionKey; POSITION_KEYS],
}
impl PositionMotion {
    pub(crate) fn supported(mode: u32) -> bool {
        matches!(mode as u8, 0 | 0x20 | 0x21) || mode as u8 == 1 && mode & 0x8000 == 0
    }
    fn start(&mut self, words: &[u32; 22], key: u32) {
        self.playback = Some(PositionPlayback {
            base: [words[8] as i16, words[9] as i16],
            start: key,
            current: key,
            elapsed: 0,
        });
    }
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        let p = self.playback.as_mut()?;
        p.elapsed = p.elapsed.wrapping_add(delta.min(200));
        let k = self.keys[p.current as usize];
        if k.duration > p.elapsed {
            let base = p.base.map(i32::from);
            let target = k.xy.map(i32::from);
            let duration = k.duration as i32;
            let result = match k.mode as u8 {
                0 => std::array::from_fn(|i| {
                    let distance = if k.mode & 0x8000 != 0 {
                        target[i]
                    } else {
                        target[i] - base[i]
                    };
                    base[i]
                        .wrapping_add((p.elapsed.wrapping_mul(distance as u32) as i32) / duration)
                }),
                1 => {
                    let angle = (p.elapsed.wrapping_mul(9000) as i32) / duration;
                    std::array::from_fn(|i| base[i].wrapping_add(sine(angle, target[i] - base[i])))
                }
                mode => {
                    let angle = (p.elapsed.wrapping_mul(18000) as i32) / duration;
                    let y = sine(angle, ((k.mode >> 16) & 255) as i32);
                    let x = if mode == 0x21 {
                        sine(angle, (k.mode >> 24) as i32)
                    } else {
                        0
                    };
                    [base[0].wrapping_add(x), base[1].wrapping_add(y)]
                }
            };
            words[8..10].copy_from_slice(&result.map(|v| v as u32));
            return None;
        }
        // Bound zero-duration cycles as for the other researched effect channels.
        for _ in 0..super::ANIMATION_FRAMES {
            let k = self.keys[p.current as usize];
            p.base = k.xy;
            words[8..10].copy_from_slice(&k.xy.map(|v| i32::from(v) as u32));
            if k.next < 0 {
                return Some(k.next);
            }
            p.elapsed -= k.duration;
            p.current = k.next as u32;
            if self.keys[p.current as usize].duration > p.elapsed {
                return None;
            }
        }
        None
    }
}
/// 0x140008a30: centidegrees -> f64 radians, f64 sin/product, cvttsd2si.
fn sine(angle: i32, amplitude: i32) -> i32 {
    let radians = if angle == 0 {
        0.0
    } else {
        f64::from(angle) / 18000.0 * std::f64::consts::PI
    };
    (radians.sin() * f64::from(amplitude)) as i32
}
impl SceneHost {
    pub(super) fn handle_position_motion(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);

        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x1f5 => 8,
            0x1f6 => 3,
            _ => 2,
        };
        let mut args = [0; 8];
        for (i, a) in args.iter_mut().take(count).enumerate() {
            *a = call.argument((i + 1) * 4, super::argument_names(call.command)[i])?;
        }
        let slot = u8::try_from(args[0])
            .ok()
            .filter(|s| self.objects.contains_key(s))
            .ok_or_else(|| fail("position motion needs an existing parent in slots 0..255"))?;
        let parent = &self.objects[&slot];
        let child = if (args[1] as i32) < 0 {
            None
        } else {
            Some(
                u16::try_from(args[1])
                    .ok()
                    .filter(|id| *id < 768 && parent.children.contains_key(id))
                    .ok_or_else(|| fail("position motion child outside 0..767 or missing"))?,
            )
        };
        let object = child.map_or(parent, |id| &parent.children[&id]);
        let draw = object
            .draw
            .as_ref()
            .ok_or_else(|| fail("position motion target has no draw state"))?;
        if draw.position_motion.is_some() {
            if matches!(command, 0x1f5 | 0x1f6) && args[2] >= POSITION_KEYS as u32 {
                return Err(fail("position motion key outside 0..15"));
            }
            if command == 0x1f5 {
                if (args[3] as i32) >= POSITION_KEYS as i32 {
                    return Err(fail("position motion next key outside 0..15"));
                }
                if !PositionMotion::supported(args[4]) {
                    return Err(fail(
                        "position motion interpolation mode is not implemented",
                    ));
                }
                if args[5] > i32::MAX as u32 {
                    return Err(fail(
                        "position motion duration exceeds verified signed range",
                    ));
                }
            }
        }
        call.complete(0x4000 | (count * 4) as u16)?;
        let parent = self.objects.get_mut(&slot).expect("validated parent");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("validated child"),
            None => parent,
        };
        let draw = object.draw.as_mut().expect("validated draw");
        match command {
            0x1f4 => draw.position_motion = Some(Box::default()),
            0x1f5 => {
                if let Some(m) = draw.position_motion.as_mut() {
                    m.keys[args[2] as usize] = PositionKey {
                        next: args[3] as i32,
                        xy: [args[6] as i16, args[7] as i16],
                        mode: args[4],
                        duration: args[5],
                    };
                }
            }
            0x1f6 => {
                if let Some(m) = draw.position_motion.as_mut() {
                    m.start(&draw.words, args[2]);
                }
            }
            0x1f7 => {
                if let Some(m) = draw.position_motion.take()
                    && let Some(p) = m.playback
                {
                    draw.words[8..10].copy_from_slice(
                        &m.keys[p.current as usize].xy.map(|v| i32::from(v) as u32),
                    );
                }
            }
            _ => {
                let value = draw.position_motion.as_ref().map_or(u32::MAX, |m| {
                    m.playback.as_ref().map_or(u32::MAX - 1, |p| p.current)
                });
                call.vm
                    .state_mut()
                    .set_system_integer(0, value)
                    .expect("result exists");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_products_elapsed_and_zero_duration_cycles_follow_integer_channel_rules() {
        let mut words = [0; 22];
        words[8] = 32767;
        words[9] = (-32768i32) as u32;
        let mut m = PositionMotion::default();
        m.start(&words, 0);
        m.keys[0] = PositionKey {
            next: -1,
            xy: [-32768, 32767],
            mode: 0,
            duration: 1_000_000,
        };
        m.playback.as_mut().unwrap().elapsed = 600_000;
        assert_eq!(m.advance(0, &mut words), None);
        assert_eq!(&words[8..10], &[32101, (-32102i32) as u32]);
        m.playback.as_mut().unwrap().elapsed = u32::MAX - 5;
        m.advance(16, &mut words);
        assert_eq!(m.playback.as_ref().unwrap().elapsed, 10);
        assert_eq!(&words[8..10], &[32767, (-32768i32) as u32]);
        m.keys = [PositionKey::default(); POSITION_KEYS];
        m.start(&words, 0);
        assert_eq!(m.advance(0, &mut words), None);
        assert_eq!(m.playback.as_ref().unwrap().current, 0);
    }
}

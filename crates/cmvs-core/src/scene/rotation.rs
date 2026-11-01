//! Draw-state rotation track; addresses refer to the documented binary hash.
use super::{ANIMATION_FRAMES, SceneHost};
use crate::{bytecode::DecodeError, dispatch::Call, evaluate::error};

/// Keys in the 0x94-byte rotation track (0x1400263d0 zeroes them all).
pub const ROTATION_KEYS: usize = 8;

/// One 16-byte key at track +20 + 16 * index, written by 0x140026450.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RotationKey {
    /// +0: the key 0x1400264a0 moves to next; negative ends the track (-2 also marks the
    /// object for disposal).
    pub next: i32,
    /// +4: signed target angle in hundredths, 100 * (degrees % 360).
    pub value: i32,
    /// +8: low byte 0 linear, 1 eased, 0x80 periodic sine; bit 15 reverses direction.
    pub mode: u32,
    /// +12: milliseconds to reach `value`, compared unsigned with the elapsed time.
    pub duration: u32,
}

/// The draw state +120 rotation track (0x21e5). Header words: +0 playing, +4 base value,
/// +8 start key, +12 current key, +16 elapsed milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationTrack {
    pub playing: bool,
    pub base: i32,
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
    pub keys: [RotationKey; ROTATION_KEYS],
}
impl RotationTrack {
    pub(crate) fn supports(mode: u32) -> bool {
        matches!(mode as u8, 0 | 1 | 0x80)
    }

    /// 0x1400264a0: angles are signed hundredths of a degree, not float bits.
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        if !self.playing {
            return None;
        }
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        let key = self.keys[self.current as usize];
        if key.mode as u8 == 0x80 {
            self.elapsed %= key.duration;
            let phase = self.elapsed.wrapping_mul(36000) / key.duration;
            words[20] = self
                .base
                .wrapping_add(sine(phase as i32, key.value))
                .wrapping_add(36000)
                .wrapping_rem(36000) as u32;
            return None;
        }
        if key.duration > self.elapsed {
            let reverse = key.mode & 0x8000 != 0;
            let distance = if reverse {
                let base = if key.value >= self.base {
                    self.base.wrapping_add(36000)
                } else {
                    self.base
                };
                base.wrapping_sub(key.value)
            } else {
                let target = if key.value <= self.base {
                    key.value.wrapping_add(36000)
                } else {
                    key.value
                };
                target.wrapping_sub(self.base)
            };
            let offset = if key.mode as u8 == 1 {
                let phase = self.elapsed.wrapping_mul(9000) / key.duration;
                sine(phase as i32, distance)
            } else {
                self.elapsed
                    .wrapping_mul(distance as u32)
                    .wrapping_div(key.duration) as i32
            };
            let value = if reverse {
                self.base.wrapping_sub(offset).wrapping_add(36000)
            } else {
                self.base.wrapping_add(offset)
            };
            words[20] = value.wrapping_rem(36000) as u32;
            return None;
        }
        for _ in 0..ANIMATION_FRAMES {
            let key = self.keys[self.current as usize];
            if (key.mode as u8) < 0x80 {
                self.base = key.value;
                words[20] = key.value as u32;
            }
            if key.next < 0 {
                return Some(key.next);
            }
            self.elapsed = self.elapsed.wrapping_sub(key.duration);
            self.current = key.next as u32;
            if self.keys[self.current as usize].duration > self.elapsed {
                return None;
            }
        }
        None
    }
}
fn sine(angle: i32, amplitude: i32) -> i32 {
    let radians = f64::from(angle) / 18000.0 * std::f64::consts::PI;
    (radians.sin() * f64::from(amplitude)) as i32
}

impl SceneHost {
    /// 0x21e5..0x21e9: native wrappers 0x14006df30..0x14006e1d0.
    /// Slot/child, then constructor angle; key/next/mode/duration/angle; start key.
    pub(super) fn rotation(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        let count = match command {
            0x1e6 => 5,
            0x1e5 | 0x1e7 => 1,
            _ => 0,
        };
        let mut args = [0i32; 7];
        for (i, arg) in args.iter_mut().take(count + 2).enumerate() {
            *arg = call.argument(4 + 4 * i, super::argument_names(call.command)[i])? as i32;
        }
        let [slot, child, a, b, c, d, e] = args;
        let slot = u8::try_from(slot)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| error(pc, op, "effect needs an existing object in slots 0..255"))?;
        let child = match u16::try_from(child) {
            Err(_) if child < 0 => None,
            Ok(id) if id < 768 && self.objects[&slot].children.contains_key(&id) => Some(id),
            _ => return Err(error(pc, op, "effect child outside 0..767 or missing")),
        };
        let key_ok = |index: i32| usize::try_from(index).is_ok_and(|i| i < ROTATION_KEYS);
        if (matches!(command, 0x1e6 | 0x1e7) && !key_ok(a))
            || (command == 0x1e6 && b >= 0 && !key_ok(b))
        {
            return Err(error(
                pc,
                op,
                "rotation key outside 0..7 leaves the original track",
            ));
        }
        if command == 0x1e6 && (!RotationTrack::supports(c as u32) || (c as u8 == 0x80 && d == 0)) {
            return Err(error(
                pc,
                op,
                "unsupported rotation mode or zero sine period",
            ));
        }
        let parent = self.objects.get_mut(&slot).expect("checked above");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("checked above"),
            None => parent,
        };
        let draw = object
            .draw
            .as_mut()
            .ok_or_else(|| error(pc, op, "effect target has no draw state"))?;
        call.complete(0x4008 + 4 * count as u16)?;
        let magnitude = |value: i32| (value % 360) * 100;
        match command {
            0x1e5 => {
                draw.rotation_track = Some(Box::new(RotationTrack {
                    playing: false,
                    base: magnitude(a),
                    start: 0,
                    current: 0,
                    elapsed: 0,
                    keys: [RotationKey::default(); ROTATION_KEYS],
                }));
            }
            0x1e6 => {
                if let Some(track) = draw.rotation_track.as_deref_mut() {
                    track.keys[a as usize] = RotationKey {
                        next: b,
                        value: magnitude(e),
                        mode: c as u32,
                        duration: d as u32,
                    };
                }
            }
            0x1e7 => {
                if let Some(track) = draw.rotation_track.as_deref_mut() {
                    track.playing = true;
                    track.start = a as u32;
                    track.current = a as u32;
                    track.elapsed = 0;
                }
            }
            0x1e8 => {
                if let Some(track) = draw.rotation_track.take()
                    && track.playing
                {
                    draw.words[20] = track.keys[track.current as usize].value as u32;
                }
            }
            _ => {
                let result = match draw.rotation_track.as_deref() {
                    None => u32::MAX,
                    Some(track) if !track.playing => u32::MAX - 1,
                    Some(track) => track.current,
                };
                call.vm
                    .state_mut()
                    .set_system_integer(0, result)
                    .expect("slot 0 exists");
            }
        }
        Ok(())
    }
}

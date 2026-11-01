//! Draw-state opacity track; addresses refer to the documented binary hash.
use super::{ANIMATION_FRAMES, SceneHost};
use crate::{bytecode::DecodeError, dispatch::Call, evaluate::error};

/// Keys in the 0x94-byte opacity track (0x1400267a0 zeroes them all).
pub const OPACITY_KEYS: usize = 8;

/// One 16-byte key at track +20 + 16 * index, written by 0x140026870.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpacityKey {
    /// +0: the key 0x1400268d0 moves to next; negative ends the track (-2 also marks the
    /// object for disposal).
    pub next: i32,
    /// +4: target opacity, |value| % 256 as C computes it.
    pub value: i32,
    /// +8: while its low byte is set the opacity holds instead of interpolating.
    pub hold: u32,
    /// +12: milliseconds to reach `value`, compared unsigned with the elapsed time.
    pub duration: u32,
}

/// The draw state +128 opacity track (0x21ea). Header words: +0 playing, +4 base value,
/// +8 start key, +12 current key, +16 elapsed milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpacityTrack {
    pub playing: bool,
    pub base: i32,
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
    pub keys: [OpacityKey; OPACITY_KEYS],
}
impl OpacityTrack {
    /// 0x1400268d0 for a playing track, writing draw word 17: add the frame delta (at most
    /// 200 ms); while the current key's duration has elapsed, take its value as both base
    /// and opacity, end with its negative `next`, or move on subtracting the duration.
    /// Otherwise interpolate `base + elapsed * (value - base) / duration` (unsigned 32-bit
    /// product, signed division) unless the key holds. A cycle of zero-length keys never
    /// exits in the original; Rust moves at most ANIMATION_FRAMES times per update.
    pub(super) fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        if !self.playing {
            return None;
        }
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        let key = self.keys[self.current as usize];
        if key.duration > self.elapsed {
            if key.hold & 0xff == 0 {
                let span = (key.value.wrapping_sub(self.base) as u32).wrapping_mul(self.elapsed);
                let step = (span as i32).wrapping_div(key.duration as i32);
                words[17] = self.base.wrapping_add(step) as u32;
            }
            return None;
        }
        for _ in 0..ANIMATION_FRAMES {
            let key = self.keys[self.current as usize];
            self.base = key.value;
            words[17] = key.value as u32;
            if key.next < 0 {
                return Some(key.next);
            }
            self.elapsed -= key.duration;
            self.current = key.next as u32;
            if self.keys[self.current as usize].duration > self.elapsed {
                return None;
            }
        }
        None
    }
}

impl SceneHost {
    /// The opacity track family 0x21ea..0x21ee (0x14006e260..0x14006e500): SP-4 slot (signed
    /// < 256 with an object, otherwise +12056 bit 0x10), SP-8 child (>= 768 sets bit 0x20,
    /// negative selects the object; a missing child or draw state would be dereferenced).
    /// - 0x21ea (0x14001fab0): replace the track with an empty one whose base is |SP-12| % 256.
    /// - 0x21eb (0x14001fbc0 -> 0x140026870): key SP-12 gets next SP-16, hold SP-20, duration
    ///   SP-24 and value |SP-28| % 256; no track is a no-op. Pops 28.
    /// - 0x21ec (0x14001fba0 -> 0x1400268b0): play from key SP-12 (no opacity change yet).
    /// - 0x21ed (0x14001fb40 -> 0x140026430): a playing track writes its current key's value
    ///   to opacity; the track is freed. Pops 8.
    /// - 0x21ee (0x14001fbf0): system result 0 = current key while playing, -2 when stopped,
    ///   -1 without a track. Pops 8.
    ///
    /// The original bounds neither key indices nor `next`; Rust accepts keys 0..7.
    pub(super) fn opacity(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        let count = match command {
            0x1eb => 5,
            0x1ea | 0x1ec => 1,
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
        let key_ok = |index: i32| usize::try_from(index).is_ok_and(|i| i < OPACITY_KEYS);
        if (matches!(command, 0x1eb | 0x1ec) && !key_ok(a))
            || (command == 0x1eb && b >= 0 && !key_ok(b))
        {
            return Err(error(
                pc,
                op,
                "opacity key outside 0..7 leaves the original track",
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
        let magnitude = |value: i32| value.wrapping_abs() % 256;
        match command {
            0x1ea => {
                draw.opacity_track = Some(Box::new(OpacityTrack {
                    playing: false,
                    base: magnitude(a),
                    start: 0,
                    current: 0,
                    elapsed: 0,
                    keys: [OpacityKey::default(); OPACITY_KEYS],
                }));
            }
            0x1eb => {
                if let Some(track) = draw.opacity_track.as_deref_mut() {
                    track.keys[a as usize] = OpacityKey {
                        next: b,
                        value: magnitude(e),
                        hold: c as u32,
                        duration: d as u32,
                    };
                }
            }
            0x1ec => {
                if let Some(track) = draw.opacity_track.as_deref_mut() {
                    track.playing = true;
                    track.start = a as u32;
                    track.current = a as u32;
                    track.elapsed = 0;
                }
            }
            0x1ed => {
                if let Some(track) = draw.opacity_track.take()
                    && track.playing
                {
                    draw.words[17] = track.keys[track.current as usize].value as u32;
                }
            }
            _ => {
                let result = match draw.opacity_track.as_deref() {
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

//! Engine +4632 and +4648 mask transitions (0x4b0/0x4b8-byte objects built by 0x140051450 /
//! 0x140051d10); addresses refer to the documented binary hash. 0x14001d150 selects the
//! pixel-shader path when the device reports ps_2_0 and the configuration enables it; this
//! model follows that path: the mask texture is attached to the slot object and
//! MaskEffectPut.o2 / MaskEffectCut.o2 (ps.cpz, dwords XORed with "PSX2") scale the object's
//! alpha. The CPU path, which rewrites the image alpha through a threshold table (the +4648
//! object also saves the original alpha at +1056), is not modeled. On the shader path the
//! two channels differ only by an 8-byte shift of the fields after +1056 and by 0x2234 not
//! detaching the previous transition's shader.
use super::{ObjectImage, SceneHost, load_image};
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow},
    evaluate::error,
    fade::SkipGate,
    image::ImageSource,
    input::ActionInput,
    machine::{Machine, Step},
    vm::SystemResult,
};
use std::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskTransition {
    /// +0, SP-20: nonzero reveals the object (MaskEffectPut); zero hides it (MaskEffectCut)
    /// and destroys the object when the transition ends.
    pub put: bool,
    /// +4, SP-8: target object slot.
    pub slot: u8,
    /// +8 = SP-12 − SP-16 and +12 = SP-16 as u32; their sum is the duration. The shader
    /// path uses only the sum; the CPU path would use +12 as the edge width.
    pub ramp: u32,
    pub width: u32,
    /// +1056: elapsed milliseconds, clamped to the duration.
    pub elapsed: u32,
    /// +1064 name and the decoded mask texture; the shaders sample its green channel.
    pub mask: ObjectImage,
}

/// Object bytes record 0x620 copies after the name (+0..+1064).
const SAVED_OBJECT: usize = 0x428;

impl MaskTransition {
    pub fn duration(&self) -> u32 {
        self.ramp.wrapping_add(self.width)
    }
    /// Record 0x620 (0x140051170): slot, duration, edge width and mode for the reader's
    /// constructor, the mask name, then the object bytes +0..+1064. On the shader path
    /// +16..+32 (CPU mask buffer and size) and the +32 threshold table stay zero; +1060 is
    /// written as zero.
    pub(crate) fn saved(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for word in [
            u32::from(self.slot),
            self.duration(),
            self.width,
            u32::from(self.put),
        ] {
            out.extend(word.to_le_bytes());
        }
        out.extend(&self.mask.name);
        out.push(0);
        let mut object = [0u8; SAVED_OBJECT];
        for (at, word) in [
            (0, u32::from(self.put)),
            (4, u32::from(self.slot)),
            (8, self.ramp),
            (12, self.width),
            (1056, self.elapsed),
        ] {
            object[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        out.extend(object);
        out
    }
    /// Reader: 0x140051450 builds a new transition from the head words and the name on the
    /// rebuilt slot object, so the transition restarts with elapsed 0; the object bytes go
    /// to an empty function (0x14006ad10) and are not read.
    pub(crate) fn restore(
        record: &[u8],
        objects: &std::collections::BTreeMap<u8, crate::scene::SceneObject>,
        images: &mut dyn ImageSource,
    ) -> Result<Self, Box<dyn Error>> {
        let word = |at: usize| -> Result<u32, &'static str> {
            Ok(u32::from_le_bytes(
                record
                    .get(at..at + 4)
                    .ok_or("truncated mask record")?
                    .try_into()
                    .unwrap(),
            ))
        };
        let (slot, duration, width, put) = (word(0)?, word(4)?, word(8)?, word(12)?);
        let tail = record.get(16..).ok_or("truncated mask record")?;
        let end = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or("mask record name lacks its NUL")?;
        if tail.len() != end + 1 + SAVED_OBJECT {
            return Err("mask record length differs from the original".into());
        }
        let slot = u8::try_from(slot)
            .ok()
            .filter(|slot| objects.contains_key(slot))
            .ok_or("saved mask transition targets a missing object")?;
        let name = tail[..end].to_vec();
        if name.len() >= 64 {
            return Err("saved mask name exceeds the original 64-byte buffers".into());
        }
        let decoded =
            crate::image::load(images, &name)?.ok_or("saved mask image has an unknown codec")?;
        let mask = ObjectImage {
            filter: None,
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            surface: false,
        };
        Ok(Self {
            put: put != 0,
            slot,
            ramp: duration.wrapping_sub(width),
            width,
            elapsed: 0,
            mask,
        })
    }
    /// Shader constant `count` (0x1400517a0 at creation, 0x14005189e per frame): f32
    /// elapsed / duration × 2.0 for Put and 2.0 minus that for Cut; the end value once the
    /// elapsed time reaches the duration, so a zero duration never divides.
    pub fn progress(&self) -> f32 {
        let duration = self.duration();
        if self.elapsed >= duration {
            return if self.put { 2.0 } else { 0.0 };
        }
        let ratio = self.elapsed as f32 / duration as f32 * 2.0;
        if self.put { ratio } else { 2.0 - ratio }
    }
    /// Shader factor for a mask texel's green byte m: Put keeps min(p − m, 1) where p > m,
    /// Cut keeps min(p − (1 − m), 1) where p > 1 − m, else 0. It scales the texel alpha
    /// times the vertex (opacity) alpha.
    pub fn coverage(&self, green: u8) -> f32 {
        let m = f32::from(green) / 255.0;
        let threshold = if self.put { m } else { 1.0 - m };
        let p = self.progress();
        if threshold - p >= 0.0 {
            0.0
        } else {
            (p - threshold).min(1.0)
        }
    }
    /// 0x140051810: true once the elapsed time reaches the duration (and is clamped).
    fn advance(&mut self, delta: u32) -> bool {
        self.elapsed = self.elapsed.wrapping_add(delta);
        let duration = self.duration();
        if self.elapsed < duration {
            return false;
        }
        self.elapsed = duration;
        true
    }
}

/// (channel, command offset) of 0x222a..0x222d (+4632) and 0x2234..0x2237 (+4648):
/// create, wait, query (unhandled, unused by this title), finish.
fn channel(command: u16) -> Option<(usize, u16)> {
    match command {
        0x22a..=0x22d => Some((0, command - 0x22a)),
        0x234..=0x237 => Some((1, command - 0x234)),
        _ => None,
    }
}

impl SceneHost {
    /// The +4632 transition (0x222a..0x222d).
    pub fn mask(&self) -> Option<&MaskTransition> {
        self.masks[0].as_ref()
    }
    /// The +4648 transition (0x2234..0x2237).
    pub fn second_mask(&self) -> Option<&MaskTransition> {
        self.masks[1].as_ref()
    }
    /// The transition shading the object in `slot`; creation keeps it to one per object.
    pub fn mask_on(&self, slot: u8) -> Option<&MaskTransition> {
        self.masks.iter().flatten().find(|mask| mask.slot == slot)
    }

    /// 0x14007b270 / 0x14007b500 (0x222b/0x2235 skip, 0x222d/0x2237) and the frame update's
    /// end branch: a Put object keeps its final state without the shader; a Cut object is
    /// destroyed. A missing object only frees the transition.
    fn finish_mask(&mut self, channel: usize) {
        if let Some(mask) = self.masks[channel].take()
            && !mask.put
        {
            self.objects.remove(&mask.slot);
        }
    }

    /// The +4632 and then +4648 parts of the frame update 0x140015900, after the fade and
    /// the strip wipe. The original would dereference a target object destroyed meanwhile;
    /// here only the timing continues.
    pub(super) fn advance_mask(&mut self, delta: u32) {
        for channel in 0..2 {
            if self.masks[channel]
                .as_mut()
                .is_some_and(|mask| mask.advance(delta))
            {
                self.finish_mask(channel);
            }
        }
    }

    /// 0x222a / 0x14007ae40 and 0x2234 / 0x14007b370: SP-4 mask name, SP-8 object slot,
    /// SP-12 duration, SP-16 edge, SP-20 mode. 0x222a detaches the previous transition's
    /// shader (0x140051140) and frees it without finishing (a Cut object survives); 0x2234
    /// only frees it, so an old target other than the new one would stay shaded, which is
    /// rejected here, as is a second transition on an object the other channel shades. The
    /// shader path always succeeds: system result 0 = 1, returns 0x4014. 0x222d / 0x14007b270
    /// and 0x2237 / 0x14007b500 finish the transition and return 0x4000. The slot is not
    /// bounds-checked and the shader setup dereferences the object, so missing objects are
    /// errors, as are mask images the 0x2030 lookup cannot decode.
    pub fn try_handle_mask(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        let (channel, offset) = match channel(command) {
            Some((channel, offset @ (0 | 3))) => (channel, offset),
            _ => return Ok(false),
        };
        if offset == 3 {
            vm.complete_system(SystemResult(0x4000))?;
            self.finish_mask(channel);
            return Ok(true);
        }
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        let mut args = [0u32; 5];
        for (i, value) in args.iter_mut().enumerate() {
            *value = vm.stack().peek_u32(4 * (i + 1)).map_err(|e| fail(e.0))?;
        }
        let [name, slot, duration, width, put] = args;
        let slot = u8::try_from(slot as i32)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| fail("mask transition needs an existing object in slots 0..255"))?;
        if self.masks[1 - channel]
            .as_ref()
            .is_some_and(|other| other.slot == slot)
        {
            return Err(fail("two mask transitions on one object are not modeled").into());
        }
        if channel == 1
            && self.masks[1]
                .as_ref()
                .is_some_and(|old| old.slot != slot && self.objects.contains_key(&old.slot))
        {
            return Err(fail("0x2234 would leave the previous object shaded (not modeled)").into());
        }
        let name = vm.resolve_string(name)?;
        let mask = load_image(images, name, pc, op)?.ok_or_else(|| {
            fail("undecodable mask leaves an unmodeled transition without shader")
        })?;
        vm.complete_system(SystemResult(0x4014))?;
        self.masks[channel] = Some(MaskTransition {
            put: put != 0,
            slot,
            ramp: duration.wrapping_sub(width),
            width,
            elapsed: 0,
            mask,
        });
        vm.state_mut()
            .set_system_integer(0, 1)
            .expect("slot 0 exists");
        Ok(true)
    }

    /// 0x222b / 0x140081170 and 0x2235 / 0x140081230 for their channel: system result 0 = 0;
    /// without a transition returns 0x4004. With SP-4 nonzero the wait ends at once when
    /// effects are off, action 7 or 8 is held or the skip flag is set, or on a released
    /// action 0, which is consumed: the transition finishes as for 0x222d/0x2237, result 0 =
    /// 1, 0xc004. Otherwise 0xa000 until the frame update frees the transition (always for
    /// SP-4 zero).
    pub fn try_wait_mask(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(None);
        };
        let Some((_, 1)) = channel(command) else {
            return Ok(None);
        };
        self.handle_wait_mask(&mut Call::new(vm, pc, command), gate, input)
            .map(|flow| Some(flow.wait_step()))
    }

    pub(crate) fn handle_wait_mask(
        &mut self,
        call: &mut Call<'_, '_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Flow, DecodeError> {
        let command = call.command;
        let Some((channel, 1)) = channel(command) else {
            return Err(call.error("unregistered mask wait"));
        };
        let skippable = call.argument(4, "skippable")? != 0;
        if self.masks[channel].is_none() {
            let flow = call.finish(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0");
            return Ok(flow);
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let released = skippable && !immediate && input.released(0);
        if skippable && (immediate || released) {
            let flow = call.finish(0xc004)?;
            if released {
                input.consume(0);
            }
            call.vm
                .state_mut()
                .set_system_integer(0, 1)
                .expect("slot 0");
            self.finish_mask(channel);
            return Ok(flow);
        }
        let flow = call.wait(4)?;
        call.vm
            .state_mut()
            .set_system_integer(0, 0)
            .expect("slot 0");
        Ok(flow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{raster::RgbaImage, scene::SceneObject};
    use std::collections::BTreeMap;

    struct Missing;
    impl ImageSource for Missing {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            Ok(None)
        }
    }

    fn transition() -> MaskTransition {
        MaskTransition {
            put: true,
            slot: 7,
            ramp: 300,
            width: 100,
            elapsed: 250,
            mask: ObjectImage {
                filter: None,
                name: b"mask.pb3".to_vec(),
                pixels: RgbaImage::new(1, 1, vec![0, 128, 0, 255]).unwrap(),
                alpha: false,
                surface: false,
            },
        }
    }

    #[test]
    fn mask_records_hold_head_words_name_and_the_shader_path_object() {
        let saved = transition().saved();
        assert_eq!(saved.len(), 16 + 9 + SAVED_OBJECT);
        let word = |at: usize| u32::from_le_bytes(saved[at..at + 4].try_into().unwrap());
        assert_eq!([word(0), word(4), word(8), word(12)], [7, 400, 100, 1]);
        assert_eq!(&saved[16..25], b"mask.pb3\0");
        let object = 25;
        assert_eq!(
            [
                word(object),
                word(object + 4),
                word(object + 8),
                word(object + 12)
            ],
            [1, 7, 300, 100]
        );
        assert_eq!(word(object + 1056), 250);
        assert!(saved[object + 16..object + 1056].iter().all(|&b| b == 0));
    }

    #[test]
    fn mask_records_need_their_object_image_and_exact_length() {
        let saved = transition().saved();
        let objects = BTreeMap::from([(7u8, SceneObject::default())]);
        // The image lookup runs only for a well-formed record on an existing object.
        let missing = MaskTransition::restore(&saved, &objects, &mut Missing).unwrap_err();
        assert!(missing.to_string().contains("mask.pb3"), "{missing}");
        assert!(MaskTransition::restore(&saved, &BTreeMap::new(), &mut Missing).is_err());
        assert!(
            MaskTransition::restore(&saved[..saved.len() - 1], &objects, &mut Missing).is_err()
        );
        let mut unterminated = saved.clone();
        unterminated.truncate(24);
        assert!(MaskTransition::restore(&unterminated, &objects, &mut Missing).is_err());
    }
}

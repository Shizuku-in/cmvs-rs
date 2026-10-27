//! Engine +4632 mask transition (0x4b0-byte object built by 0x140051450); addresses refer to
//! the documented binary hash. 0x14001d150 selects the pixel-shader path when the device
//! reports ps_2_0 and the configuration enables it; this model follows that path: the mask
//! texture is attached to the slot object and MaskEffectPut.o2 / MaskEffectCut.o2 (ps.cpz,
//! dwords XORed with "PSX2") scale the object's alpha. The CPU path, which rewrites the image
//! alpha through a threshold table, is not modeled.
use super::{ObjectImage, SceneHost, load_image};
use crate::{
    bytecode::DecodeError,
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

impl MaskTransition {
    pub fn duration(&self) -> u32 {
        self.ramp.wrapping_add(self.width)
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

impl SceneHost {
    pub fn mask(&self) -> Option<&MaskTransition> {
        self.mask.as_ref()
    }

    /// 0x14007b270 (0x222b skip, 0x222d) and the frame update's end branch: a Put object
    /// keeps its final state without the shader; a Cut object is destroyed. A missing
    /// object only frees the transition.
    fn finish_mask(&mut self) {
        if let Some(mask) = self.mask.take()
            && !mask.put
        {
            self.objects.remove(&mask.slot);
        }
    }

    /// The +4632 part of the frame update 0x140015900, after the fade. The original would
    /// dereference a target object destroyed meanwhile; here only the timing continues.
    pub(super) fn advance_mask(&mut self, delta: u32) {
        if self.mask.as_mut().is_some_and(|mask| mask.advance(delta)) {
            self.finish_mask();
        }
    }

    /// 0x222a / 0x14007ae40: SP-4 mask name, SP-8 object slot, SP-12 duration, SP-16 edge,
    /// SP-20 mode. The previous transition loses its shader and is freed without finishing
    /// (a Cut object survives). The shader path always succeeds: system result 0 = 1,
    /// returns 0x4014. 0x222d / 0x14007b270 finishes the transition and returns 0x4000.
    /// The slot is not bounds-checked and the shader setup dereferences the object, so
    /// missing objects are errors, as are mask images the 0x2030 lookup cannot decode.
    pub fn try_handle_mask(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let pc = match vm.pending_system() {
            Some((pc, 0x22a)) => pc,
            Some((_, 0x22d)) => {
                vm.complete_system(SystemResult(0x4000))?;
                self.finish_mask();
                return Ok(true);
            }
            _ => return Ok(false),
        };
        let fail = |why| error(pc, 0x222a, why);
        let mut args = [0u32; 5];
        for (i, value) in args.iter_mut().enumerate() {
            *value = vm.stack().peek_u32(4 * (i + 1)).map_err(|e| fail(e.0))?;
        }
        let [name, slot, duration, width, put] = args;
        let slot = u8::try_from(slot as i32)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| fail("mask transition needs an existing object in slots 0..255"))?;
        let name = vm.resolve_string(name)?;
        let mask = load_image(images, name, pc, 0x222a)?.ok_or_else(|| {
            fail("undecodable mask leaves an unmodeled transition without shader")
        })?;
        vm.complete_system(SystemResult(0x4014))?;
        self.mask = Some(MaskTransition {
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

    /// 0x222b / 0x140081170: system result 0 = 0; without a transition returns 0x4004.
    /// With SP-4 nonzero the wait ends at once when effects are off, action 7 or 8 is held
    /// or the skip flag is set, or on a released action 0, which is consumed: the transition
    /// finishes as for 0x222d, result 0 = 1, 0xc004. Otherwise 0xa000 until the frame update
    /// frees the transition (always for SP-4 zero).
    pub fn try_wait_mask(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x22b)) = vm.pending_system() else {
            return Ok(None);
        };
        let skippable = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x222b, e.0))? != 0;
        if self.mask.is_none() {
            let step = vm.complete_system(SystemResult(0x4004))?;
            vm.state_mut().set_system_integer(0, 0).expect("slot 0");
            return Ok(Some(step));
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let released = skippable && !immediate && input.released(0);
        if skippable && (immediate || released) {
            let step = vm.complete_system(SystemResult(0xc004))?;
            if released {
                input.consume(0);
            }
            vm.state_mut().set_system_integer(0, 1).expect("slot 0");
            self.finish_mask();
            return Ok(Some(step));
        }
        let step = vm.yield_pending_system()?;
        vm.state_mut().set_system_integer(0, 0).expect("slot 0");
        Ok(Some(step))
    }
}

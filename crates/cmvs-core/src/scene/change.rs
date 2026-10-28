//! 0x22c4 / 0x14007d340 -> 0x140015620 -> 0x14003c900, software fallback.
//! Timed replacement keeps the previous object and an opaque common-area image. The
//! shader optimization and secondary image manager are not modeled. The renderer queues
//! common, previous and current images with their individual fade factors. Addresses are bound to the binary hash in vm-semantics.md.
//! 0x22c5..0x22c7 cancel, query and wait on this lifecycle without advancing frame time.
use super::{DrawState, ObjectImage, SceneHost, SceneObject, load_image};
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    fade::SkipGate,
    image::ImageSource,
    input::ActionInput,
    machine::{Machine, Step},
    raster::RgbaImage,
    vm::SystemResult,
};
use std::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageChange {
    /// +14512: prior object, including its children, after moving its effect tracks.
    pub previous: Box<SceneObject>,
    /// +14520: centered overlap containing new RGB only where both alphas equal 255.
    pub common: Box<SceneObject>,
    /// +14528, raw unsigned milliseconds from SP-12.
    pub duration: u32,
    /// +14532, advanced with wrapping u32 addition.
    pub elapsed: u32,
}

impl SceneObject {
    /// 0x14003c830 cancels a previous replacement and restores the current fade factor.
    fn cancel_change(&mut self) {
        self.change = None;
        if let Some(draw) = &mut self.draw {
            draw.words[21] = 256;
        }
    }

    /// 0x14003cdd0, after this object's and its children's effects. Retained objects
    /// advance first, then elapsed wraps as u32; only position/order/world XYZ follow
    /// the new draw state. Effects were moved to the new object, not cloned.
    pub(super) fn advance_change(&mut self, delta: u32) {
        let Some(change) = &mut self.change else {
            return;
        };
        change.previous.advance_effects(delta);
        change.common.advance_effects(delta);
        change.elapsed = change.elapsed.wrapping_add(delta);
        let draw = self.draw.as_mut().expect("replacement has a draw state");
        if change.elapsed >= change.duration {
            draw.words[21] = 256;
            self.change = None;
            return;
        }
        for object in [&mut change.previous, &mut change.common] {
            object.draw.as_mut().expect("retained draw state").words[8..14]
                .copy_from_slice(&draw.words[8..14]);
        }
        let factor = change.elapsed.wrapping_shl(8) / change.duration;
        draw.words[21] = factor;
        change
            .previous
            .draw
            .as_mut()
            .expect("retained draw state")
            .words[21] = 256u32.wrapping_sub(factor);
    }
}

fn fit(draw: &mut DrawState, old: &ObjectImage, new: &ObjectImage) {
    if (old.pixels.width(), old.pixels.height()) != (new.pixels.width(), new.pixels.height()) {
        draw.words[2..6].copy_from_slice(&[0, 0, new.pixels.width(), new.pixels.height()]);
        draw.words[6] = if draw.words[6] == 0 {
            0
        } else {
            new.pixels.width() >> 1
        };
    }
}

/// 0x140032d70: common area has min dimensions, centered horizontally, top-aligned.
/// Keep the NEW pixel only where both images are fully opaque; other RGB is irrelevant.
fn common_image(old: &ObjectImage, new: &ObjectImage) -> Result<ObjectImage, Box<dyn Error>> {
    let width = old.pixels.width().min(new.pixels.width());
    let height = old.pixels.height().min(new.pixels.height());
    let mut pixels = vec![0; (u64::from(width) * u64::from(height) * 4) as usize];
    let old_x = (old.pixels.width() - width) >> 1;
    let new_x = (new.pixels.width() - width) >> 1;
    for y in 0..height {
        for x in 0..width {
            let a = ((u64::from(y) * u64::from(old.pixels.width()) + u64::from(old_x + x)) * 4)
                as usize;
            let b = ((u64::from(y) * u64::from(new.pixels.width()) + u64::from(new_x + x)) * 4)
                as usize;
            if old.pixels.pixels()[a + 3] == 255 && new.pixels.pixels()[b + 3] == 255 {
                let dst = ((u64::from(y) * u64::from(width) + u64::from(x)) * 4) as usize;
                pixels[dst..dst + 4].copy_from_slice(&new.pixels.pixels()[b..b + 4]);
            }
        }
    }
    Ok(ObjectImage {
        filter: None,
        name: Vec::new(),
        pixels: RgbaImage::new(width, height, pixels)?,
        alpha: true,
    })
}

pub(crate) struct PreparedChange {
    image: ObjectImage,
    draw: DrawState,
    common: Option<Box<SceneObject>>,
    duration: u32,
}

impl SceneHost {
    /// 0x22c5 / 0x14007d3c0 cancels the slot object's replacement, restoring factor 256.
    /// 0x22c6 / 0x14007d400 writes whether +14504 or +14508 is active to result 0.
    /// Both take only SP-4, return 0x4004 and do not advance time or yield.
    pub(super) fn control_change(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let slot = self.change_slot(vm, pc, command)?;
        if command == 0x2c5 && self.objects[&slot].draw.is_none() {
            return Err(error(
                pc,
                0x22c5,
                "canceling replacement needs a draw state",
            ));
        }
        let active = self.objects[&slot].change.is_some();
        vm.complete_system(SystemResult(0x4004))?;
        if command == 0x2c5 {
            self.objects
                .get_mut(&slot)
                .expect("validated")
                .cancel_change();
        } else {
            vm.state_mut()
                .set_system_integer(0, u32::from(active))
                .expect("result 0 exists");
        }
        Ok(true)
    }

    fn change_slot(&self, vm: &Machine<'_>, pc: usize, command: u16) -> Result<u8, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let raw = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        // The original sign-extends and indexes the slot table without bounds/null checks.
        u8::try_from(raw)
            .ok()
            .filter(|s| self.objects.contains_key(s))
            .ok_or_else(|| fail("replacement control needs an existing object in slots 0..255"))
    }

    /// 0x22c7 / 0x14007d440: SP-4 slot, SP-8 skippable. While active return 0xa000,
    /// preserving the command/arguments. No active replacement: result 0=0, 0x4008.
    /// Skipping cancels, result 0=1, 0xc008; only a release-triggered skip consumes input.
    pub fn try_wait_change(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x2c7)) = vm.pending_system() else {
            return Ok(None);
        };
        let slot = self.change_slot(vm, pc, 0x2c7)?;
        let skippable = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x22c7, e.0))? != 0;
        if self.objects[&slot].change.is_none() {
            let step = vm.complete_system(SystemResult(0x4008))?;
            vm.state_mut()
                .set_system_integer(0, 0)
                .expect("result 0 exists");
            return Ok(Some(step));
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let released = skippable && !immediate && input.released(0);
        if skippable && (immediate || released) {
            let step = vm.complete_system(SystemResult(0xc008))?;
            if released {
                input.consume(0);
            }
            vm.state_mut()
                .set_system_integer(0, 1)
                .expect("result 0 exists");
            self.objects
                .get_mut(&slot)
                .expect("validated")
                .cancel_change();
            return Ok(Some(step));
        }
        let step = vm.yield_pending_system()?;
        vm.state_mut()
            .set_system_integer(0, 0)
            .expect("result 0 exists");
        Ok(Some(step))
    }

    /// SP-4 unsigned slot (unbounded in the original), SP-8 name, SP-12 duration,
    /// SP-16 manager selector (0 = primary). Writes result 0, returns 0x4010, no yield.
    pub fn try_change_image(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x2c4)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, 0x22c4, why);
        let mut args = [0; 4];
        for (i, arg) in args.iter_mut().enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let name = vm.resolve_string(args[1])?;
        let slot = u8::try_from(args[0]).map_err(|_| fail("replacement slot outside 0..255"))?;
        let Some(_) = self.objects.get(&slot) else {
            vm.complete_system(SystemResult(0x4010))?;
            vm.state_mut()
                .set_system_integer(0, 0)
                .expect("result 0 exists");
            return Ok(true);
        };
        if args[3] != 0 {
            return Err(fail("secondary replacement image manager is not implemented").into());
        }
        let prepared = self.prepare_image_change(slot, &name, args[2], images, pc, 0x22c4)?;
        vm.complete_system(SystemResult(0x4010))?;
        vm.state_mut()
            .set_system_integer(0, 1)
            .expect("result 0 exists");
        self.commit_image_change(slot, prepared.expect("validated existing slot"));
        Ok(true)
    }

    /// Prepare all fallible work before replacing the slot or consuming a VM command.
    pub(crate) fn prepare_image_change(
        &self,
        slot: u8,
        name: &[u8],
        duration: u32,
        images: &mut dyn ImageSource,
        pc: usize,
        opcode: u16,
    ) -> Result<Option<PreparedChange>, Box<dyn Error>> {
        let fail = |why| error(pc, opcode, why);
        let Some(old) = self.objects.get(&slot) else {
            return Ok(None);
        };
        let old_image = old
            .image
            .as_ref()
            .ok_or_else(|| fail("replacement needs an existing image"))?;
        let old_draw = old
            .draw
            .as_ref()
            .ok_or_else(|| fail("replacement needs an existing draw state"))?;
        let mut image = load_image(images, name.to_vec(), pc, opcode)?
            .ok_or_else(|| fail("missing replacement image leaves an unmodeled partial object"))?;
        if let Some(filter) = old_image.filter {
            image.apply_filter(filter)?;
        }
        // 0x14003c900 copies the first 88 bytes, then 0x14001f520 copies +88..+108 into
        // both the new and the common draw state.
        let mut draw = DrawState {
            words: old_draw.words,
            depth_bias: old_draw.depth_bias,
            opacity_track: None,
            animation: None,
            position_motion: None,
            world_motion: None,
        };
        draw.words[21] = 256;
        fit(&mut draw, old_image, &image);
        let common = if duration != 0 {
            let image = common_image(old_image, &image)?;
            let mut common_draw = DrawState {
                words: old_draw.words,
                depth_bias: old_draw.depth_bias,
                opacity_track: None,
                animation: None,
                position_motion: None,
                world_motion: None,
            };
            common_draw.words[21] = 256;
            fit(&mut common_draw, old_image, &image);
            Some(Box::new(SceneObject {
                image: Some(image),
                draw: Some(common_draw),
                logical_size: old.logical_size,
                ..SceneObject::default()
            }))
        } else {
            None
        };
        Ok(Some(PreparedChange {
            image,
            draw,
            common,
            duration,
        }))
    }

    pub(crate) fn commit_image_change(&mut self, slot: u8, prepared: PreparedChange) {
        let PreparedChange {
            image,
            mut draw,
            common,
            duration,
        } = prepared;
        let mut previous = self.objects.remove(&slot).expect("validated slot");
        previous.cancel_change();
        let old_draw = previous.draw.as_mut().expect("validated draw");
        draw.opacity_track = old_draw.opacity_track.take();
        draw.animation = old_draw.animation.take();
        draw.position_motion = old_draw.position_motion.take();
        draw.world_motion = old_draw.world_motion.take();
        let mut replacement = SceneObject {
            image: Some(image),
            draw: Some(draw),
            logical_size: previous.logical_size,
            ..SceneObject::default()
        };
        if let Some(common) = common {
            replacement.draw.as_mut().expect("new draw").words[21] = 0;
            replacement.change = Some(Box::new(ImageChange {
                previous: Box::new(previous),
                common,
                duration,
                elapsed: 0,
            }));
        }
        self.objects.insert(slot, replacement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixels: Vec<u8>) -> ObjectImage {
        ObjectImage {
            filter: None,
            name: b"synthetic".to_vec(),
            pixels: RgbaImage::new(width, height, pixels).unwrap(),
            alpha: true,
        }
    }

    #[test]
    fn common_area_is_centered_horizontally_top_aligned_and_requires_two_opaque_pixels() {
        let old = image(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 254]);
        let new = image(
            4,
            2,
            vec![
                10, 11, 12, 255, 20, 21, 22, 255, 30, 31, 32, 255, 40, 41, 42, 255, 50, 51, 52,
                255, 60, 61, 62, 255, 70, 71, 72, 255, 80, 81, 82, 255,
            ],
        );
        let common = common_image(&old, &new).unwrap();
        assert_eq!((common.pixels.width(), common.pixels.height()), (2, 1));
        assert_eq!(common.pixels.pixels(), &[20, 21, 22, 255, 0, 0, 0, 0]);
        let reverse = common_image(&new, &old).unwrap();
        assert_eq!(reverse.pixels.pixels(), &[1, 2, 3, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn resizing_resets_crop_and_nonzero_horizontal_origin_but_preserves_other_words() {
        let old = image(2, 1, vec![255; 8]);
        let new = image(4, 2, vec![255; 32]);
        for origin in [0, 1, u32::MAX] {
            let mut draw = DrawState::default();
            draw.words[2..8].copy_from_slice(&[1, 1, 1, 1, origin, 99]);
            draw.words[11..14].copy_from_slice(&[1, 2, 3]);
            let mut expected = draw.clone();
            fit(&mut draw, &old, &old);
            assert_eq!(draw, expected);
            fit(&mut draw, &old, &new);
            expected.words[2..6].copy_from_slice(&[0, 0, 4, 2]);
            expected.words[6] = if origin == 0 { 0 } else { 2 };
            assert_eq!(draw, expected);
        }
    }
}

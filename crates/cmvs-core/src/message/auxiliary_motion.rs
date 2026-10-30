//! 0x14002e920 / 0x14002e9d0 startup and 0x14002edb0 frame interpolation.
use super::{MessageHost, auxiliary::PositionMotion};
use crate::{
    FormatError, bytecode::DecodeError, evaluate::error, machine::Machine, scene::SceneHost,
    vm::SystemResult,
};

impl MessageHost {
    pub(super) fn try_auxiliary_motion(
        &mut self,
        vm: &mut Machine<'_>,
    ) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x39c | 0x39f))) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let count = if command == 0x39c { 4 } else { 6 };
        let mut args = [0; 6];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let slot = args[0] as usize;
        let group = self
            .auxiliary_texts
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("auxiliary text group does not exist or outside 0..7"))?;
        let (mode, delta, extra, duration) = if command == 0x39c {
            (0, [args[1], args[2]], 0, args[3])
        } else {
            (args[1], [args[2], args[3]], args[4], args[5])
        };
        if mode > 1 || duration > i32::MAX as u32 {
            return Err(fail("auxiliary motion mode or duration is not verified"));
        }
        let base = group.position();
        let blocked = delta
            .into_iter()
            .zip(base)
            .zip([(152, 156), (160, 164)])
            .any(|((delta, base), (lo, hi))| {
                let d = delta as i32;
                let b = base as i32;
                (d > 0 && b >= group.word(hi).expect("bound") as i32)
                    || (d < 0 && b <= group.word(lo).expect("bound") as i32)
            });
        vm.complete_system(SystemResult(0x4000 | (count as u16 * 4)))?;
        if blocked {
            return Ok(true);
        }
        let target = std::array::from_fn(|i| base[i].wrapping_add(delta[i]));
        let group = self.auxiliary_texts[slot].as_mut().expect("validated");
        group.start_motion(PositionMotion {
            mode,
            base,
            target,
            duration,
            start: self.auxiliary_clock,
            extra,
        });
        Ok(true)
    }

    /// Auxiliary objects update after ordinary scene/window objects: the motion, then the
    /// glyphs at the new position. The wrapping millisecond clock advances once per frame,
    /// independently of VM query retries.
    pub fn advance_auxiliary_motion(
        &mut self,
        delta: u32,
        scene: &mut SceneHost,
    ) -> Result<(), FormatError> {
        let now = self.auxiliary_clock.wrapping_add(delta);
        let mut updates = Vec::new();
        for (slot, group) in self
            .auxiliary_texts
            .iter()
            .enumerate()
            .filter_map(|(i, g)| g.as_ref().map(|g| (i, g)))
        {
            let Some(motion) = group.motion() else {
                continue;
            };
            if let Some(target) = group.associated_scene
                && scene.object(target).and_then(|o| o.draw.as_ref()).is_none()
            {
                return Err(FormatError(
                    "auxiliary motion associated scene object has no draw state",
                ));
            }
            let elapsed = now.wrapping_sub(motion.start);
            let mut active = elapsed < motion.duration;
            let position: [u32; 2] = std::array::from_fn(|i| {
                if !active {
                    return motion.target[i];
                }
                let distance = motion.target[i].wrapping_sub(motion.base[i]) as i32;
                let offset = if motion.mode == 0 {
                    (elapsed.wrapping_mul(distance as u32) as i32) / motion.duration as i32
                } else {
                    let angle = (elapsed.wrapping_mul(9000) as i32) / motion.duration as i32;
                    let value = (f64::from(angle) / 18000.0 * std::f64::consts::PI).sin()
                        * f64::from(distance.wrapping_mul(100));
                    let integer = if !(-2147483648.0..2147483648.0).contains(&value) {
                        i32::MIN
                    } else {
                        value as i32
                    };
                    integer / 100
                };
                motion.base[i].wrapping_add(offset as u32)
            });
            let clamped = group.clamp_position(position[0], position[1]);
            if position
                .into_iter()
                .zip([(152, 156), (160, 164)])
                .any(|(value, (lo, hi))| {
                    (value as i32) < group.word(lo).expect("bound") as i32
                        || (value as i32) > group.word(hi).expect("bound") as i32
                })
            {
                active = false;
            }
            updates.push((slot, clamped, active));
        }
        self.auxiliary_clock = now;
        for (slot, position, active) in updates {
            let group = self.auxiliary_texts[slot].as_mut().expect("planned");
            group.commit_motion_position(position, active);
            if let Some(target) = group.associated_scene {
                let draw = scene
                    .object_mut(target)
                    .expect("validated")
                    .draw
                    .as_mut()
                    .expect("validated");
                draw.words[8..10].copy_from_slice(&group.position());
            }
        }
        for group in self.auxiliary_texts.iter_mut().flatten() {
            group.attach_glyphs()?;
        }
        Ok(())
    }
}

//! Engine +4640 strip wipe (0x40-byte object, constructor 0x140052700, layout 0x140052740);
//! addresses refer to the documented binary hash. The strips are solid-color objects in the
//! +6152 attached list of an owned 0x14003a3c0 object (+56), with a 20-byte record each in
//! the +40 table. The frame update 0x140015900 runs 0x140052e20 after the fade (+4616) and
//! before the mask (+4632), freeing the wipe when it reports the end. 0x14003de80 queues
//! attached objects by their own layer even though the owner has no draw state; that
//! software renderer draws these verified solid rectangles in the same layer queue.
use super::{DrawState, SceneHost, SceneObject};
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    fade::SkipGate,
    input::ActionInput,
    machine::{Machine, Step},
    message::RESEARCH_SCREEN,
    vm::SystemResult,
};

/// One +40 table record and its attached object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    /// Record +0: attached object id from the owner's +6160 counter (constructor 0).
    pub id: u32,
    /// Record +4/+8: position at creation.
    pub base: [u32; 2],
    /// Record +12/+16: full strip width/height.
    pub full: [u32; 2],
    /// +6248 flags 0 (0x14003a710); words 2..6 source 0, 0, width, height and word 14
    /// color | 0xff000000 (0x14003b150); 8..10 position; 10 layer.
    pub object: SceneObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripWipe {
    /// +4, SP-4: 0 grows columns and 2 rows to full size, covering the screen; 10 and 12
    /// shrink full columns or rows towards their far edge, revealing it. Strip i starts once
    /// the progress exceeds i, so the strips follow one another in screen order.
    pub mode: u32,
    /// +8, SP-8.
    pub color: u32,
    /// +12, SP-12: strip width (modes 0/10) or height (2/12).
    pub size: u32,
    /// +16, SP-20: layer of the strips and of the final cover.
    pub layer: u32,
    /// +20/+24: device width/height (0x140027000/0x140027010), the research viewport.
    pub screen: [u32; 2],
    /// +28, SP-16.
    pub duration: u32,
    /// +32: unclamped accumulated frame milliseconds.
    pub elapsed: u32,
    /// +0: set by 0x140052d70.
    pub finished: bool,
    /// +56 object. 0x140052d70 gives a finished covering wipe a full-screen draw state of
    /// the color (0x14003c7a0, 0x14003b150, position 0, the layer).
    pub object: SceneObject,
    /// +40/+48 table and the attached list, in creation order.
    pub strips: Vec<Strip>,
}

fn solid(color: u32, size: [u32; 2], position: [u32; 2], layer: u32) -> DrawState {
    let mut draw = DrawState::default();
    draw.words[2..6].copy_from_slice(&[0, 0, size[0], size[1]]);
    draw.words[14] = color | 0xff00_0000;
    draw.words[8..10].copy_from_slice(&position);
    draw.words[10] = layer;
    draw
}

impl StripWipe {
    /// 0x140052700 then 0x140052740. Only the four layouts with strips are accepted: other
    /// modes leave the +48 count uninitialized for the 0x6e..0x71 update branches (built by
    /// 0x2247 instead) and are not used with 0x222f by this title. A zero size divides by
    /// zero in the original.
    fn new(args: [u32; 5]) -> Result<Self, &'static str> {
        let [mode, color, size, duration, layer] = args;
        if !matches!(mode, 0 | 2 | 10 | 12) {
            return Err("strip wipe mode is not verified (expected 0, 2, 10 or 12)");
        }
        if size == 0 {
            return Err("strip wipe size 0 divides by zero");
        }
        let screen = RESEARCH_SCREEN;
        let columns = matches!(mode, 0 | 10);
        let span = if columns { screen[0] } else { screen[1] };
        let count = span / size + u32::from(span % size != 0);
        let mut offset = 0u32;
        let strips = (0..count)
            .map(|id| {
                let (base, full) = if columns {
                    ([offset, 0], [size, screen[1]])
                } else {
                    ([0, offset], [screen[0], size])
                };
                let start = match mode {
                    0 => [0, screen[1]],
                    2 => [screen[0], 0],
                    _ => full,
                };
                offset = offset.wrapping_add(size);
                Strip {
                    id,
                    base,
                    full,
                    object: SceneObject {
                        draw: Some(solid(color, start, base, layer)),
                        flags: 0,
                        ..SceneObject::default()
                    },
                }
            })
            .collect();
        Ok(Self {
            mode,
            color,
            size,
            layer,
            screen,
            duration,
            elapsed: 0,
            finished: false,
            object: SceneObject::default(),
            strips,
        })
    }

    /// 0x140052d70: frees the strips and their table, marks the wipe finished and returns
    /// whether to free it; modes below 10 keep a full-screen cover instead.
    fn finish(&mut self) -> bool {
        self.strips.clear();
        self.finished = true;
        if self.mode < 10 {
            self.object.draw = Some(solid(self.color, self.screen, [0, 0], self.layer));
            false
        } else {
            true
        }
    }

    /// 0x140052e20: true when the wipe ends and is to be freed. Progress is
    /// elapsed × (count + size) / duration in wrapping u32 (imul, div); strip i takes
    /// min(progress − i, size) once progress exceeds i.
    fn advance(&mut self, delta: u32) -> bool {
        if self.finished {
            return false;
        }
        self.elapsed = self.elapsed.wrapping_add(delta);
        if self.elapsed >= self.duration {
            return self.finish();
        }
        let count = self.strips.len() as u32;
        let progress = self.elapsed.wrapping_mul(count.wrapping_add(self.size)) / self.duration;
        for (i, strip) in (0u32..).zip(&mut self.strips) {
            if progress <= i {
                continue;
            }
            let n = (progress - i).min(self.size);
            let words = &mut strip
                .object
                .draw
                .as_mut()
                .expect("strips are built with a draw state")
                .words;
            let (size, position) = match self.mode {
                0 => ([n, self.screen[1]], None),
                2 => ([self.screen[0], n], None),
                10 => (
                    [self.size - n, self.screen[1]],
                    Some([strip.base[0].wrapping_add(n), strip.base[1]]),
                ),
                _ => (
                    [self.screen[0], self.size - n],
                    Some([strip.base[0], strip.base[1].wrapping_add(n)]),
                ),
            };
            words[4..6].copy_from_slice(&size);
            if let Some(position) = position {
                words[8..10].copy_from_slice(&position);
            }
        }
        false
    }

    /// 0x140052e00: −1 once finished, else the time left (0 when overdue).
    pub fn remaining(&self) -> u32 {
        if self.finished {
            u32::MAX
        } else {
            self.duration.saturating_sub(self.elapsed)
        }
    }
}

impl SceneHost {
    pub fn strip_wipe(&self) -> Option<&StripWipe> {
        self.strip.as_ref()
    }

    /// The +4640 part of the frame update 0x140015900.
    pub(super) fn advance_strip(&mut self, delta: u32) {
        if self.strip.as_mut().is_some_and(|wipe| wipe.advance(delta)) {
            self.strip = None;
        }
    }

    /// 0x2230 / 0x14007aa30 and the skip branch of 0x2233.
    fn finish_strip(&mut self) {
        if self.strip.as_mut().is_some_and(StripWipe::finish) {
            self.strip = None;
        }
    }

    /// 0x222f / 0x14007a7a0: frees the wipe (0x140053380), builds one from SP-4 mode, SP-8
    /// color, SP-12 size, SP-16 duration and SP-20 layer, sets system result 0 to 1 and
    /// returns 0x4014. 0x2230 / 0x14007aa30 finishes it; 0x2231 / 0x14007a9e0 frees it;
    /// 0x2232 / 0x14007aaa0 sets system result 0 to the time left, −1 without a wipe. These
    /// three return 0x4000 and pop nothing. 0x2247 / 0x14007a8a0 builds a wipe from a slot
    /// object instead and stays unhandled.
    pub fn try_handle_strip(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ 0x22f..=0x232)) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        match command {
            0x22f => {
                let mut args = [0; 5];
                for (i, value) in args.iter_mut().enumerate() {
                    *value = vm
                        .stack()
                        .peek_u32((i + 1) * 4)
                        .map_err(|e| error(pc, op, e.0))?;
                }
                let wipe = StripWipe::new(args).map_err(|why| error(pc, op, why))?;
                vm.complete_system(SystemResult(0x4014))?;
                self.strip = Some(wipe);
                vm.state_mut()
                    .set_system_integer(0, 1)
                    .expect("slot 0 exists");
            }
            0x230 => {
                vm.complete_system(SystemResult(0x4000))?;
                self.finish_strip();
            }
            0x231 => {
                vm.complete_system(SystemResult(0x4000))?;
                self.strip = None;
            }
            _ => {
                vm.complete_system(SystemResult(0x4000))?;
                let left = self.strip.as_ref().map_or(u32::MAX, StripWipe::remaining);
                vm.state_mut()
                    .set_system_integer(0, left)
                    .expect("slot 0 exists");
            }
        }
        Ok(true)
    }

    /// 0x2233 / 0x14007aae0: system result 0 = 0; without a wipe, or once it is finished
    /// (time left negative as i32), 0x4004. With SP-4 nonzero the wait ends at once when
    /// effects are off, action 7 or 8 is held or the skip flag is set, or on a released
    /// action 0, which is consumed: the wipe finishes as for 0x2230, result 0 = 1, 0xc004.
    /// Otherwise 0xa000 (always for SP-4 zero) until the frame update finishes it.
    pub fn try_wait_strip(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x233)) = vm.pending_system() else {
            return Ok(None);
        };
        let skippable = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2233, e.0))? != 0;
        let active = self
            .strip
            .as_ref()
            .is_some_and(|wipe| (wipe.remaining() as i32) >= 0);
        if !active {
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
            self.finish_strip();
            return Ok(Some(step));
        }
        let step = vm.yield_pending_system()?;
        vm.state_mut().set_system_integer(0, 0).expect("slot 0");
        Ok(Some(step))
    }
}

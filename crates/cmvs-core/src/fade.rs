//! Engine +4616 full-screen color fade; addresses refer to the documented binary hash.
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow},
    evaluate::error,
    input::ActionInput,
    machine::Machine,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fade {
    /// 0 increases coverage, 1 decreases coverage.
    pub direction: u32,
    pub color: u32,
    pub duration: u32,
    pub order: i32,
    pub elapsed: u32,
    pub finished: bool,
    pub opacity: i32,
}
impl Fade {
    fn finish(&mut self) -> bool {
        self.opacity = if self.direction == 0 { 255 } else { 0 };
        self.finished = true;
        self.direction == 1
    }
    /// 0x140051010: unsigned wrapping arithmetic, including the 255 * elapsed product.
    fn advance(&mut self, delta: u32) -> bool {
        if self.finished {
            return false;
        }
        self.elapsed = self.elapsed.wrapping_add(delta);
        if self.elapsed >= self.duration {
            return self.finish();
        }
        self.set_opacity();
        false
    }
    pub fn remaining(&self) -> u32 {
        if self.finished {
            u32::MAX
        } else {
            self.duration.saturating_sub(self.elapsed)
        }
    }
    /// Record 0x610 (0x140050ec0): direction, color, duration and order for the reader's
    /// constructor, then the object words +0..+24: finished, direction, color, order,
    /// duration, elapsed. The overlay object at +24 is not saved.
    pub(crate) fn saved(&self) -> Vec<u8> {
        let order = self.order as u32;
        [
            self.direction,
            self.color,
            self.duration,
            order,
            u32::from(self.finished),
            self.direction,
            self.color,
            order,
            self.duration,
            self.elapsed,
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect()
    }
    /// Reader: 0x140050da0 and 0x140050f00 rebuild the overlay, copy the object words back
    /// and set the opacity from them (full or clear when finished). An unfinished
    /// zero-length fade would divide by zero there and is rejected.
    pub(crate) fn restore(record: &[u8]) -> Result<Self, &'static str> {
        if record.len() != 40 {
            return Err("fade record length differs from the original");
        }
        let w: Vec<u32> = record
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        if [w[0], w[1], w[2], w[3]] != [w[5], w[6], w[8], w[7]] {
            return Err("fade record header disagrees with its object words");
        }
        if w[4] > 1 || w[5] > 1 || w[4] == 0 && w[8] == 0 {
            return Err("saved fade state is not supported");
        }
        let mut fade = Self {
            direction: w[5],
            color: w[6],
            duration: w[8],
            order: w[7] as i32,
            elapsed: w[9],
            finished: w[4] != 0,
            opacity: 0,
        };
        if fade.finished {
            fade.opacity = if fade.direction == 0 { 255 } else { 0 };
        } else {
            fade.set_opacity();
        }
        Ok(fade)
    }
    fn set_opacity(&mut self) {
        let value = 255u32.wrapping_mul(self.elapsed) / self.duration;
        let value = if self.direction == 0 {
            value
        } else {
            255u32.wrapping_sub(value)
        };
        self.opacity = (value as i32).min(255);
    }
}

/// Engine settings 0x2224's skippable branch (0x14007a6c0) reads besides input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkipGate {
    /// +1508, EFFECT_USE (constructor 1); when false a skippable wait ends at once.
    pub effects: bool,
    /// +1520, the script skip flag of 0x22eb.
    pub skip: bool,
}
impl Default for SkipGate {
    fn default() -> Self {
        Self {
            effects: true,
            skip: false,
        }
    }
}

/// 0x22d1 / 0x14007bcf0: nonzero SP-4 sets EFFECT_USE (+1508) to bool(SP-8) through
/// 0x140091040, which also checks popup menu item 32773 (no native menu is modeled); zero
/// SP-4 copies it to system result 0 without reading SP-8. Both pop 8 bytes.
pub fn try_handle_effects(vm: &mut Machine<'_>, effects: &mut bool) -> Result<bool, DecodeError> {
    let Some((pc, 0x2d1)) = vm.pending_system() else {
        return Ok(false);
    };
    handle_effects(&mut Call::new(vm, pc, 0x2d1), effects)?;
    Ok(true)
}

pub(crate) fn handle_effects(
    call: &mut Call<'_, '_>,
    effects: &mut bool,
) -> Result<(), DecodeError> {
    let mode = call.argument(4, "mode")?;
    let value = if mode != 0 {
        Some(call.argument(8, "value")? != 0)
    } else {
        call.ignore_argument(8, "unused value")?;
        None
    };
    call.complete(0x4008)?;
    match value {
        Some(value) => *effects = value,
        None => call
            .vm
            .state_mut()
            .set_system_integer(0, u32::from(*effects))
            .expect("slot 0 exists"),
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct FadeHost {
    current: Option<Fade>,
}
impl FadeHost {
    /// 0x2224 / 0x14007a6c0: SP-4 nonzero makes an active wait skippable. It ends at once
    /// when effects are off, input action 7 or 8 is held (0x140054e20/0x140054e30) or the
    /// skip flag is set; otherwise a released action 0 (0x140054d60) is consumed
    /// (0x140054d90) and ends it. Ending forces the fade like 0x2222 (0x14007a610), sets
    /// system result 0 to 1 and returns 0xc004; waiting returns 0xa000 with result 0.
    pub fn try_wait(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<crate::machine::Step>, DecodeError> {
        let Some((pc, 0x224)) = vm.pending_system() else {
            return Ok(None);
        };
        self.handle_wait(&mut Call::new(vm, pc, 0x224), gate, input)
            .map(|flow| Some(flow.wait_step()))
    }

    pub(crate) fn handle_wait(
        &mut self,
        call: &mut Call<'_, '_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Flow, DecodeError> {
        let skippable = call.argument(4, "skippable")? != 0;
        let active = self
            .current
            .as_ref()
            .is_some_and(|f| (f.remaining() as i32) >= 0);
        if !active {
            let flow = call.finish(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0");
            return Ok(flow);
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let decided = skippable && !immediate && input.released(0);
        if skippable && (immediate || decided) {
            let flow = call.finish(0xc004)?;
            if decided {
                input.consume(0);
            }
            call.vm
                .state_mut()
                .set_system_integer(0, 1)
                .expect("slot 0");
            if self.current.as_mut().is_some_and(Fade::finish) {
                self.current = None;
            }
            return Ok(flow);
        }
        let flow = call.wait(4)?;
        call.vm
            .state_mut()
            .set_system_integer(0, 0)
            .expect("slot 0");
        Ok(flow)
    }
    pub fn current(&self) -> Option<&Fade> {
        self.current.as_ref()
    }
    pub(crate) fn restored(current: Option<Fade>) -> Self {
        Self { current }
    }
    /// Explicit engine-frame delta; VM instruction execution never advances time.
    pub fn advance(&mut self, delta: u32) {
        if self
            .current
            .as_mut()
            .is_some_and(|fade| fade.advance(delta))
        {
            self.current = None;
        }
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if !matches!(command, 0x220..=0x223) {
            return Ok(false);
        }
        self.handle(&mut Call::new(vm, pc, command))?;
        Ok(true)
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);

        let fail = |why| error(pc, command + 0x2000, why);
        let next = if command == 0x220 {
            let mut args = [0; 4];
            for (i, value) in args.iter_mut().enumerate() {
                *value = call.argument((i + 1) * 4, fade_argument_names(call.command)[i])?;
            }
            if args[0] > 1 {
                return Err(fail("unsupported fade direction (expected 0 or 1)"));
            }
            Some(Fade {
                direction: args[0],
                color: args[1],
                duration: args[2],
                order: args[3] as i32,
                elapsed: 0,
                finished: false,
                opacity: if args[0] == 0 { 0 } else { 255 },
            })
        } else {
            None
        };
        let remaining = self.current.as_ref().map_or(u32::MAX, Fade::remaining);
        call.complete(if command == 0x220 { 0x4010 } else { 0x4000 })?;
        match command {
            0x220 => {
                self.current = next;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 1)
                    .expect("slot 0");
            }
            0x221 => self.current = None,
            0x222 => {
                if self.current.as_mut().is_some_and(Fade::finish) {
                    self.current = None;
                }
            }
            0x223 => call
                .vm
                .state_mut()
                .set_system_integer(0, remaining)
                .expect("slot 0"),
            _ => unreachable!(),
        }
        Ok(())
    }
}

fn fade_argument_names(_: u16) -> &'static [&'static str] {
    &["direction", "color", "duration", "order"]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fade(direction: u32, elapsed: u32, finished: bool) -> Fade {
        Fade {
            direction,
            color: 0x0012_3456,
            duration: 400,
            order: -3,
            elapsed,
            finished,
            opacity: 0,
        }
    }

    #[test]
    fn fade_records_round_trip_and_derive_the_overlay_opacity() {
        for (direction, elapsed, finished, opacity) in [
            (0, 100, false, 63),
            (1, 100, false, 192),
            (0, 400, true, 255),
            (1, 400, true, 0),
        ] {
            let saved = fade(direction, elapsed, finished).saved();
            assert_eq!(saved.len(), 40);
            let restored = Fade::restore(&saved).unwrap();
            let mut expected = fade(direction, elapsed, finished);
            expected.opacity = opacity;
            assert_eq!(restored, expected);
        }
    }

    #[test]
    fn fade_records_reject_inconsistent_or_crashing_states() {
        let saved = fade(0, 100, false).saved();
        for (at, value) in [(0, 1), (16, 2), (20, 2)] {
            let mut bad = saved.clone();
            bad[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
            assert!(Fade::restore(&bad).is_err(), "offset {at}");
        }
        // An unfinished zero-length fade divides by zero in the reader.
        let mut zero = fade(0, 0, false);
        zero.duration = 0;
        assert!(Fade::restore(&zero.saved()).is_err());
        assert!(Fade::restore(&saved[..36]).is_err());
    }
}

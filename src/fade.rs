//! Engine +4616 full-screen color fade; addresses refer to the documented binary hash.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

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
        let value = 255u32.wrapping_mul(self.elapsed) / self.duration;
        let value = if self.direction == 0 {
            value
        } else {
            255u32.wrapping_sub(value)
        };
        self.opacity = (value as i32).min(255);
        false
    }
    pub fn remaining(&self) -> u32 {
        if self.finished {
            u32::MAX
        } else {
            self.duration.saturating_sub(self.elapsed)
        }
    }
}

#[derive(Debug, Default)]
pub struct FadeHost {
    current: Option<Fade>,
}
impl FadeHost {
    /// 0x2224: only the no-input branch is implemented while a fade is active.
    /// A nonzero skip flag stays unhandled until a verified input adapter is supplied.
    pub fn try_wait(
        &self,
        vm: &mut Machine<'_>,
    ) -> Result<Option<crate::machine::Step>, DecodeError> {
        let Some((pc, 0x224)) = vm.pending_system() else {
            return Ok(None);
        };
        let skip = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2224, e.0))?;
        let active = self
            .current
            .as_ref()
            .is_some_and(|f| (f.remaining() as i32) >= 0);
        if active && skip != 0 {
            return Ok(None);
        }
        let step = if active {
            vm.yield_pending_system()?
        } else {
            vm.complete_system(SystemResult(0x4004))?
        };
        vm.state_mut().set_system_integer(0, 0).expect("slot 0");
        Ok(Some(step))
    }
    pub fn current(&self) -> Option<&Fade> {
        self.current.as_ref()
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
        let Some((pc, command @ 0x220..=0x223)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let next = if command == 0x220 {
            let mut args = [0; 4];
            for (i, value) in args.iter_mut().enumerate() {
                *value = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
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
        vm.complete_system(SystemResult(if command == 0x220 { 0x4010 } else { 0x4000 }))?;
        match command {
            0x220 => {
                self.current = next;
                vm.state_mut().set_system_integer(0, 1).expect("slot 0");
            }
            0x221 => self.current = None,
            0x222 => {
                if self.current.as_mut().is_some_and(Fade::finish) {
                    self.current = None;
                }
            }
            0x223 => vm
                .state_mut()
                .set_system_integer(0, remaining)
                .expect("slot 0"),
            _ => unreachable!(),
        }
        Ok(true)
    }
}

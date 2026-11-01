//! Engine +4624 screen quake (0x28-byte object, constructor 0x1400522b0); addresses refer to
//! the documented binary hash. The render pass 0x140015da0 advances it once per frame after
//! queuing everything and frees it when 0x140052380 reports the end; the draw pass
//! 0x140015fc0 offsets queue items through 0x1400526b0, in back-buffer pixels (the research
//! viewport here), for objects whose +6248 bit 0x100 is set (a child uses its parent's).
use super::SceneHost;
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow},
    evaluate::error,
    machine::{Machine, Step},
    vm::SystemResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quake {
    /// +0, SP-4. Low nibble: 0 shakes every queue item, 1 only slot objects, 2 only the
    /// message windows and text objects. Bits 4..7: direction (0 horizontal, 0x10
    /// vertical, 0x30 rotating corners). Bits 8..11: waveform (0 steps, 0x200 damped cosine).
    pub mode: u32,
    /// +4, SP-8.
    pub amplitude: i32,
    /// +8, SP-12: step period, or the whole duration of the damped cosine.
    pub period: u32,
    /// +12, SP-16: steps or cosine cycles; u32::MAX never ends a step quake.
    pub count: u32,
    /// +20, 0x2227.
    pub paused: bool,
    /// +24: unclamped accumulated milliseconds (steps subtract whole periods).
    pub elapsed: u32,
    /// +28: completed steps or cycles.
    pub steps: u32,
    /// +32/+36.
    pub offset: [i32; 2],
}

/// Record 0x600 length (writer 0x1400522e0).
pub const SAVED_QUAKE: usize = 56;

impl Quake {
    /// Record 0x600: the constructor words +0..+12, then the whole 0x28-byte object.
    /// +16 is zeroed by the constructor 0x1400522b0 and written by no implemented command.
    pub(crate) fn saved(&self) -> Vec<u8> {
        let object = [
            self.mode,
            self.amplitude as u32,
            self.period,
            self.count,
            0,
            u32::from(self.paused),
            self.elapsed,
            self.steps,
            self.offset[0] as u32,
            self.offset[1] as u32,
        ];
        let mut out = Vec::with_capacity(SAVED_QUAKE);
        for word in object[..4].iter().chain(&object) {
            out.extend(word.to_le_bytes());
        }
        out
    }
    /// Reader: 0x1400522b0 with the first four words, then 0x140052330 copies the object.
    pub(crate) fn restore(record: &[u8]) -> Result<Self, &'static str> {
        if record.len() != SAVED_QUAKE {
            return Err("quake record has unexpected length");
        }
        let word =
            |i: usize| u32::from_le_bytes(record[16 + 4 * i..20 + 4 * i].try_into().unwrap());
        if word(4) != 0 || word(5) > 1 {
            return Err("saved quake state is not modeled");
        }
        Ok(Self {
            mode: word(0),
            amplitude: word(1) as i32,
            period: word(2),
            count: word(3),
            paused: word(5) != 0,
            elapsed: word(6),
            steps: word(7),
            offset: [word(8) as i32, word(9) as i32],
        })
    }
}

impl Quake {
    /// 0x1400526b0: the offset for queue items enqueued as message windows/text (`ui`) or
    /// as slot objects; zero while paused or for the layer the low nibble excludes.
    pub fn offset(&self, ui: bool) -> [i32; 2] {
        let excluded = if ui { 1 } else { 2 };
        if self.paused || self.mode & 0xf == excluded {
            [0, 0]
        } else {
            self.offset
        }
    }

    /// 0x140052380: true once the quake ends. Paused quakes keep their time.
    fn advance(&mut self, delta: u32) -> bool {
        if self.paused {
            return false;
        }
        self.elapsed = self.elapsed.wrapping_add(delta);
        if self.mode & 0xf00 == 0x200 {
            return self.damped();
        }
        let mut done = false;
        while self.elapsed >= self.period {
            self.elapsed -= self.period;
            let a = self.amplitude;
            let [x, y] = &mut self.offset;
            match self.mode & 0xf0 {
                0x00 => *x = if *x == 0 { a } else { x.wrapping_neg() },
                0x10 => *y = if *y == 0 { a } else { y.wrapping_neg() },
                0x30 => {
                    let neg = a.wrapping_neg();
                    [*x, *y] = match self.steps & 3 {
                        0 => [neg, neg],
                        1 => [a, a],
                        2 => [a, neg],
                        _ => [neg, a],
                    };
                }
                // 0x20 is rejected at creation; others leave the offset.
                _ => {}
            }
            // Compared before counting this step, so count + 2 steps run.
            if self.count != u32::MAX && self.count < self.steps {
                done = true;
            }
            self.steps = self.steps.wrapping_add(1);
        }
        done
    }

    /// The 0x200 waveform: the phase angle is (count × elapsed × 360) / duration in u32,
    /// the cosine of (angle + 90) mod 360 degrees scaled by the amplitude is truncated
    /// (0x1400089b0, f64), then damped by the remaining time with signed division.
    fn damped(&mut self) -> bool {
        let (value, done) = if self.period <= self.elapsed {
            self.steps = self.count;
            (0, true)
        } else {
            let angle = self.count.wrapping_mul(self.elapsed).wrapping_mul(360) / self.period;
            self.steps = angle / 360;
            let degrees = angle.wrapping_add(90) % 360;
            let radians = if degrees == 0 {
                0.0
            } else {
                f64::from(degrees) / 180.0 * std::f64::consts::PI
            };
            let cosine = (radians.cos() * f64::from(self.amplitude)) as i32;
            let remaining = self.period.wrapping_sub(self.elapsed) as i32;
            (
                remaining
                    .wrapping_mul(cosine)
                    .wrapping_div(self.period as i32),
                false,
            )
        };
        match self.mode & 0xf0 {
            0x00 => self.offset = [value, 0],
            0x10 => self.offset = [0, value],
            _ => {}
        }
        done
    }
}

impl SceneHost {
    pub fn quake(&self) -> Option<&Quake> {
        self.quake.as_ref()
    }

    /// The +4624 part of the render pass 0x140015da0, after the slot, text and window queues.
    pub(super) fn advance_quake(&mut self, delta: u32) {
        if self
            .quake
            .as_mut()
            .is_some_and(|quake| quake.advance(delta))
        {
            self.quake = None;
        }
    }

    /// 0x2225 / 0x14007abc0: replaces the quake with SP-4 mode, SP-8 amplitude, SP-12 period,
    /// SP-16 count (offset and time zero); returns 0x4010. 0x2226 / 0x14007ac80: system result
    /// 0 = count − steps, or −1 for an endless or missing quake. 0x2227 / 0x14007acc0: pauses
    /// while SP-4 is nonzero; pops 4. 0x2229 / 0x14007adf0: frees the quake. 0x2226 and
    /// 0x2229 return 0x4000. Waveforms the scripts never use stay unmodeled errors: 0x100 and
    /// step direction 0x20, whose rand() shares the CRT state with 0x20d4.
    pub fn try_handle_quake(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x20c | 0x225..=0x227 | 0x229))) = vm.pending_system() else {
            return Ok(false);
        };
        if command == 0x20c {
            // 0x14007ad00: system result 0 = a quake exists and is paused (+20,
            // 0x140052370); 0x4000.
            let paused = self.quake.as_ref().is_some_and(|q| q.paused);
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(paused))
                .expect("slot 0 exists");
            return Ok(true);
        }
        let op = command + 0x2000;
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        match command {
            0x225 => {
                let (mode, amplitude, period, count) = (arg(4)?, arg(8)?, arg(12)?, arg(16)?);
                match mode & 0xf00 {
                    0x200 => {}
                    0 if mode & 0xf0 == 0x20 => {
                        return Err(error(
                            pc,
                            op,
                            "random quake shares the unmodeled CRT rand state",
                        ));
                    }
                    0 => {}
                    _ => return Err(error(pc, op, "quake waveform not used by this title")),
                }
                vm.complete_system(SystemResult(0x4010))?;
                self.quake = Some(Quake {
                    mode,
                    amplitude: amplitude as i32,
                    period,
                    count,
                    paused: false,
                    elapsed: 0,
                    steps: 0,
                    offset: [0, 0],
                });
            }
            0x226 => {
                vm.complete_system(SystemResult(0x4000))?;
                let left = self.quake.as_ref().map_or(u32::MAX, |quake| {
                    if quake.count == u32::MAX {
                        u32::MAX
                    } else {
                        quake.count.wrapping_sub(quake.steps)
                    }
                });
                vm.state_mut()
                    .set_system_integer(0, left)
                    .expect("slot 0 exists");
            }
            0x227 => {
                let paused = arg(4)? != 0;
                vm.complete_system(SystemResult(0x4004))?;
                if let Some(quake) = &mut self.quake {
                    quake.paused = paused;
                }
            }
            _ => {
                vm.complete_system(SystemResult(0x4000))?;
                self.quake = None;
            }
        }
        Ok(true)
    }

    /// 0x2228 / 0x14007ad50: without a quake system result 0 = 1 and 0x4004. With one, SP-4
    /// nonzero waits (result 0, 0xa000) until the render pass frees it; SP-4 zero frees it
    /// at once (result 1, 0xc004). No input skips this wait.
    pub fn try_wait_quake(&mut self, vm: &mut Machine<'_>) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x228)) = vm.pending_system() else {
            return Ok(None);
        };
        self.handle_wait_quake(&mut Call::new(vm, pc, 0x228))
            .map(|flow| Some(flow.wait_step()))
    }

    pub(crate) fn handle_wait_quake(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<Flow, DecodeError> {
        let wait = call.argument(4, "wait")? != 0;
        let (flow, result) = match &self.quake {
            None => (call.finish(0x4004)?, 1),
            Some(_) if wait => (call.wait(4)?, 0),
            Some(_) => {
                let flow = call.finish(0xc004)?;
                self.quake = None;
                (flow, 1)
            }
        };
        call.vm
            .state_mut()
            .set_system_integer(0, result)
            .expect("slot 0 exists");
        Ok(flow)
    }
}

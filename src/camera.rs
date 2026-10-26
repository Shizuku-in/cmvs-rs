//! Screen cameras at engine +1968 -> +16 and +24 (0xa0 bytes each, constructor 0x14004fab0);
//! addresses refer to the documented binary hash. 0x2058..0x205f/0x2066/0x2067/0x209c set
//! fields of the camera selected by SP-4 (0x140027090), 0x2060..0x2065 the same fields of camera 0
//! (0x140027070). Fields stay raw words named by offset: the original only reads them for
//! coordinate modes 2/3 (0x140020340 -> 0x14004ef40), which the renderer rejects, so they
//! do not affect the supported 2D output. Eased moves (+84) are not modeled.
use crate::{
    bytecode::DecodeError,
    evaluate::{error, truncate},
    machine::Machine,
    vm::SystemResult,
};

pub const CAMERAS: usize = 2;
pub const CAMERA_WORDS: usize = 40;

/// Word index of +84, set while an eased move runs (0x14004f5b0..0x14004f9a0).
const MOVING: usize = 21;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Camera {
    pub words: [u32; CAMERA_WORDS],
}

impl Default for Camera {
    /// 0x14004fab0 writes every word; unlisted ones are zero.
    fn default() -> Self {
        let mut words = [0; CAMERA_WORDS];
        for (index, value) in [
            (2, 3.0f32),
            (6, 9.0),
            (7, 9.0),
            (8, 6.0),
            (9, 1024.0),
            (10, 612.0),
            (11, 24.0),
            (15, 512.0),
            (16, 306.0),
            (17, 1.0),
            (18, 1.0),
        ] {
            words[index] = value.to_bits();
        }
        words[0] = 1;
        Self { words }
    }
}

impl Camera {
    pub fn float(&self, index: usize) -> f32 {
        f32::from_bits(self.words[index])
    }
    fn set(&mut self, index: usize, value: f32) {
        self.words[index] = value.to_bits();
    }
}

/// `(float)(int)(v * 100) / 100` as 0x14004ed10 computes it with cvttss2si.
fn hundredths(value: f32) -> f32 {
    truncate(value * 100.0) as f32 / 100.0
}

#[derive(Debug, Default)]
pub struct CameraHost {
    cameras: [Camera; CAMERAS],
}

impl CameraHost {
    pub fn camera(&self, index: usize) -> Option<&Camera> {
        self.cameras.get(index)
    }
    /// Camera 0 +84: an eased move is running. No modeled command starts one.
    pub fn moving(&self) -> bool {
        self.cameras[0].words[MOVING] != 0
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x58..=0x68 | 0x9c))) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        // Commands 0x2060..0x2065 address camera 0 directly and take one argument less.
        let indexed = !(0x60..=0x65).contains(&command) && command != 0x68;
        let values = match command {
            0x58 | 0x5a | 0x60 | 0x62 => 3,
            0x59 | 0x5d | 0x61 | 0x65 | 0x67 => 2,
            _ => 1,
        };
        let count = values + usize::from(indexed);
        let mut args = Vec::with_capacity(count);
        for i in 1..=count {
            args.push(vm.stack().peek_u32(i * 4).map_err(|e| fail(e.0))?);
        }
        if command == 0x68 {
            // 0x2068 / 0x14006fd50 -> 0x14004f550 ends camera 0's move; nothing runs here.
            if self.moving() {
                return Err(fail("ending a camera move needs the move parameters"));
            }
            vm.complete_system(SystemResult(0x4004))?;
            return Ok(true);
        }
        let camera = if indexed {
            // 0x140027090 indexes the screen object's pointers without a bound; only
            // indices 0 and 1 are cameras.
            usize::try_from(args.remove(0))
                .ok()
                .filter(|index| *index < CAMERAS)
                .ok_or_else(|| fail("camera index outside 0..1"))?
        } else {
            0
        };
        let f = |i: usize| f32::from_bits(args[i]);
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let target = &mut self.cameras[camera];
        match command {
            // 0x14004ecd0: +28, +32, +24.
            0x58 | 0x60 => {
                target.set(7, f(0));
                target.set(8, f(1));
                target.set(6, f(2));
            }
            // 0x14004ece0: +36, +40 and their halves at +60, +64.
            0x59 | 0x61 => {
                target.set(9, f(0));
                target.set(10, f(1));
                target.set(15, f(0) * 0.5);
                target.set(16, f(1) * 0.5);
            }
            // 0x14004ed10: +4, +8, +12 in hundredths.
            0x5a | 0x62 => {
                for i in 0..3 {
                    target.set(1 + i, hundredths(f(i)));
                }
            }
            // 0x14004ed90: +56 = (|(int)(v * 100)| % 36000) / 100, an angle in degrees.
            0x5b | 0x63 => {
                let centi = truncate(f(0) * 100.0).wrapping_abs();
                target.set(14, (centi % 36000) as f32 / 100.0);
            }
            // 0x14004ede0: +44.
            0x5c | 0x64 => target.set(11, f(0)),
            // 0x14004ed70: +60, +64.
            0x5d | 0x65 => {
                target.set(15, f(0));
                target.set(16, f(1));
            }
            // 0x14002e840 / 0x14004ecb0 / 0x14004edf0: integers at +148, +0, +156.
            0x5e => target.words[37] = args[0],
            // 0x209c / 0x14006f9e0 -> 0x140019f80: integer at +152.
            0x9c => target.words[38] = args[0],
            0x5f => target.words[0] = args[0],
            0x66 => target.words[39] = args[0],
            // 0x14004ed80: +68, +72.
            0x67 => {
                target.set(17, f(0));
                target.set(18, f(1));
            }
            _ => unreachable!("matched above"),
        }
        Ok(true)
    }
}

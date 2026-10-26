//! Engine +4480: three movie slots holding 0x170-byte player objects (constructor
//! 0x1400383c0) that draw CMV6 files into a scene object; addresses refer to the documented
//! binary hash. Only slot destruction (0x230a) is modeled. Creation (0x2308/0x230e) needs
//! CMV decoding through mog2x64.dll and stays unhandled, so slots are always empty here.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const MOVIE_SLOTS: usize = 3;

#[derive(Debug, Default)]
pub struct MovieHost {
    /// The constructor (0x140014fc2..0x140014fd0) stores null in all three pointers.
    occupied: [bool; MOVIE_SLOTS],
}

impl MovieHost {
    pub fn occupied(&self, slot: usize) -> bool {
        self.occupied.get(slot).copied().unwrap_or(false)
    }
    /// 0x230a / 0x14007f390: SP-4 slot compared unsigned against 2; destroys and clears the
    /// slot's player if present and returns 0x4004. Out-of-range slots only set +12056 bit
    /// 0x1000000 in the original, which is not modeled.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x30a)) = vm.pending_system() else {
            return Ok(false);
        };
        let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x230a, e.0))? as usize;
        if slot >= MOVIE_SLOTS {
            return Err(error(
                pc,
                0x230a,
                "movie slot outside 0..2 sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4004))?;
        self.occupied[slot] = false;
        Ok(true)
    }
}

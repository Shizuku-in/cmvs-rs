//! Script-controlled message settings; addresses refer to the documented binary hash.
//! These live in the +1496 settings block that message objects read (e.g. 0x14005e220).
//! The two boolean flags are cleared by the constructor/reset/save loading; no
//! configuration helper writes either boolean. The +1548 mode starts at zero and
//! 0x140092d40 writes it while selecting popup menu items 0x809d..0x809f.
//! The bypass flag is engine +12244, outside that block, but read by the same message waits.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MessageFlags {
    skip: bool,
    auto_advance: bool,
    mode_1548: u32,
    bypass: bool,
}
impl MessageFlags {
    pub fn from_ini(bytes: &[u8]) -> Self {
        Self {
            mode_1548: crate::save_menu::profile_int(bytes, "MESSAGE_INFO", "VOICE_CUT_MODE", 0)
                as u32,
            ..Self::default()
        }
    }
    /// 0x140076700/76880: READ_SKIP clears script skip upon reaching an unread ID.
    /// Return a candidate so a failed message command does not change global flags.
    pub fn prepare_read(
        self,
        state: &crate::evaluate::State,
        id: i32,
        read_skip: bool,
    ) -> Result<(Self, bool), &'static str> {
        let mut candidate = self;
        let mut read = true;
        if read_skip && id >= 0 {
            read = state
                .bit(id as u32)
                .ok_or("message read-skip bit is unknown or outside the modeled bank")?;
            if !read {
                candidate.skip = false;
            }
        }
        if self.bypass {
            read = true;
        }
        Ok((candidate, read))
    }
    /// Loader 0x140081960 clears +1520 (skip) and +1552 (auto advance).
    pub(crate) fn clear_for_load(&mut self) {
        self.skip = false;
        self.auto_advance = false;
    }
    /// Engine +12244: zeroed by the CMainFrameWnd constructor (qword store at 0x140016ff5)
    /// and written only by 0x2340. 0x2152/0x2153/0x215c treat it as read and finish like
    /// the skip flag without consuming input (0x14005d900 a7, 0x14005e220 a8).
    pub fn bypass(&self) -> bool {
        self.bypass
    }
    /// +1548: raw setting read/written by 0x22d0. Normally 0..2, but the original
    /// wrapping abs of i32::MIN produces -2 after signed remainder. VOICE_CUT_MODE;
    /// the game scripts query it to choose text-send, next-voice or fade-out behavior.
    pub fn mode_1548(&self) -> u32 {
        self.mode_1548
    }
    /// +1520, 0x22eb. Also read by 0x2224's input-skip branch and 0x22ef, which need input.
    pub fn skip(&self) -> bool {
        self.skip
    }
    /// +1552, 0x22e5; the 自動送りを実行 checkbox (control 1018) of dialog 138 also edits it.
    pub fn auto_advance(&self) -> bool {
        self.auto_advance
    }
    /// 0x22eb / 0x14007c890 and 0x22e5 / 0x14007c5d0: nonzero SP-4 sets the flag to
    /// bool(SP-8); zero SP-4 copies it to system result 0 without reading SP-8. Both pop 8 bytes.
    /// 0x2340 (inline 0x1400847fa) sets the bypass to bool(SP-4) and pops 4 bytes; it has no
    /// query form and leaves the system results alone.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if let Some((pc, 0x340)) = vm.pending_system() {
            let value = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2340, e.0))?;
            vm.complete_system(SystemResult(0x4004))?;
            self.bypass = value != 0;
            return Ok(true);
        }
        if let Some((pc, 0x2d0)) = vm.pending_system() {
            let mode = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22d0, e.0))?;
            let value = if mode != 0 {
                let raw = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x22d0, e.0))?;
                Some(((raw as i32).wrapping_abs() % 3) as u32)
            } else {
                None
            };
            vm.complete_system(SystemResult(0x4008))?;
            if let Some(value) = value {
                self.mode_1548 = value;
            } else {
                vm.state_mut()
                    .set_system_integer(0, self.mode_1548)
                    .expect("slot 0 exists");
            }
            return Ok(true);
        }
        let Some((pc, command @ (0x2eb | 0x2e5))) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, command + 0x2000, e.0))
        };
        let value = if arg(4)? != 0 {
            Some(arg(8)? != 0)
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4008))?;
        let flag = if command == 0x2eb {
            &mut self.skip
        } else {
            &mut self.auto_advance
        };
        match value {
            Some(active) => *flag = active,
            None => vm
                .state_mut()
                .set_system_integer(0, u32::from(*flag))
                .expect("slot 0 exists"),
        }
        Ok(true)
    }
}

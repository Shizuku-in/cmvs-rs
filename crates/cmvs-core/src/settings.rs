//! Runtime configuration commands, researched against cmvs-20160201 x64.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// Shift-JIS `ＭＳ ゴシック`, the initialized global at 0x1400fc150.
pub const DEFAULT_FONT_FACE: &[u8] = b"\x82l\x82r \x83S\x83V\x83b\x83N";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsHost {
    font_face: Vec<u8>,
    auto: AutoAdvanceSettings,
}

/// +1556..+1572. Timer fields retain raw words, including negative INI/script values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoAdvanceSettings {
    pub base: u32,
    pub per_glyph: u32,
    pub acceleration: u32,
    pub voice_wait: u32,
    pub sync_voice: bool,
}
impl Default for AutoAdvanceSettings {
    fn default() -> Self {
        Self {
            base: 10,
            per_glyph: 0,
            acceleration: 0,
            voice_wait: 0,
            sync_voice: true,
        }
    }
}
impl AutoAdvanceSettings {
    /// 0x14005e220: text record count includes ruby glyphs, not source byte count.
    pub fn text_delay_ms(self, glyphs: usize) -> Result<u32, &'static str> {
        if glyphs > 4096 {
            return Err("auto advance glyph count exceeds research limit");
        }
        let linear = 5u32.wrapping_mul(
            (glyphs as u32)
                .wrapping_mul(self.per_glyph)
                .wrapping_add(20u32.wrapping_mul(self.base)),
        );
        if self.acceleration == 0 || self.per_glyph == 0 {
            return Ok(linear);
        }
        let target = self.per_glyph.wrapping_mul(5) as i32 as f32;
        let increment = self.acceleration as i32 as f32 * 0.25;
        let mut sum = 0.0f32;
        let mut step = 0.0f32;
        for _ in 0..glyphs {
            sum += step;
            if target > step {
                step += increment;
            }
        }
        // x64 CVTTSS2SI returns INT_MIN for an out-of-range float.
        let reduction = if !(-2147483648.0..2147483648.0).contains(&sum) {
            i32::MIN
        } else {
            sum as i32
        };
        let mut result = linear.wrapping_sub(reduction as u32);
        if result > 65535 {
            result = linear;
        }
        Ok(result.max(self.base.wrapping_mul(100)))
    }
}
impl Default for SettingsHost {
    fn default() -> Self {
        Self {
            font_face: DEFAULT_FONT_FACE.to_vec(),
            auto: AutoAdvanceSettings::default(),
        }
    }
}
impl SettingsHost {
    pub fn auto_advance(&self) -> AutoAdvanceSettings {
        self.auto
    }
    pub fn from_ini(bytes: &[u8]) -> Self {
        let int = |key, default| {
            crate::save_menu::profile_int(bytes, "MESSAGE_INFO", key, default) as u32
        };
        Self {
            auto: AutoAdvanceSettings {
                base: int("AUTO_MODE_1", 10),
                per_glyph: int("AUTO_MODE_2", 0),
                acceleration: int("AUTO_MODE_3", 0),
                voice_wait: int("AUTO_MODE_4", 0),
                sync_voice: int("AUTO_VOICE_SYNC", 1) != 0,
            },
            ..Self::default()
        }
    }
    pub fn font_face(&self) -> &[u8] {
        &self.font_face
    }
    /// 0x22de / 0x14007c2d0: SP-4 selects write/read, SP-8 is a string reference.
    /// Read writes the global font name to the destination and result 0 = 1.
    /// Native font selection is still a barrier; accepting another face without
    /// changing subsequent font construction would misrepresent the command.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if let Some((pc, command)) = vm.pending_system() {
            let auto_index = match command {
                0x2e7 => Some(0),
                0x2e8 => Some(1),
                0x2f8 => Some(2),
                0x2e6 => Some(3),
                _ => None,
            };
            if let Some(index) = auto_index {
                let mode = vm
                    .stack()
                    .peek_u32(4)
                    .map_err(|e| error(pc, command + 0x2000, e.0))?;
                let value = if mode != 0 {
                    let raw = vm
                        .stack()
                        .peek_u32(8)
                        .map_err(|e| error(pc, command + 0x2000, e.0))?;
                    Some(if index == 3 { u32::from(raw != 0) } else { raw })
                } else {
                    None
                };
                vm.complete_system(SystemResult(0x4008))?;
                let field = match index {
                    0 => &mut self.auto.base,
                    1 => &mut self.auto.per_glyph,
                    2 => &mut self.auto.voice_wait,
                    _ => {
                        if let Some(value) = value {
                            self.auto.sync_voice = value != 0;
                        } else {
                            vm.state_mut()
                                .set_system_integer(0, u32::from(self.auto.sync_voice))
                                .expect("slot 0 exists");
                        }
                        return Ok(true);
                    }
                };
                if let Some(value) = value {
                    *field = value;
                } else {
                    vm.state_mut()
                        .set_system_integer(0, *field)
                        .expect("slot 0 exists");
                }
                return Ok(true);
            }
            // Constructor 0x140014e60. Setters require their consumers and backend
            // effects before they may be accepted; these reads expose current defaults.
            let value = match command {
                0x2d5 => Some(1),
                0x2df => Some(1),
                0x2e0 => Some(0),
                0x2e1 => Some(4),
                0x2e2 => Some(1),
                0x2e3 => Some(2),
                0x2e4 => Some(0),
                0x2e9 => Some(1),
                _ => None,
            };
            if let Some(value) = value {
                let mode = vm
                    .stack()
                    .peek_u32(4)
                    .map_err(|e| error(pc, command + 0x2000, e.0))?;
                if mode != 0 {
                    return Err(error(
                        pc,
                        command + 0x2000,
                        "runtime configuration setter is not implemented",
                    ));
                }
                // Bounded integer wrappers load SP-8 even on query.
                if matches!(command, 0x2e1..=0x2e3) {
                    vm.stack()
                        .peek_u32(8)
                        .map_err(|e| error(pc, command + 0x2000, e.0))?;
                }
                vm.complete_system(SystemResult(0x4008))?;
                vm.state_mut()
                    .set_system_integer(0, value)
                    .expect("slot 0 exists");
                return Ok(true);
            }
        }
        let Some((pc, 0x2de)) = vm.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x22de, e.0))
        };
        let (write, reference) = (arg(4)? != 0, arg(8)?);
        if write {
            if vm.resolve_string(reference)? != self.font_face {
                return Err(error(
                    pc,
                    0x22de,
                    "changing the default font face is not implemented",
                ));
            }
            vm.complete_system(SystemResult(0x4008))?;
        } else {
            let slot = crate::text::bank_slot(reference).ok_or_else(|| {
                error(
                    pc,
                    0x22de,
                    "font name destination outside the string bank is not modeled",
                )
            })?;
            vm.complete_system(SystemResult(0x4008))?;
            vm.state_mut()
                .set_string(slot, &self.font_face)
                .expect("bounded font name and valid slot");
            vm.state_mut()
                .set_system_integer(0, 1)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

//! Additional configuration flags read and written by CVNS (`0x21ec` / `0x21ed`).
use super::{Configuration, SettingsHost};
use crate::{FormatError, save_menu::profile_int};

const TEXT_SHADOW_KEY: &str = "TEXT_SADOW";

/// Configuration fields specific to CVNS that are absent from CMVS `SettingsHost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvnsSettings {
    pub wait_timing: bool,
    pub cpu_priority: u32,
    pub text_shadow: bool,
    pub auto_skip: bool,
    pub mouse_right_mode: bool,
    pub wheel_mode: u32,
    pub auto_move: bool,
    pub frame_ctrl: u32,
    pub frame_interval_ms: u32,
    pub fixed_frame_timing: bool,
    pub sync_redraw: bool,
}

impl Default for CvnsSettings {
    fn default() -> Self {
        Self {
            wait_timing: false,
            cpu_priority: 1,
            text_shadow: false,
            auto_skip: false,
            mouse_right_mode: false,
            wheel_mode: 0,
            auto_move: false,
            frame_ctrl: 0,
            frame_interval_ms: 16,
            fixed_frame_timing: false,
            sync_redraw: true,
        }
    }
}

impl CvnsSettings {
    /// Parses `[SYSTEM_INFO]`, `[MESSAGE_INFO]`, `[MOUSE_INFO]`, `[VIDEO_INFO]` CVNS keys from INI.
    pub fn from_ini(bytes: &[u8]) -> Self {
        Self {
            wait_timing: profile_int(bytes, "SYSTEM_INFO", "WAIT_TIMING", 0) != 0,
            text_shadow: profile_int(bytes, "MESSAGE_INFO", TEXT_SHADOW_KEY, 0) != 0,
            auto_skip: profile_int(bytes, "MESSAGE_INFO", "AUTO_SKIP", 0) != 0,
            mouse_right_mode: profile_int(bytes, "SYSTEM_INFO", "MOUSE_RIGHT_MODE", 0) != 0,
            wheel_mode: profile_int(bytes, "MOUSE_INFO", "WHEEL_MODE", 0) as u32,
            auto_move: profile_int(bytes, "MOUSE_INFO", "AUTO_MOVE", 0) != 0,
            frame_ctrl: profile_int(bytes, "VIDEO_INFO", "FRAME_CTRL", 0) as u32,
            ..Self::default()
        }
    }

    pub(crate) fn write_ini(&self, configuration: &mut Configuration) -> Result<(), FormatError> {
        for (section, key, value) in [
            ("SYSTEM_INFO", "WAIT_TIMING", u32::from(self.wait_timing)),
            (
                "SYSTEM_INFO",
                "MOUSE_RIGHT_MODE",
                u32::from(self.mouse_right_mode),
            ),
            ("MESSAGE_INFO", TEXT_SHADOW_KEY, u32::from(self.text_shadow)),
            ("MESSAGE_INFO", "AUTO_SKIP", u32::from(self.auto_skip)),
            ("MOUSE_INFO", "WHEEL_MODE", self.wheel_mode),
            ("MOUSE_INFO", "AUTO_MOVE", u32::from(self.auto_move)),
            ("VIDEO_INFO", "FRAME_CTRL", self.frame_ctrl),
        ] {
            configuration.set_integer(section, key, value)?;
        }
        Ok(())
    }
}

impl SettingsHost {
    pub fn cvns(&self) -> &CvnsSettings {
        &self.cvns
    }

    pub fn cvns_mut(&mut self) -> &mut CvnsSettings {
        &mut self.cvns
    }

    pub(crate) fn set_active_draw(&mut self, value: bool) {
        self.active_draw = value;
    }

    pub(crate) fn set_read_skip(&mut self, value: bool) {
        self.read_skip = value;
    }

    pub(crate) fn set_text_speed(&mut self, value: u32) {
        self.text.speed = value;
    }

    pub(crate) fn set_reveal_mode(&mut self, value: u32) {
        self.text.reveal_mode = value;
    }

    pub(crate) fn set_reveal_duration(&mut self, value: u32) {
        self.text.reveal_duration = value;
    }

    pub(crate) fn set_auto_base(&mut self, value: u32) {
        self.auto.base = value;
    }

    pub(crate) fn set_auto_per_glyph(&mut self, value: u32) {
        self.auto.per_glyph = value;
    }

    pub(crate) fn set_auto_voice_sync(&mut self, value: bool) {
        self.auto.sync_voice = value;
    }

    pub(crate) fn set_cvns_cursor_auto_clear(&mut self, value: u32) {
        // sub_46c900 clears values above 5 back to 0.
        self.cursor_auto_clear = if value > 5 { 0 } else { value };
    }
}

use super::{Startup, SystemOutput};
use crate::{
    audio::AudioHost,
    message_flags::MessageFlags,
    save_menu::profile_int,
    settings::{Configuration, DisplayPreferences, SettingsHost},
};
use std::error::Error;

impl Startup<'_> {
    pub(super) fn install_configuration(
        &mut self,
        configuration: Configuration,
    ) -> Result<(), Box<dyn Error>> {
        let bytes = configuration.bytes();
        let audio = AudioHost::from_ini(bytes);
        self.audio_backend
            .configure_voice_ducking(audio.voice_ducking())?;
        let settings = SettingsHost::from_ini(bytes);
        settings.check_font_face()?;
        self.settings = settings;
        self.message_flags = MessageFlags::from_ini(bytes);
        self.save_menu.set_confirmations(
            profile_int(bytes, "SYSTEM_INFO", "DLG_LOAD", 0) != 0,
            profile_int(bytes, "SYSTEM_INFO", "DLG_SAVE", 0) != 0,
        );
        self.effects = profile_int(bytes, "SYSTEM_INFO", "EFFECT_USE", 1) != 0;
        self.audio = audio;
        self.platform.configure_display_selection(bytes);
        self.configuration = configuration;
        Ok(())
    }
    pub fn display_preferences(&self) -> DisplayPreferences {
        DisplayPreferences::from_ini(self.configuration.bytes())
    }

    /// Explicit graceful-exit save to the locked user directory. Each file replacement
    /// is atomic; CSS1 and INI are separate commits. No profile means no writes.
    pub fn save_preferences(
        &mut self,
        display: DisplayPreferences,
    ) -> Result<bool, Box<dyn Error>> {
        if !matches!(self.system_output, Some(SystemOutput::Profile(_))) {
            return Ok(false);
        }
        if display.aspect_mode > 3 || display.render_mode > 3 {
            return Err("invalid saved display aspect or render mode".into());
        }
        let mut configuration = self.configuration.clone();
        let text = self.settings.text_output();
        let auto = self.settings.auto_advance();
        let [load, save] = self.save_menu.confirmations();
        for (key, value) in [
            ("DLG_LOAD", u32::from(load)),
            ("DLG_SAVE", u32::from(save)),
            ("EFFECT_USE", u32::from(self.effects)),
            ("ACTIVE_DRAW", u32::from(self.settings.active_draw())),
        ] {
            configuration.set_integer("SYSTEM_INFO", key, value)?;
        }
        configuration.set_integer(
            "MOUSE_INFO",
            "AUTO_CLEAR",
            self.settings.cursor_auto_clear(),
        )?;
        for (key, value) in [
            ("TEXT_GANG", text.outline),
            ("TEXT_BOLD", u32::from(text.bold)),
            ("TEXT_WAIT", text.speed),
            ("EFFECT_TYPE", text.reveal_mode),
            ("EFFECT_TIMER", text.reveal_duration),
            ("READ_SKIP", u32::from(self.settings.read_skip())),
            ("VOICE_CUT_MODE", self.message_flags.mode_1548()),
            ("AUTO_MODE_1", auto.base),
            ("AUTO_MODE_2", auto.per_glyph),
            ("AUTO_MODE_3", auto.acceleration),
            ("AUTO_MODE_4", auto.voice_wait),
            ("AUTO_VOICE_SYNC", u32::from(auto.sync_voice)),
        ] {
            configuration.set_integer("MESSAGE_INFO", key, value)?;
        }
        configuration.set_string("MESSAGE_INFO", "FONT", self.settings.font_face())?;
        let audio = self.audio.settings();
        let duck = self.audio.voice_ducking();
        for (key, value) in [
            ("MUSIC_USE", u32::from(audio.bgm_enabled)),
            ("SE_USE", u32::from(audio.se_enabled)),
            ("VOICE_USE", u32::from(audio.voice_enabled)),
            ("MASTER_VOLUME", audio.master),
            ("MUSIC_VOLUME", audio.bgm),
            ("SE_VOLUME", audio.se),
            ("VOICE_VOLUME", audio.voice),
            ("VIDEO_VOLUME", audio.movie),
            ("VOICE_FADE_VOLOME", duck.percent),
            ("VOICE_FADE_TIMER", duck.fade_ms),
            ("VOICE_FADE_WAIT", duck.wait_ms),
            ("VOICE_FADE_USE", u32::from(duck.include_fading)),
        ] {
            configuration.set_integer("SOUND_INFO", key, value)?;
        }
        configuration.set_integer(
            "CMVS_RS_DESKTOP",
            "FULLSCREEN",
            u32::from(display.fullscreen),
        )?;
        configuration.set_integer("CMVS_RS_DESKTOP", "ASPECT_MODE", display.aspect_mode)?;
        configuration.set_integer(
            "CMVS_RS_DESKTOP",
            "ASPECT_SCALING",
            u32::from(display.aspect_scaling),
        )?;
        configuration.set_integer(
            "GRAPHIC_CONFIG",
            "ASPECT_RATIO_FILTER",
            crate::settings::render_mode_to_ini(display.render_mode),
        )?;
        let Some(SystemOutput::Profile(profile)) = &mut self.system_output else {
            unreachable!()
        };
        let id = self.bootstrap.settings().get(&0x201d).copied();
        // Only persist CSS1 after the startup load command has completed,
        // including its missing-file branch. Earlier exits may precede variable
        // initialization and must not manufacture persistent defaults.
        if self.system_host.loaded.is_some() || self.auto_resume_checked {
            self.system_host
                .write_now(self.machine.state(), profile, &mut self.random, id)?;
            self.system_source = crate::system_save::FileSystemSave::new(profile.path());
        }
        profile.write_configuration(&configuration)?;
        self.configuration = configuration;
        Ok(true)
    }
}

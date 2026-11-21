use super::{Runtime, SystemOutput};
use crate::{
    FormatError,
    audio::AudioHost,
    message_flags::MessageFlags,
    raster::ScaleFilter,
    save_menu::{profile_int, profile_string},
    settings::{Configuration, DisplayPreferences, SettingsHost},
};
use std::error::Error;

impl Runtime {
    pub(super) fn install_configuration(
        &mut self,
        configuration: Configuration,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let bytes = configuration.bytes();
        let audio = AudioHost::for_profile(self.gate.profile(), bytes);
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
        if self.gate.profile().uses_window_full_move() {
            self.platform.configure_window_full_move(bytes);
        }
        self.configuration = configuration;
        Ok(())
    }
    pub fn display_preferences(&self) -> DisplayPreferences {
        DisplayPreferences::from_ini(self.configuration.bytes())
    }

    /// Resolve the host's final presentation filter, with a launch choice taking priority.
    pub fn scale_filter_override(
        &self,
        launch: Option<&str>,
    ) -> Result<Option<ScaleFilter>, FormatError> {
        let configured;
        let name = match launch {
            Some(name) => name.as_bytes(),
            None => {
                configured = profile_string(
                    self.configuration.bytes(),
                    "CMVS_RS_DISPLAY",
                    "SCALE_FILTER",
                    b"game",
                    64,
                );
                &configured
            }
        };
        match name {
            b"game" => Ok(None),
            b"nearest" => Ok(Some(ScaleFilter::Nearest)),
            b"bilinear" => Ok(Some(ScaleFilter::Bilinear)),
            b"keys" => Ok(Some(ScaleFilter::Keys)),
            b"mitchell" => Ok(Some(ScaleFilter::Mitchell)),
            b"lanczos3" => Ok(Some(ScaleFilter::Lanczos3 {
                anti_ringing: false,
            })),
            b"lanczos3-anti-ringing" => Ok(Some(ScaleFilter::Lanczos3 { anti_ringing: true })),
            b"lanczos4" => Ok(Some(ScaleFilter::Lanczos4)),
            _ => Err(FormatError(
                "SCALE_FILTER must be game, nearest, bilinear, keys, mitchell, lanczos3, lanczos3-anti-ringing, or lanczos4",
            )),
        }
    }

    /// Explicit graceful-exit save to the locked user directory. Each file replacement
    /// is atomic; CSS1 and INI are separate commits. No profile means no writes.
    pub fn save_preferences(
        &mut self,
        display: DisplayPreferences,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !matches!(self.system_output, Some(SystemOutput::Profile(_))) {
            return Ok(false);
        }
        if display.aspect_mode > 3 || display.render_mode > 3 {
            return Err("invalid saved display aspect or render mode".into());
        }
        let mut configuration = self.configuration.clone();
        if self.gate.profile().uses_window_full_move() {
            configuration.set_integer(
                "SYSTEM_INFO",
                "WINDOW_FULL_MOVE_USE",
                u32::from(self.platform.window_full_move()),
            )?;
        }
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
            ("TEXT_REVISION_ALPHA", u32::from(text.revision_alpha)),
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
        let [master, music, se, voice, movie] = if self.gate.profile().is_cvns() {
            self.settings.cvns().write_ini(&mut configuration)?;
            std::array::from_fn(|kind| self.audio.cvns_volume(kind))
        } else {
            [audio.master, audio.bgm, audio.se, audio.voice, audio.movie]
        };
        let duck = self.audio.voice_ducking();
        let background_output = u32::from(self.audio.background_output());
        for (key, value) in [
            ("INACTIVE_SOUND_PLAY", background_output),
            ("MUSIC_USE", u32::from(audio.bgm_enabled)),
            ("SE_USE", u32::from(audio.se_enabled)),
            ("VOICE_USE", u32::from(audio.voice_enabled)),
            ("MASTER_VOLUME", master),
            ("MUSIC_VOLUME", music),
            ("SE_VOLUME", se),
            ("VOICE_VOLUME", voice),
            ("VIDEO_VOLUME", movie),
            ("VOICE_FADE_VOLOME", duck.percent),
            ("VOICE_FADE_TIMER", duck.fade_ms),
            ("VOICE_FADE_WAIT", duck.wait_ms),
            ("VOICE_FADE_USE", u32::from(duck.include_fading)),
        ] {
            configuration.set_integer("SOUND_INFO", key, value)?;
        }
        configuration.set_integer(
            "WINDOW_RESOLVING",
            "FULL_SCREEN_MONITOR_MODE",
            self.platform.fullscreen_monitor_mode(),
        )?;
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
        let id = self.system_save_identifier();
        let Some(SystemOutput::Profile(profile)) = &mut self.system_output else {
            unreachable!()
        };
        // Only persist CSS1/CSS0 after the startup load command has completed,
        // including its missing-file branch. Earlier exits may precede variable
        // initialization and must not manufacture persistent defaults.
        let updated_source = if self.system_host.loaded.is_some() || self.auto_resume_checked {
            self.system_host
                .write_now(self.machine.state(), profile, &mut self.random, id)?;
            Some((profile.path(), profile.fs().clone()))
        } else {
            None
        };
        profile.write_configuration(&configuration)?;
        if let Some((path, fs)) = updated_source {
            self.system_source = self.open_system_source(path, fs);
        }
        self.configuration = configuration;
        Ok(true)
    }
}

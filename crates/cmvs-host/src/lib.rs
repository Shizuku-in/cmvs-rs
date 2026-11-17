//! Embed a complete CMVS player without owning a window.
//!
//! Open a [`Player`], forward input with [`Player::set_key_state`] and
//! [`Player::set_pointer_position`], then call [`Player::tick`]. Present
//! [`Player::frame`] on [`TickOutcome::FrameReady`]; on
//! [`TickOutcome::BudgetExhausted`], resume the same frame in another slice.
//! Call [`Player::save_preferences`] before closing.
//!
//! The player configures fonts, audio, movies, and frame composition. For a custom
//! host that supplies those services, use [`cmvs_core::runtime`] instead.
//!
//! All calls for one player belong to one host thread;
//! the native audio backend owns and synchronizes its device callback separately.
mod backend;
mod error;
pub use error::PlayerError;
#[allow(unsafe_code)]
mod ffi;
#[cfg(all(test, unix))]
mod savedata_replay;
mod storage;

use backend::HostBackend;
use cmvs_core::{
    format::system_save::ProfileSystemSave, host::input::CodeInput, host::input::KeyBindings,
    profile::Registry, render::raster::RgbaImage, render::raster::movie_canvas,
    runtime::FrameBudget, runtime::FrameDriver, runtime::FrameOutcome, runtime::Runtime,
    runtime::RuntimeEvent, runtime::RuntimeOptions, runtime::SaveStorage,
    runtime::load_start_with_fs,
};
use cmvs_font::font::FontRenderer;
use std::{error::Error, path::Path};

struct Input {
    held: [bool; 128],
    pulses: [bool; 128],
    codes: CodeInput,
    pointer: [i32; 2],
}
impl Default for Input {
    fn default() -> Self {
        Self {
            held: [false; 128],
            pulses: [false; 128],
            codes: CodeInput::default(),
            pointer: [0; 2],
        }
    }
}
impl Input {
    fn key(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
        let held = self.held.get_mut(code).ok_or("input code outside 0..128")?;
        if pressed && !*held {
            self.pulses[code] = true;
        }
        *held = pressed;
        Ok(())
    }
    fn sample(&mut self, bindings: &KeyBindings, input: &mut cmvs_core::host::input::ActionInput) {
        let now = std::array::from_fn(|i| self.held[i] || self.pulses[i]);
        self.pulses.fill(false);
        self.codes.update(bindings, &now, input);
        input.set_pointer(self.pointer[0], self.pointer[1]);
    }
}

/// Result of one [`Player::tick`]. A budget slice does not publish a partial frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// Resume the same VM run in a later slice; no new frame is available.
    BudgetExhausted,
    /// A complete frame is available through [`Player::frame`].
    FrameReady,
    /// The game requested shutdown.
    Quit,
    /// The player is paused or suspended by the game's focus policy.
    Inactive,
}

/// An embedded player that configures fonts, audio, movies, and frame composition.
pub struct Player {
    runtime: Runtime,
    backend: HostBackend,
    font: FontRenderer,
    bindings: KeyBindings,
    input: Input,
    driver: FrameDriver,
    paused: bool,
    serial: u64,
    audio: Option<cmvs_audio::HostAudioControl>,
}
impl Player {
    pub fn open(
        root: &Path,
        user: &Path,
        font_path: &Path,
        native_audio: bool,
    ) -> Result<Self, PlayerError> {
        Self::open_with_profile(
            root,
            || {
                Ok(ProfileSystemSave::open_with_fs(
                    user,
                    root,
                    storage::host_fs(),
                )?)
            },
            font_path,
            native_audio,
        )
        .map_err(PlayerError::Initialization)
    }
    /// Prefer game/savedata, importing old external saves without replacing
    /// existing files. Read-only media retains the external per-game profile.
    pub fn open_game_savedata(
        root: &Path,
        legacy: &Path,
        font_path: &Path,
        native_audio: bool,
    ) -> Result<Self, PlayerError> {
        Self::open_with_profile(
            root,
            || Ok(storage::game_savedata(root, legacy)?),
            font_path,
            native_audio,
        )
        .map_err(PlayerError::Initialization)
    }
    fn open_with_profile(
        root: &Path,
        open_profile: impl FnOnce() -> Result<ProfileSystemSave, Box<dyn Error + Send + Sync>>,
        font_path: &Path,
        native_audio: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "android")]
        if native_audio && !ffi::android_initialized() {
            return Err(
                "initialize the CMVS Android application context before native audio".into(),
            );
        }
        let fs = storage::host_fs();
        let game = Registry::builtin().identify_game_with_fs(&*fs, root, None)?;
        let script = load_start_with_fs(&*fs, root, &game.files)?;
        let encoding =
            cmvs_font::text_codec::select_or_default_with_fs(&*fs, root, &game.files, &script)?;
        let font = FontRenderer::read(font_path, 0)?.with_text_encoding(encoding);
        let profile = open_profile()?;
        font.set_user_directory(profile.directory())?;
        let mut audio_control = None;
        let mut runtime = Runtime::with_options(
            script,
            root,
            &game,
            RuntimeOptions {
                encoding,
                saves: SaveStorage::Profile(profile),
                decode_cmv_frames: true,
                fs: Some(fs.clone()),
                ..Default::default()
            },
        )?;
        runtime.set_save_clock(Box::new(cmvs_core::runtime::utc_save_time));
        runtime.set_random_seed(cmvs_core::host::random::host_time_seed());
        runtime.set_record_text(true);
        runtime.set_host_capture(true);
        runtime.set_glyph_metrics(font.metrics());
        if !font.has_face(runtime.settings().font_face()) {
            runtime.set_font_face(cmvs_core::host::settings::DEFAULT_FONT_FACE)?;
        }
        if native_audio {
            let audio = cmvs_audio::NativeAudio::new(root)?;
            audio_control = Some(audio.host_control());
            runtime.set_audio_backend(Box::new(audio))?;
        }
        runtime.set_mgv_factory(Box::new(
            cmvs_video::PrefetchTheoraFactory::default().with_lookahead(64 << 20, 48),
        ));
        runtime.set_scene_prefetch(|| {});
        runtime.set_image_decode_threads(cmvs_core::host::image::DecodeThreads::host());
        runtime.set_decoded_image_cache(256 << 20);
        let mut backend = HostBackend::new(runtime.screen_size());
        let preferences = runtime.display_preferences();
        backend.fullscreen = preferences.fullscreen;
        backend.aspect = preferences.aspect_mode;
        backend.scaling = preferences.aspect_scaling;
        backend.render =
            cmvs_core::host::platform::supported_render_mode(&backend, preferences.render_mode)?;
        let bindings =
            KeyBindings::load_optional_for_profile_with_fs(&*fs, root, game.selection.profile)?;
        Ok(Self {
            runtime,
            backend,
            font,
            bindings,
            input: Input::default(),
            driver: FrameDriver::default(),
            paused: false,
            serial: 0,
            audio: audio_control,
        })
    }
    pub fn set_key_state(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
        self.input.key(code, pressed)
    }
    pub fn set_pointer_position(&mut self, x: i32, y: i32) {
        self.input.pointer = [x, y];
    }
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(audio) = &self.audio {
            audio.set_active(!paused);
        }
        if paused {
            self.driver.suspend();
            self.input.held.fill(false);
            self.input.pulses.fill(false);
            self.input.codes.clear(self.runtime.input_mut());
        }
    }
    pub fn size(&self) -> [u32; 2] {
        self.backend.size
    }
    pub fn frame_serial(&self) -> u64 {
        self.serial
    }
    /// Compatibility name for [`Self::set_key_state`].
    #[doc(hidden)]
    pub fn key(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
        self.set_key_state(code, pressed)
    }
    /// Compatibility name for [`Self::set_pointer_position`].
    #[doc(hidden)]
    pub fn pointer(&mut self, x: i32, y: i32) {
        self.set_pointer_position(x, y);
    }
    /// Compatibility name for [`Self::set_paused`].
    #[doc(hidden)]
    pub fn pause(&mut self, paused: bool) {
        self.set_paused(paused);
    }
    /// Compatibility name for [`Self::frame_serial`].
    #[doc(hidden)]
    pub fn serial(&self) -> u64 {
        self.frame_serial()
    }
    pub fn frame(&self) -> Option<&RgbaImage> {
        self.backend.frame.as_ref()
    }
    pub fn save_preferences(&mut self) -> Result<bool, PlayerError> {
        let display = self.backend.preferences();
        self.runtime
            .save_preferences(display)
            .map_err(PlayerError::Persistence)
    }
    /// Run a VM budget slice, publishing a frame only when the VM yields.
    ///
    /// `delta_ms` is consumed on a completed frame, capped at 250 ms. When the budget
    /// expires, the driver retains that delta and input until the frame completes;
    /// delta values on continuation calls are ignored.
    /// Unsupported commands and barriers retain their structured [`RuntimeEvent`]
    /// in [`PlayerError::Stopped`].
    pub fn tick(&mut self, delta_ms: u32) -> Result<TickOutcome, PlayerError> {
        if self.paused {
            return Ok(TickOutcome::Inactive);
        }
        self.runtime
            .poll_audio_output()
            .map_err(PlayerError::Execution)?;
        self.driver.latch_delta(delta_ms.min(250));
        self.driver.sample_input(self.runtime.input_mut(), |input| {
            self.input.sample(&self.bindings, input);
        });
        match self
            .driver
            .run_slice(
                &mut self.runtime,
                &mut FrameBudget::steps(4000),
                |runtime| runtime.tick(&mut self.backend),
            )
            .map_err(|e| PlayerError::Execution(e.into()))?
        {
            FrameOutcome::Yield => {
                self.driver
                    .finish_frame(&mut self.runtime, 0)
                    .map_err(PlayerError::Execution)?;
                let frame = if let Some(image) = self.runtime.movie_frame() {
                    movie_canvas(image, self.backend.size, self.backend.render == 1)
                        .map_err(|e| PlayerError::Execution(e.into()))?
                } else {
                    self.font
                        .render(
                            self.runtime.scene(),
                            self.runtime.camera(),
                            self.runtime.messages(),
                        )
                        .map_err(|e| PlayerError::Execution(e.into()))?
                };
                if self.runtime.capture_due() {
                    self.runtime
                        .supply_capture(frame.clone())
                        .map_err(PlayerError::Execution)?;
                }
                self.backend.frame = Some(frame);
                self.serial += 1;
                Ok(TickOutcome::FrameReady)
            }
            FrameOutcome::Stopped(RuntimeEvent::Quit) => Ok(TickOutcome::Quit),
            FrameOutcome::Inactive => Ok(TickOutcome::Inactive),
            FrameOutcome::Stopped(event) => Err(PlayerError::Stopped(event)),
            FrameOutcome::BudgetExhausted => Ok(TickOutcome::BudgetExhausted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_open_preserves_the_initialization_error_source() {
        let missing =
            std::env::temp_dir().join(format!("cmvs-missing-player-{}", std::process::id()));
        let error = Player::open(&missing, &missing, &missing, false)
            .err()
            .unwrap();
        assert!(matches!(error, PlayerError::Initialization(_)));
        assert!(error.source().is_some());
    }
    #[test]
    fn quick_click_is_latched_and_released_at_frame_boundaries() {
        let mut host = Input::default();
        let mut input = cmvs_core::host::input::ActionInput::default();
        host.key(1, true).unwrap();
        host.key(1, false).unwrap();
        host.sample(&KeyBindings::default(), &mut input);
        assert!(input.mouse(0).iter().any(|x| *x));
        host.sample(&KeyBindings::default(), &mut input);
        host.sample(&KeyBindings::default(), &mut input);
        assert_eq!(input.mouse(0), [false; 3]);
        assert!(host.key(128, true).is_err());
    }
}

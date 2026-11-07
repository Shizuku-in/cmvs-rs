//! A window-independent player. All calls for one player belong to one host thread;
//! the native audio backend owns and synchronizes its device callback separately.
mod backend;
#[allow(unsafe_code)]
mod ffi;
#[cfg(all(test, unix))]
mod savedata_replay;
mod storage;

use backend::HostBackend;
use cmvs_core::{
    input::{CodeInput, KeyBindings},
    profile::Registry,
    ps3::Script,
    raster::RgbaImage,
    runner::{Startup, StartupEvent, load_start},
    system_save::ProfileSystemSave,
};
use cmvs_font::font::FontRenderer;
use std::{error::Error, io::Read, path::Path};

self_cell::self_cell! {
    struct Session {
        owner: Script,
        #[covariant]
        dependent: Startup,
    }
}

struct Input {
    held: [bool; 128],
    pulses: [bool; 128],
    codes: CodeInput,
    functions: u16,
    pointer: [i32; 2],
}
impl Default for Input {
    fn default() -> Self {
        Self {
            held: [false; 128],
            pulses: [false; 128],
            codes: CodeInput::default(),
            functions: 0,
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
    fn sample(&mut self, bindings: &KeyBindings, input: &mut cmvs_core::input::ActionInput) {
        let now = std::array::from_fn(|i| self.held[i] || self.pulses[i]);
        self.pulses.fill(false);
        let functions = (0..12).fold(0u16, |mask, i| mask | (u16::from(now[52 + i]) << i));
        input.press_function_keys(functions & !self.functions);
        self.functions = functions;
        self.codes.update(bindings, &now, input);
        input.set_pointer(self.pointer[0], self.pointer[1]);
    }
}

pub struct Player {
    session: Session,
    backend: HostBackend,
    font: FontRenderer,
    bindings: KeyBindings,
    input: Input,
    input_due: bool,
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
    ) -> Result<Self, Box<dyn Error>> {
        Self::open_with_profile(
            root,
            || Ok(ProfileSystemSave::open(user, root)?),
            font_path,
            native_audio,
        )
    }
    /// Prefer game/savedata, importing old external saves without replacing
    /// existing files. Read-only media retains the external per-game profile.
    pub fn open_game_savedata(
        root: &Path,
        legacy: &Path,
        font_path: &Path,
        native_audio: bool,
    ) -> Result<Self, Box<dyn Error>> {
        Self::open_with_profile(
            root,
            || Ok(storage::game_savedata(root, legacy)?),
            font_path,
            native_audio,
        )
    }
    fn open_with_profile(
        root: &Path,
        open_profile: impl FnOnce() -> Result<ProfileSystemSave, Box<dyn Error>>,
        font_path: &Path,
        native_audio: bool,
    ) -> Result<Self, Box<dyn Error>> {
        #[cfg(target_os = "android")]
        if native_audio && !ffi::android_initialized() {
            return Err(
                "initialize the CMVS Android application context before native audio".into(),
            );
        }
        let game = Registry::builtin().identify_game(root, None)?;
        let font = FontRenderer::read(font_path, 0)?;
        let profile = open_profile()?;
        font.set_user_directory(profile.directory())?;
        let script = load_start(root, &game.files)?;
        let mut audio_control = None;
        let session = Session::try_new(script, |script| -> Result<Startup<'_>, Box<dyn Error>> {
            let mut startup = Startup::new(script, root, &game, 0)?;
            startup.set_save_profile(profile)?;
            startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
            startup.set_record_text(true);
            startup.set_glyph_metrics(font.metrics());
            if !font.has_face(startup.settings().font_face()) {
                startup.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)?;
            }
            if native_audio {
                let audio = cmvs_audio::NativeAudio::new(root)?;
                audio_control = Some(audio.host_control());
                startup.set_audio_backend(Box::new(audio))?;
            }
            startup.set_decode_movie_frames(true);
            Ok(startup)
        })?;
        let startup = session.borrow_dependent();
        let mut backend = HostBackend::new(startup.screen_size());
        let preferences = startup.display_preferences();
        backend.fullscreen = preferences.fullscreen;
        backend.aspect = preferences.aspect_mode;
        backend.scaling = preferences.aspect_scaling;
        backend.render =
            cmvs_core::platform::supported_render_mode(&backend, preferences.render_mode)?;
        let key_path = root.join("key.cfg");
        let bindings = if key_path.is_file() {
            let mut text = String::new();
            std::fs::File::open(key_path)?
                .take(65537)
                .read_to_string(&mut text)?;
            KeyBindings::parse(&text)?
        } else {
            KeyBindings::default()
        };
        Ok(Self {
            session,
            backend,
            font,
            bindings,
            input: Input::default(),
            input_due: true,
            paused: false,
            serial: 0,
            audio: audio_control,
        })
    }
    pub fn key(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
        self.input.key(code, pressed)
    }
    pub fn pointer(&mut self, x: i32, y: i32) {
        self.input.pointer = [x, y];
    }
    pub fn pause(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(audio) = &self.audio {
            audio.set_active(!paused);
        }
        if paused {
            self.input.held.fill(false);
            self.input.pulses.fill(false);
            self.input.functions = 0;
            self.session
                .with_dependent_mut(|_, s| self.input.codes.clear(s.input_mut()));
        }
    }
    pub fn size(&self) -> [u32; 2] {
        self.backend.size
    }
    pub fn serial(&self) -> u64 {
        self.serial
    }
    pub fn frame(&self) -> Option<&RgbaImage> {
        self.backend.frame.as_ref()
    }
    pub fn save_preferences(&mut self) -> Result<bool, Box<dyn Error>> {
        let display = self.backend.preferences();
        self.session
            .with_dependent_mut(|_, s| s.save_preferences(display))
    }
    /// 0 = budget slice, 1 = complete frame, 2 = game quit, 3 = paused/inactive.
    /// A budget slice must not advance time, re-latch input or expose a partial frame.
    pub fn tick(&mut self, delta_ms: u32) -> Result<i32, Box<dyn Error>> {
        if self.paused {
            return Ok(3);
        }
        self.session.with_dependent_mut(|_, startup| {
            startup.poll_audio_output()?;
            if self.input_due {
                self.input.sample(&self.bindings, startup.input_mut());
                self.input_due = false;
            }
            for _ in 0..4000 {
                match startup.tick(&mut self.backend)? {
                    StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                    StartupEvent::Yield => {
                        startup.advance_decoded_frame(delta_ms.min(250))?;
                        let frame = if let Some(image) = startup.movie_frame() {
                            self.backend.movie_canvas(image)?
                        } else {
                            self.font.render(
                                startup.scene(),
                                startup.camera(),
                                startup.messages(),
                            )?
                        };
                        self.backend.frame = Some(frame);
                        self.serial += 1;
                        self.input_due = true;
                        return Ok(1);
                    }
                    StartupEvent::Quit => return Ok(2),
                    StartupEvent::Inactive => {
                        self.input_due = true;
                        return Ok(3);
                    }
                    event => return Err(format!("CMVS stopped: {event:?}").into()),
                }
            }
            Ok(0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_click_is_latched_and_released_at_frame_boundaries() {
        let mut host = Input::default();
        let mut input = cmvs_core::input::ActionInput::default();
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

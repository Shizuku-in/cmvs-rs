mod backend;
mod backup;
#[allow(unsafe_code)]
mod ffi;
mod inspect;
#[cfg(target_os = "ios")]
mod ios_audio;
mod renderer;

use backend::Backend;
use cmvs_audio::HostAudioControl;
#[cfg(not(target_os = "ios"))]
use cmvs_audio::NativeAudio;
use cmvs_core::{
    format::system_save::ProfileSystemSave,
    runtime::{FrameBudget, FrameOutcome, RuntimeEvent, SaveStorage, SnapshotPolicy},
};
use cmvs_font::font::FontRenderer;
use cmvs_host::{HostSession, SessionConfig, game_savedata};
use cmvs_input::{TouchInput, Viewport};
use renderer::{Frame, Metal};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Deserialize)]
struct Config {
    root: PathBuf,
    user: PathBuf,
    font: PathBuf,
    face: u32,
    profile: Option<String>,
    encoding: String,
    game_savedata: bool,
    native_audio: bool,
    metal: bool,
    aspect: Option<u32>,
    skip_auto_resume: bool,
    continue_without_snapshot: bool,
    block_presumed: bool,
    block_unreviewed: bool,
}
struct Player {
    session: HostSession,
    backend: Backend,
    audio: Option<HostAudioControl>,
    #[cfg(target_os = "ios")]
    output_audio: Option<ios_audio::Output>,
    metal: Option<Metal>,
    use_metal: bool,
    output: [u32; 2],
    touch: TouchInput,
    touching: bool,
    fresh_touch: bool,
    paused: bool,
    cursor_clock: std::time::Instant,
    cursor: cmvs_core::format::cursor::CursorVisibility,
    cursor_visible: bool,
}
impl Player {
    fn open(c: Config) -> Result<Self> {
        let boot = inspect::bootstrap(&c.root, c.profile.as_deref(), &c.encoding)?;
        let font = FontRenderer::read(&c.font, c.face)?;
        let saves = if c.game_savedata {
            game_savedata(&c.root, &c.user)?
        } else {
            ProfileSystemSave::open(&c.user, &c.root)?
        };
        let user_font_dir = Some(saves.directory().to_owned());
        let mut session = boot.into_session(
            font,
            SessionConfig {
                saves: SaveStorage::Profile(saves),
                user_font_dir,
                skip_auto_resume: c.skip_auto_resume,
                block_presumed: c.block_presumed,
                block_unreviewed: c.block_unreviewed,
                snapshot_policy: if c.continue_without_snapshot {
                    SnapshotPolicy::ContinueWithoutImage
                } else {
                    SnapshotPolicy::Capture
                },
                ..Default::default()
            },
        )?;
        #[cfg(target_os = "ios")]
        let mut output_audio = None;
        let audio = if c.native_audio {
            #[cfg(target_os = "ios")]
            let audio = {
                let (audio, output) = ios_audio::Output::new(&c.root)?;
                output_audio = Some(output);
                audio
            };
            #[cfg(not(target_os = "ios"))]
            let audio = NativeAudio::new(&c.root)?;
            let ctl = audio.host_control();
            session.runtime.set_audio_backend(Box::new(audio))?;
            Some(ctl)
        } else {
            None
        };
        let mut backend = Backend {
            display: session.runtime.display_preferences(),
            aspect_override: c.aspect.filter(|m| *m <= 3),
            size: session.runtime.screen_size(),
            encoding: session.encoding,
            frame: None,
            events: Default::default(),
            comment: Default::default(),
            font: None,
            message: None,
        };
        backend.display.render_mode = cmvs_core::host::platform::supported_render_mode(
            &backend,
            backend.display.render_mode,
        )?;
        let mut touch = TouchInput::default();
        touch.focus(true, session.runtime.input_mut());
        Ok(Self {
            session,
            output: backend.size,
            backend,
            audio,
            #[cfg(target_os = "ios")]
            output_audio,
            use_metal: c.metal,
            metal: None,
            touch,
            touching: false,
            fresh_touch: false,
            paused: false,
            cursor_clock: std::time::Instant::now(),
            cursor: Default::default(),
            cursor_visible: true,
        })
    }
    fn viewport(&self) -> Option<Viewport> {
        Viewport::for_canvas(self.backend.size, self.output, self.backend.mode())
    }
    fn pause(&mut self, value: bool) {
        self.paused = value;
        self.session.runtime.set_focused(!value);
        self.touch.focus(!value, self.session.runtime.input_mut());
        if value {
            self.session.driver.suspend();
            self.session.input.clear(self.session.runtime.input_mut());
            self.touching = false;
        }
        if let Some(audio) = &self.audio {
            audio.set_active(!value);
        }
    }
    fn tick(&mut self, delta: u32) -> Result<i32> {
        if self.paused || self.output.contains(&0) {
            return Ok(3);
        }
        let viewport = self.viewport().ok_or("invalid viewport")?;
        #[cfg(target_os = "ios")]
        if let Some(audio) = &self.output_audio
            && audio.needs_restart()?
        {
            self.pause(true);
            self.backend
                .events
                .push_back(serde_json::json!({"kind":"audioPaused"}));
            return Ok(3);
        }
        let s = &mut self.session;
        s.runtime.poll_audio_output()?;
        let auto_clear = s.runtime.settings().cursor_auto_clear();
        s.driver.sample_input(s.runtime.input_mut(), |input| {
            if self.touching {
                let (point, pressed) = self.touch.sample_pointer(viewport);
                let [x, y] = point.unwrap_or([-1, -1]);
                if self.fresh_touch {
                    s.input.set_pointer(x, y);
                    self.fresh_touch = false;
                }
                s.input.set_held(1, pressed).expect("valid code");
            }
            s.input.sample(&s.bindings, input);
            let point = s.input.pointer();
            let inside = point.iter().all(|i| *i >= 0);
            let shown = self.cursor.update(
                self.cursor_clock.elapsed().as_millis() as u32,
                auto_clear,
                inside,
                point,
            ) || !inside;
            if shown != self.cursor_visible {
                self.backend
                    .events
                    .push_back(serde_json::json!({"kind":"visibility","visible":shown}));
                self.cursor_visible = shown;
            }
        });
        let outcome = s.driver.run_slice(
            &mut s.runtime,
            &mut FrameBudget::time(Duration::from_millis(8)),
            |rt| rt.tick(&mut self.backend),
        )?;
        if let Some([x, y]) = s.runtime.input_mut().take_warp() {
            self.fresh_touch = false;
            s.input.set_pointer(x, y);
            if cfg!(target_os = "macos") {
                self.backend.events.push_back(
                    serde_json::json!({"kind":"warp","point":viewport.to_window([x,y])}),
                );
            }
        }
        match outcome {
            FrameOutcome::Yield => {
                s.driver.finish_frame(&mut s.runtime, delta.min(250))?;
                let linear = self.backend.display.render_mode == 1;
                self.backend.frame = Some(if self.use_metal {
                    let frame = self.metal.as_mut().ok_or("Metal layer is detached")?.draw(
                        s,
                        self.output,
                        self.backend.mode(),
                        linear,
                    )?;
                    if s.runtime.capture_due() {
                        s.runtime.supply_capture(frame.read()?)?;
                    }
                    frame
                } else {
                    Frame::Cpu(s.compose_cpu_frame(linear, true)?)
                });
                Ok(1)
            }
            FrameOutcome::BudgetExhausted => Ok(0),
            FrameOutcome::Inactive => Ok(3),
            FrameOutcome::Stopped(RuntimeEvent::Quit) => Ok(2),
            FrameOutcome::Stopped(event) => Err(format!("{event:?}").into()),
        }
    }
}

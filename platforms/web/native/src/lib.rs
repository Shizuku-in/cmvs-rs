//! Browser game host. JavaScript owns files, scheduling and presentation.
mod audio;
mod backend;
#[cfg(target_arch = "wasm32")]
mod gpu;
use backend::BrowserBackend;
use cmvs_core::{
    input::{CodeInput, KeyBindings},
    profile::Registry,
    ps3::Script,
    runner::{Startup, StartupEvent, load_start},
};
use cmvs_font::font::FontRenderer;
use std::{error::Error, path::Path};
use wasm_bindgen::prelude::*;

self_cell::self_cell! {
    struct Session {
        owner: Script,
        #[covariant]
        dependent: Startup,
    }
}
fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// One runtime per worker; dropping it releases scripts, archives and render caches.
#[wasm_bindgen]
pub struct WebRuntime {
    session: Session,
    backend: BrowserBackend,
    font: FontRenderer,
    bindings: KeyBindings,
    codes: CodeInput,
    input_due: bool,
    functions: u16,
}
#[wasm_bindgen]
impl WebRuntime {
    #[cfg(target_arch = "wasm32")]
    pub async fn enable_gpu(&mut self, canvas: web_sys::OffscreenCanvas) -> Result<(), JsValue> {
        if self.backend.gpu.is_some() {
            return Err(js_error("WebGPU is already initialized"));
        }
        self.backend.gpu = Some(
            gpu::BrowserGpu::new(canvas, self.backend.size)
                .await
                .map_err(js_error)?,
        );
        self.backend.frame = None;
        Ok(())
    }
    /// An immediate screenshot suspends at its pending command until mapping completes.
    #[cfg(target_arch = "wasm32")]
    pub async fn tick_gpu(
        &mut self,
        delta_ms: u32,
        keys: Vec<u8>,
        x: i32,
        y: i32,
        focused: bool,
    ) -> Result<u8, JsValue> {
        loop {
            let result = self.tick(delta_ms, &keys, x, y, focused)?;
            if result != 4 {
                return Ok(result);
            }
            self.backend.read_requested = false;
            let frame = self
                .backend
                .gpu
                .as_ref()
                .and_then(|gpu| gpu.frame.as_ref())
                .ok_or_else(|| js_error("no completed GPU frame"))?;
            self.backend.frame = Some(frame.read_async().await.map_err(js_error)?);
        }
    }
    #[wasm_bindgen(constructor)]
    pub fn new(
        font: Vec<u8>,
        face: u32,
        profile: &str,
        skip_auto_resume: bool,
    ) -> Result<WebRuntime, JsValue> {
        Self::create(font, face, profile, skip_auto_resume).map_err(js_error)
    }
    pub fn render_mode(&self) -> u32 {
        self.backend.preferences().render_mode
    }
    pub fn save_preferences(&mut self) -> Result<bool, JsValue> {
        let display = self.backend.preferences();
        self.session
            .with_dependent_mut(|_, startup| startup.save_preferences(display))
            .map_err(js_error)
    }
    pub fn set_clock(&mut self, time: &[u16]) -> Result<(), JsValue> {
        let time: [u16; 6] = time
            .try_into()
            .map_err(|_| js_error("expected six save clock fields"))?;
        self.session
            .with_dependent_mut(|_, startup| startup.set_save_clock(Box::new(move || time)));
        Ok(())
    }
    pub fn needs_input(&self) -> bool {
        self.input_due
    }
    pub fn width(&self) -> u32 {
        self.backend.size[0]
    }
    pub fn height(&self) -> u32 {
        self.backend.size[1]
    }
    /// 0: cooperative slice, 1: complete frame, 2: quit, 3: inactive.
    /// Input is latched once per engine frame, including across budget slices.
    pub fn tick(
        &mut self,
        delta_ms: u32,
        keys: &[u8],
        x: i32,
        y: i32,
        focused: bool,
    ) -> Result<u8, JsValue> {
        let backend = &mut self.backend;
        let font = &mut self.font;
        let codes = &mut self.codes;
        let bindings = &self.bindings;
        let input_due = &mut self.input_due;
        let functions = &mut self.functions;
        self.session
            .with_dependent_mut(|_, startup| -> Result<u8, Box<dyn Error>> {
                startup.set_focused(focused);
                startup.poll_audio_output()?;
                if !focused {
                    codes.clear(startup.input_mut());
                    *functions = 0;
                }
                if *input_due {
                    let mut now = [false; 128];
                    for &key in keys {
                        if let Some(value) = now.get_mut(usize::from(key)) {
                            *value = true;
                        }
                    }
                    let held = (0..12).fold(0u16, |mask, i| mask | (u16::from(now[52 + i]) << i));
                    startup.input_mut().press_function_keys(held & !*functions);
                    *functions = held;
                    codes.update(bindings, &now, startup.input_mut());
                    startup.input_mut().set_pointer(x, y);
                    *input_due = false;
                }
                for _ in 0..4000 {
                    let event = match startup.tick(backend) {
                        Ok(event) => event,
                        Err(error) => {
                            #[cfg(target_arch = "wasm32")]
                            if backend.read_requested {
                                return Ok(4);
                            }
                            return Err(error);
                        }
                    };
                    match event {
                        StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                        StartupEvent::Yield => {
                            startup.advance_decoded_frame(delta_ms.min(250))?;
                            #[cfg(target_arch = "wasm32")]
                            {
                                let display = backend.preferences();
                                if let Some(gpu) = &mut backend.gpu {
                                    if gpu.draw(startup, font, display)? {
                                        backend.frame = None;
                                    }
                                    *input_due = true;
                                    return Ok(1);
                                }
                            }
                            let frame = if let Some(image) = startup.movie_frame() {
                                backend.movie_canvas(image)?
                            } else {
                                font.render(startup.scene(), startup.camera(), startup.messages())?
                            };
                            backend.frame = Some(frame);
                            *input_due = true;
                            return Ok(1);
                        }
                        StartupEvent::Quit => return Ok(2),
                        StartupEvent::Inactive => {
                            *input_due = true;
                            return Ok(3);
                        }
                        event => return Err(format!("browser runtime stopped: {event:?}").into()),
                    }
                }
                Ok(0)
            })
            .map_err(js_error)
    }
    pub fn pixels(&self) -> Result<Vec<u8>, JsValue> {
        self.backend
            .frame
            .as_ref()
            .map(|frame| frame.pixels().to_vec())
            .ok_or_else(|| js_error("no completed frame"))
    }
    /// Explicit GPU readback for exports and verification; normal frames stay on the GPU.
    #[cfg(target_arch = "wasm32")]
    pub async fn pixels_async(&self) -> Result<Vec<u8>, JsValue> {
        if let Some(gpu) = &self.backend.gpu {
            let frame = gpu
                .frame
                .as_ref()
                .ok_or_else(|| js_error("no completed GPU frame"))?;
            return frame
                .read_async()
                .await
                .map(|image| image.pixels().to_vec())
                .map_err(js_error);
        }
        self.pixels()
    }
}
impl WebRuntime {
    fn create(font: Vec<u8>, face: u32, profile: &str, skip: bool) -> Result<Self, Box<dyn Error>> {
        let root = Path::new("/game");
        let game = Registry::builtin().identify_game(
            root,
            if profile.is_empty() {
                None
            } else {
                Some(profile)
            },
        )?;
        let font = FontRenderer::from_bytes(font, face)?;
        let script = load_start(root, &game.files)?;
        let session = Session::try_new(script, |script| -> Result<Startup<'_>, Box<dyn Error>> {
            let mut startup = Startup::new(script, root, &game, 0)?;
            startup.set_user_data_directory(Path::new("/user"))?;
            startup.set_skip_auto_resume(skip)?;
            startup.set_record_text(true);
            startup.set_glyph_metrics(font.metrics());
            startup.set_audio_backend(Box::new(audio::BrowserAudio::new(root)?))?;
            startup.set_decode_movie_frames(true);
            Ok(startup)
        })?;
        let size = session.borrow_dependent().screen_size();
        let key_path = root.join("key.cfg");
        let bindings = if cmvs_core::filesystem::is_file(&key_path) {
            use std::io::Read;
            let mut text = String::new();
            cmvs_core::filesystem::File::open(key_path)?
                .take(65537)
                .read_to_string(&mut text)?;
            KeyBindings::parse(&text)?
        } else {
            KeyBindings::default()
        };
        Ok(Self {
            session,
            backend: BrowserBackend::new(size),
            font,
            bindings,
            codes: CodeInput::default(),
            input_due: true,
            functions: 0,
        })
    }
}

/// Stable save namespace derived from the selected profile and input data evidence.
#[wasm_bindgen]
pub fn identify(profile: &str) -> Result<String, JsValue> {
    let game = Registry::builtin()
        .identify_game(
            Path::new("/game"),
            if profile.is_empty() {
                None
            } else {
                Some(profile)
            },
        )
        .map_err(js_error)?;
    let evidence = game.selection.evidence.join("\n");
    Ok(format!(
        "{}-{}",
        game.selection.profile.id,
        cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(evidence.as_bytes()))
    ))
}

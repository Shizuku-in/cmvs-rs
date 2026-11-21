//! Browser game host. JavaScript owns files, scheduling and presentation.
mod audio;
mod backend;
mod fs;
#[cfg(target_arch = "wasm32")]
mod gpu;
use backend::BrowserBackend;
use cmvs_core::{
    MAX_PAYLOAD, host::filesystem::HostFs, profile::Registry, render::raster::ScaleFilter,
    runtime::FrameBudget, runtime::FrameOutcome, runtime::RuntimeEvent, runtime::SaveStorage,
};
use cmvs_font::font::FontRenderer;
use cmvs_host::{EncodingPolicy, GameBootstrap, HostSession, KeyBindingPolicy, SessionConfig};
use fs::WebFs;
use std::{error::Error, path::Path, sync::Arc};
use wasm_bindgen::prelude::*;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// One runtime per worker; dropping it releases scripts, archives and render caches.
#[wasm_bindgen]
pub struct WebRuntime {
    session: HostSession,
    backend: BrowserBackend,
    scale_filter: Option<ScaleFilter>,
    output: [u32; 2],
}
#[wasm_bindgen]
pub struct WebCommentInput {
    pub id: u32,
    #[wasm_bindgen(getter_with_clone)]
    pub initial: String,
    pub max_chars: u32,
}

#[wasm_bindgen]
impl WebRuntime {
    pub fn take_comment_input(&mut self) -> Option<WebCommentInput> {
        let request = self.backend.comment_input.take_request()?;
        Some(WebCommentInput {
            id: request.id,
            initial: request.initial,
            max_chars: request.max_chars,
        })
    }

    pub fn comment_input_rect(&self) -> Vec<i32> {
        self.session
            .runtime
            .keyboard_comment_rect()
            .map_or_else(Vec::new, |rect| rect.map(|value| value as i32).to_vec())
    }

    pub fn submit_comment_input(&mut self, id: u32, text: Option<String>) {
        self.backend.comment_input.submit(id, text);
    }

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
        let mut budget = FrameBudget::steps(4000);
        loop {
            let result = self.tick_slice(delta_ms, &keys, [x, y], focused, &mut budget)?;
            if result != 4 {
                if result == 1 && self.session.runtime.capture_due() {
                    let image = {
                        let gpu = self
                            .backend
                            .gpu
                            .as_mut()
                            .ok_or_else(|| js_error("no completed GPU frame"))?;
                        let capture = gpu.capture.take();
                        let frame = capture
                            .as_ref()
                            .or(gpu.frame.as_ref())
                            .ok_or_else(|| js_error("no completed GPU frame"))?;
                        frame.read_async().await.map_err(js_error)?
                    };
                    self.session
                        .runtime
                        .supply_capture(image)
                        .map_err(js_error)?;
                }
                return Ok(result);
            }
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
        scale_filter: Option<String>,
    ) -> Result<WebRuntime, JsValue> {
        Self::create(
            font,
            face,
            profile,
            skip_auto_resume,
            scale_filter.as_deref(),
        )
        .map_err(js_error)
    }
    pub fn render_mode(&self) -> u32 {
        self.backend.preferences().render_mode
    }
    pub fn save_preferences(&mut self) -> Result<bool, JsValue> {
        let display = self.backend.preferences();
        self.session
            .runtime
            .save_preferences(display)
            .map_err(js_error)
    }
    pub fn set_random_seed(&mut self, seed: u32) {
        self.session.runtime.set_random_seed(seed);
    }
    pub fn set_clock(&mut self, time: &[u16]) -> Result<(), JsValue> {
        let time: [u16; 6] = time
            .try_into()
            .map_err(|_| js_error("expected six save clock fields"))?;
        self.session.runtime.set_save_clock(Box::new(move || time));
        Ok(())
    }
    pub fn needs_input(&self) -> bool {
        self.session.driver.needs_input()
    }
    pub fn width(&self) -> u32 {
        self.backend.size[0]
    }
    pub fn height(&self) -> u32 {
        self.backend.size[1]
    }
    pub fn uses_custom_scale_filter(&self) -> bool {
        self.scale_filter.is_some()
    }
    pub fn resize_output(&mut self, width: u32, height: u32) {
        if self.scale_filter.is_none() || width == 0 || height == 0 {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        let texture_limit = self
            .backend
            .gpu
            .as_ref()
            .map_or(u32::MAX, |gpu| gpu.texture_limit());
        #[cfg(not(target_arch = "wasm32"))]
        let texture_limit = u32::MAX;
        // Keep the real surface within the same pixel budget as the logical canvas.
        let max_pixels = (MAX_PAYLOAD / 4) as u32;
        let pixels = f64::from(width) * f64::from(height);
        let scale = (f64::from(max_pixels) / pixels)
            .sqrt()
            .min(f64::from(texture_limit.min(max_pixels)) / f64::from(width.max(height)))
            .min(1.0);
        self.output =
            [width, height].map(|value| (f64::from(value) * scale).floor().max(1.0) as u32);
    }
    pub fn presentation_size(&self) -> Vec<u32> {
        self.output.to_vec()
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
        self.tick_slice(
            delta_ms,
            keys,
            [x, y],
            focused,
            &mut FrameBudget::steps(4000),
        )
    }
    pub fn pixels(&self) -> Result<Vec<u8>, JsValue> {
        self.backend
            .frame
            .as_ref()
            .map(|frame| frame.pixels().to_vec())
            .ok_or_else(|| js_error("no completed frame"))
    }
    pub fn presented_pixels(&self) -> Result<Vec<u8>, JsValue> {
        let Some(filter) = self.scale_filter else {
            return self.pixels();
        };
        let frame = self
            .backend
            .frame
            .as_ref()
            .ok_or_else(|| js_error("no completed frame"))?;
        let image = frame
            .presentation_canvas(self.output, 3, filter)
            .map_err(js_error)?;
        Ok(image.pixels().to_vec())
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
    fn tick_slice(
        &mut self,
        delta_ms: u32,
        keys: &[u8],
        [x, y]: [i32; 2],
        focused: bool,
        budget: &mut FrameBudget,
    ) -> Result<u8, JsValue> {
        let session = &mut self.session;
        let backend = &mut self.backend;
        (|| -> Result<u8, Box<dyn Error + Send + Sync>> {
            session.runtime.set_focused(focused);
            session.runtime.poll_audio_output()?;
            if !focused {
                session.input.clear(session.runtime.input_mut());
            }
            let bindings = &session.bindings;
            let input_state = &mut session.input;
            session
                .driver
                .sample_input(session.runtime.input_mut(), |input| {
                    input_state.set_held_codes(keys);
                    input_state.set_pointer(x, y);
                    input_state.sample(bindings, input);
                });
            let outcome = session
                .driver
                .run_slice(&mut session.runtime, budget, |runtime| {
                    runtime.tick(backend)
                })?;
            match outcome {
                FrameOutcome::Yield => {
                    session
                        .driver
                        .finish_frame(&mut session.runtime, delta_ms.min(250))?;
                    #[cfg(target_arch = "wasm32")]
                    {
                        let display = backend.preferences();
                        if let Some(gpu) = &mut backend.gpu {
                            if gpu.draw(
                                &session.runtime,
                                &mut session.font,
                                display,
                                self.output,
                                self.scale_filter,
                            )? {
                                backend.frame = None;
                            }
                            return Ok(1);
                        }
                    }
                    let frame =
                        session.compose_cpu_frame(backend.preferences().render_mode == 1, true)?;
                    backend.frame = Some(frame);
                    Ok(1)
                }
                FrameOutcome::Stopped(RuntimeEvent::DeviceFramePending) => Ok(4),
                FrameOutcome::Stopped(RuntimeEvent::Quit) => Ok(2),
                FrameOutcome::Inactive => Ok(3),
                FrameOutcome::Stopped(event) => {
                    Err(format!("browser runtime stopped: {event:?}").into())
                }
                FrameOutcome::BudgetExhausted => Ok(0),
            }
        })()
        .map_err(js_error)
    }
    fn create(
        font: Vec<u8>,
        face: u32,
        profile: &str,
        skip: bool,
        scale_filter: Option<&str>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let root = Path::new("/game");
        let fs: Arc<dyn HostFs> = Arc::new(WebFs);
        let bootstrap = GameBootstrap::load(
            fs,
            root,
            (!profile.is_empty()).then_some(profile),
            EncodingPolicy::AutoDefaultShiftJis,
        )?;
        if bootstrap.game.selection.profile.is_cvns() {
            return Err(
                "CVNS games require audio support that is unavailable in the browser player".into(),
            );
        }
        let font = FontRenderer::from_bytes(font, face)?.with_text_encoding(bootstrap.encoding);
        let mut session = bootstrap.into_session(
            font,
            SessionConfig {
                saves: SaveStorage::UserDirectory("/user".into()),
                skip_auto_resume: skip,
                key_bindings: KeyBindingPolicy::Optional,
                ..SessionConfig::single_threaded()
            },
        )?;
        session
            .runtime
            .set_audio_backend(Box::new(audio::BrowserAudio::new(root)?))?;
        let backend = BrowserBackend::with_preferences(
            session.runtime.screen_size(),
            session.runtime.display_preferences(),
            session.runtime.machine().text_encoding(),
        )?;
        let scale_filter = session.runtime.scale_filter_override(scale_filter)?;
        let output = backend.size;
        Ok(Self {
            session,
            backend,
            scale_filter,
            output,
        })
    }
}

/// Stable save namespace derived from the selected profile and input data evidence.
#[wasm_bindgen]
pub fn identify(profile: &str) -> Result<String, JsValue> {
    let game = Registry::builtin()
        .identify_game_with_fs(
            &WebFs,
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
        cmvs_core::format::digest::to_hex(&cmvs_core::format::digest::sha256(evidence.as_bytes()))
    ))
}

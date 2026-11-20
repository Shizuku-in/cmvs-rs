use crate::encoding::EncodingChoice;
use crate::game_input::{GameInput, event_key};
use crate::{DesktopInput, InputEvent, NativeBackend, TextEncoding};
use cmvs_core::{
    format::cpz::ArchiveIndex, format::cursor::CursorBitmap, format::pb3, host::input::KeyBindings,
    host::platform::DirectoryAssets, host::platform::PlatformAssets,
    host::platform::PlatformBackend, host::platform::PlatformRequest, profile::Registry,
    render::raster::RgbaImage, runtime::CommentEdit, runtime::FrameBudget, runtime::FrameDriver,
    runtime::FrameOutcome, runtime::Runtime, runtime::RuntimeEvent, runtime::SaveStorage,
    runtime::SnapshotPolicy, runtime::load_start_with_fs,
};
use cmvs_host::{
    GameBootstrap, KeyBindingPolicy, SessionConfig, compose_cpu_frame, default_host_fs,
    resolve_profile_encoding_with_fs,
};
use std::{
    collections::HashSet,
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
    num::NonZeroU32,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, KeyEvent, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

const USAGE: &str = "\
Usage: cmvs-desktop <game-root> --encoding <auto|shift-jis|gbk|utf-8> [options]

Run a game in a native window. Game files are read only.

Text, audio and display:
  --encoding <name>           required; auto infers from script text and reports the result
  --font <file>               OpenType font or collection for message rendering
  --font-face <index>         zero-based collection face (default 0); requires --font
  --record-text               log glyphs and timing without drawing text; alternative to --font
  --audio <record|native>     record is the default; native needs the audio-output feature
  --input <native|record>     native is the default; record disables live input
  --renderer <cpu|wgpu>       cpu is the default; wgpu needs the wgpu-renderer feature
  --save-frame <new.png>      export the last fully presented frame to a new file on exit
  --image-archive <cpz> --preview-image <name>
                              preview an ASCII-named PB3 image

Saves:
  --user-data-dir <dir>       writable directory outside the game root; locked during use
                              CMVS uses system.dat and cmvs.cfg; CVNS uses save000.dat and cvns.cfg
  --system-save-output <new.dat>
                              export a system save once; excludes --user-data-dir
  --skip-auto-resume          skip startup auto-load for a recognised game with a known guard
  --continue-without-snapshot
                              apply live state at snapshot points; disables gameplay saving

Diagnostics:
  --exit-after <seconds>      exit after 1 to 3600 seconds
  --rand-seed <seed>          decimal u32 CRT seed (defaults to wall-clock milliseconds)
  --profile <id|alias>        select an engine profile by ID or registered game alias
  --block-presumed            stop at commands with only presumed evidence
  --block-unreviewed          stop at unreviewed commands
  -h, --help                  show this help

Use --font or --record-text to pass message output. If auto detection is
ambiguous, choose the encoding explicitly. Close the window to exit.

Build: cargo build --release -p cmvs-cli --features desktop --bin cmvs-desktop
Example: cmvs-desktop game-root --encoding auto --font font.ttf --user-data-dir user-data
See docs/guides/running.md for save behavior and platform details.";

fn warp_native_cursor(
    window: &Window,
    event_loop: &ActiveEventLoop,
    point: [f64; 2],
) -> Result<(), winit::error::ExternalError> {
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        use winit::platform::wayland::ActiveEventLoopExtWayland;
        if event_loop.is_wayland() {
            // Normal Wayland windows cannot warp the system cursor. GameInput::warp
            // has already retained the navigation point for subsequent engine frames.
            return Ok(());
        }
    }
    let _ = event_loop;
    window.set_cursor_position(winit::dpi::PhysicalPosition::new(point[0], point[1]))
}

#[cfg(feature = "wgpu-renderer")]
struct GpuDesktop {
    renderer: cmvs_wgpu::Renderer,
    presenter: cmvs_wgpu::Presenter,
    instance: cmvs_wgpu::wgpu::Instance,
    adapter: cmvs_wgpu::wgpu::Adapter,
}
struct App {
    #[cfg(windows)]
    masked_cursor: std::cell::RefCell<Option<cmvs_native::MaskedCursor>>,
    driver: FrameDriver,
    use_gpu: bool,
    #[cfg(feature = "wgpu-renderer")]
    gpu: Option<GpuDesktop>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    window: Option<Arc<Window>>,
    startup: Runtime,
    assets: DirectoryAssets,
    encoding: TextEncoding,
    input: DesktopInput,
    game_input: GameInput,
    modifiers: ModifiersState,
    comment_keys: HashSet<PhysicalKey>,
    comment_ime_allowed: bool,
    comment_preedit: bool,
    bindings: KeyBindings,
    native_input: bool,
    halted: bool,
    error: Option<String>,
    exit_after: Option<Duration>,
    deadline: Option<Instant>,
    preview_image: Option<RgbaImage>,
    device_frame: Option<Box<dyn crate::DeviceFrame>>,
    next_frame: Instant,
    last_frame: Instant,
    cursor_clock: Instant,
    cursor_visibility: cmvs_core::format::cursor::CursorVisibility,
    cursor_shown: bool,
    font: Option<crate::font::FontRenderer>,
    aspect_mode: std::cell::Cell<u32>,
    aspect_scaling: std::cell::Cell<bool>,
    render_mode: std::cell::Cell<u32>,
    fullscreen_monitor: std::cell::RefCell<crate::FullscreenMonitor>,
    windowed_size: std::cell::Cell<[u32; 2]>,
}
impl App {
    fn comment_input(&mut self, text: &str, preedit: bool) {
        let encode = |text| cmvs_font::text_codec::encode(self.encoding, text);
        let edit = if preedit {
            self.comment_preedit = !text.is_empty();
            CommentEdit::Preedit(encode(text).unwrap_or_else(|_| b"?".to_vec()))
        } else {
            self.comment_preedit = false;
            if text.is_empty() {
                return;
            }
            if text.chars().any(char::is_control) {
                CommentEdit::Invalid
            } else {
                encode(text).map_or(CommentEdit::Invalid, CommentEdit::Insert)
            }
        };
        self.startup.edit_comment(edit);
    }

    fn comment_key(&mut self, event: &KeyEvent) -> bool {
        if !self.native_input || event.state != ElementState::Pressed {
            return false;
        }
        if event.logical_key == Key::Named(NamedKey::Tab) && !self.comment_preedit {
            return self.startup.edit_comment(CommentEdit::Tab {
                backwards: self.modifiers.shift_key(),
            });
        }
        if self.startup.inline_comment_rect().is_none() {
            return false;
        }
        if self.comment_preedit {
            return true;
        }
        let shortcut = (self.modifiers.control_key() && !self.modifiers.alt_key())
            || self.modifiers.super_key();
        let edit = match &event.logical_key {
            Key::Named(
                NamedKey::Enter | NamedKey::Escape | NamedKey::ArrowUp | NamedKey::ArrowDown,
            ) => return false,
            Key::Named(NamedKey::Backspace) => CommentEdit::Backspace,
            Key::Named(NamedKey::Delete) => CommentEdit::Delete,
            Key::Named(NamedKey::ArrowLeft) => CommentEdit::Left,
            Key::Named(NamedKey::ArrowRight) => CommentEdit::Right,
            Key::Named(NamedKey::Home) => CommentEdit::Home,
            Key::Named(NamedKey::End) => CommentEdit::End,
            Key::Character(ch) if shortcut && ch.eq_ignore_ascii_case("a") => {
                CommentEdit::SelectAll
            }
            Key::Character(ch) if shortcut && ch.eq_ignore_ascii_case("v") => {
                if let Ok(text) =
                    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
                {
                    self.comment_input(&text, false);
                }
                return true;
            }
            _ => {
                if !shortcut && let Some(text) = &event.text {
                    self.comment_input(text, false);
                }
                return true;
            }
        };
        self.startup.edit_comment(edit)
    }

    fn sync_comment_ime(&mut self, window: &Window, mode: u32) {
        use crate::game_input::Viewport;
        let field = self
            .startup
            .inline_comment_rect()
            .filter(|_| self.native_input);
        if self.comment_ime_allowed != field.is_some() {
            self.comment_ime_allowed = field.is_some();
            self.comment_preedit = false;
            window.set_ime_allowed(self.comment_ime_allowed);
        }
        if let Some([x, y, w, h]) = field {
            let size = window.inner_size();
            if let Some(viewport) =
                Viewport::for_canvas(self.startup.screen_size(), [size.width, size.height], mode)
            {
                let [left, top] = viewport.to_window([x as i32, y as i32]);
                let [right, bottom] = viewport.to_window([(x + w) as i32, (y + h) as i32]);
                window.set_ime_cursor_area(
                    PhysicalPosition::new(left, top),
                    PhysicalSize::new((right - left) as u32, (bottom - top) as u32),
                );
            }
        }
    }

    fn frame(&mut self) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        if let Some(image) = &self.preview_image {
            return Ok(image.clone());
        }
        compose_cpu_frame(
            &self.startup,
            self.font.as_mut(),
            self.render_mode.get() == 1,
        )
    }
    #[cfg(feature = "wgpu-renderer")]
    fn gpu_frame(&mut self) -> Result<cmvs_wgpu::GpuFrame, Box<dyn Error + Send + Sync>> {
        let renderer = &mut self.gpu.as_mut().ok_or("GPU is not initialized")?.renderer;
        if let Some(image) = &self.preview_image {
            return renderer.preview_image(image);
        }
        cmvs_host::with_gpu_target(
            &self.startup,
            self.font.as_mut(),
            self.render_mode.get() == 1,
            |target| match target {
                cmvs_host::GpuTarget::Movie {
                    image,
                    size,
                    linear,
                } => renderer.render_image(image, size, linear),
                cmvs_host::GpuTarget::Scene(frame) => renderer.render(&frame),
            },
        )
    }
    #[cfg(feature = "wgpu-renderer")]
    fn redraw_gpu(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let window = self.window.as_ref().ok_or("no window")?.clone();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if u64::from(size.width) * u64::from(size.height) > 16 * 1024 * 1024 {
            return Err("desktop surface exceeds research pixel limit".into());
        }
        let frame = self.gpu_frame()?;
        let gpu = self.gpu.as_mut().ok_or("GPU is not initialized")?;
        if gpu.presenter.lost {
            gpu.presenter = cmvs_wgpu::Presenter::new(
                gpu.instance.create_surface(window.clone())?,
                &gpu.adapter,
                &gpu.renderer,
            )?;
        }
        let mode = if self.preview_image.is_some() {
            1
        } else {
            cmvs_core::render::viewport::presentation_mode_with_full_move(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
                self.startup.platform().window_full_move(),
            )
        };
        if gpu.presenter.present(
            &gpu.renderer,
            &frame,
            [size.width, size.height],
            mode,
            self.preview_image.is_none() && self.render_mode.get() == 1,
            self.preview_image.is_some(),
        )? && self.preview_image.is_none()
        {
            if self.startup.capture_due() {
                self.startup.supply_capture(frame.read()?)?;
            }
            self.device_frame = Some(Box::new(frame));
        }
        Ok(())
    }
    fn fail(&mut self, el: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        el.exit();
    }
    fn redraw(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // A resize/redraw can arrive between VM budget slices. Keep the previous
        // presented canvas until the engine, rather than the host budget, yields.
        if self.preview_image.is_none() && !self.driver.ready() {
            return Ok(());
        }
        #[cfg(feature = "wgpu-renderer")]
        if self.use_gpu {
            return self.redraw_gpu();
        }
        let image = self.frame()?;
        let window = self.window.as_ref().ok_or("no window")?;
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        if u64::from(size.width) * u64::from(size.height) > 16 * 1024 * 1024 {
            return Err("desktop surface exceeds research pixel limit".into());
        }
        let surface = self.surface.as_mut().ok_or("no surface")?;
        surface.resize(w, h).map_err(|e| e.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|e| e.to_string())?;
        if self.preview_image.is_some() {
            for (i, pixel) in buffer.iter_mut().enumerate() {
                let x = i % size.width as usize;
                let y = i / size.width as usize;
                *pixel = if (x / 32 + y / 32).is_multiple_of(2) {
                    0x263447
                } else {
                    0x2c3c50
                };
            }
            image.draw_fit(&mut buffer, size.width, size.height)?;
        } else {
            buffer.fill(0);
            let mode = cmvs_core::render::viewport::presentation_mode_with_full_move(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
                self.startup.platform().window_full_move(),
            );
            image.draw_scaled(
                &mut buffer,
                size.width,
                size.height,
                mode,
                self.render_mode.get() == 1,
            )?;
        }
        buffer.present().map_err(|e| e.to_string())?;
        if self.preview_image.is_none() {
            if self.startup.capture_due() {
                self.startup.supply_capture(image.clone())?;
            }
            self.device_frame = Some(Box::new(image));
        }
        Ok(())
    }
    fn preview_cursor(
        &mut self,
        el: &ActiveEventLoop,
        id: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let bitmap = if id == 0 {
            None
        } else {
            let name = self
                .startup
                .bootstrap()
                .cursor_names()
                .get((id - 1) as usize)
                .ok_or("preview cursor not registered")?;
            Some(Arc::new(CursorBitmap::decode(&self.assets.read(name)?)?))
        };
        let mut backend = NativeBackend {
            logical_size: self.startup.screen_size(),
            window: self.window.as_ref().ok_or("no window")?,
            event_loop: el,
            encoding: self.encoding,
            aspect_mode: &self.aspect_mode,
            aspect_scaling: &self.aspect_scaling,
            render_mode: &self.render_mode,
            fullscreen_monitor: &self.fullscreen_monitor,
            windowed_size: &self.windowed_size,
            fonts: self.font.as_ref(),
            device_frame: self.device_frame.as_deref(),
            #[cfg(windows)]
            masked_cursor: &self.masked_cursor,
        };
        backend.apply(&PlatformRequest::Cursor { id, bitmap })?;
        println!("# preview-only cursor shortcut={id}; VM state unchanged");
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<(), Box<dyn Error + Send + Sync>> {
            let window = Arc::new(
                el.create_window(
                    Window::default_attributes()
                        .with_title("cmvs-rs")
                        .with_inner_size(LogicalSize::new(
                            self.windowed_size.get()[0] as f64,
                            self.windowed_size.get()[1] as f64,
                        ))
                        .with_visible(false),
                )?,
            );
            // Attributes use logical units; deferred window restoration uses physical pixels.
            let size = window.inner_size();
            self.windowed_size.set([size.width, size.height]);
            if self.startup.display_preferences().fullscreen {
                let mut selection = self.fullscreen_monitor.borrow_mut();
                let monitor = crate::FullscreenMonitor::resolve(&window, selection.mode)?;
                selection.selected = monitor.clone();
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(monitor)));
            }

            #[cfg(feature = "wgpu-renderer")]
            if self.use_gpu {
                let instance = cmvs_wgpu::wgpu::Instance::new(
                    cmvs_wgpu::wgpu::InstanceDescriptor::new_without_display_handle().with_env(),
                );
                let surface = instance.create_surface(window.clone())?;
                let (renderer, adapter) = cmvs_wgpu::Renderer::new(&instance, Some(&surface))?;
                println!("# GPU adapter {:?}", adapter.get_info());
                let presenter = cmvs_wgpu::Presenter::new(surface, &adapter, &renderer)?;
                self.gpu = Some(GpuDesktop {
                    renderer,
                    presenter,
                    instance,
                    adapter,
                });
            }
            if !self.use_gpu {
                let context =
                    softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
                let surface = softbuffer::Surface::new(&context, window.clone())
                    .map_err(|e| e.to_string())?;
                self.surface = Some(surface);
                self.context = Some(context);
            }
            self.window = Some(window.clone());
            // Runtime load (0x140013bf1) applies the same capability fallback as 0x216d.
            let backend = NativeBackend {
                logical_size: self.startup.screen_size(),
                window: &window,
                event_loop: el,
                encoding: self.encoding,
                aspect_mode: &self.aspect_mode,
                aspect_scaling: &self.aspect_scaling,
                render_mode: &self.render_mode,
                fullscreen_monitor: &self.fullscreen_monitor,
                windowed_size: &self.windowed_size,
                fonts: self.font.as_ref(),
                device_frame: self.device_frame.as_deref(),
                #[cfg(windows)]
                masked_cursor: &self.masked_cursor,
            };
            let mode =
                cmvs_core::host::platform::supported_render_mode(&backend, backend.render_mode()?)?;
            self.render_mode.set(mode);
            if self.native_input && self.preview_image.is_none() {
                self.startup.set_focused(false);
            }
            self.deadline = self.exit_after.map(|d| Instant::now() + d);
            self.last_frame = Instant::now();
            self.cursor_clock = self.last_frame;
            self.next_frame = self.last_frame;
            window.set_visible(true);
            window.request_redraw();
            println!(
                "# native window created; explicit_image_preview={}; 2D scene viewport={}x{}",
                self.preview_image.is_some(),
                self.startup.screen_size()[0],
                self.startup.screen_size()[1]
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(el, error);
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                println!("# close requested");
                if self.halted || self.preview_image.is_some() || !self.startup.request_close() {
                    el.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    let renderer = if self.use_gpu { "wgpu" } else { "cpu" };
                    let vm = self.startup.machine();
                    let message = format!(
                        "{renderer} frame at {} / 0x{:x} after {} steps: {error}",
                        vm.current_name().escape_ascii(),
                        vm.pc(),
                        self.startup.steps()
                    );
                    self.fail(el, message)
                }
            }
            WindowEvent::Resized(_) => self.window.as_ref().expect("window").request_redraw(),
            WindowEvent::Focused(focus) => {
                self.input.push(InputEvent::Focus(focus));
                self.game_input.focus(focus, self.startup.input_mut());
                if self.native_input && self.preview_image.is_none() {
                    self.startup.set_focused(focus);
                    self.driver.suspend();
                }
                if !focus {
                    self.modifiers = ModifiersState::empty();
                    self.comment_input("", true);
                    self.window
                        .as_ref()
                        .expect("window")
                        .set_cursor_visible(true);
                    self.cursor_shown = true;
                }
                println!("# input focus={focus}");
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.game_input.pointer(position.x, position.y);
                self.input.push(InputEvent::Pointer {
                    x: position.x,
                    y: position.y,
                });
            }
            WindowEvent::CursorLeft { .. } => {
                self.game_input.leave();
                self.window
                    .as_ref()
                    .expect("window")
                    .set_cursor_visible(true);
                self.cursor_shown = true;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.game_input
                    .button(button, state == ElementState::Pressed);
                let button = format!("{button:?}");
                println!(
                    "# input button={button} pressed={}",
                    state == ElementState::Pressed
                );
                self.input.push(InputEvent::Button {
                    button,
                    pressed: state == ElementState::Pressed,
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                self.input.push(InputEvent::Wheel { x, y });
                self.game_input.wheel(y);
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Ime(ime) if self.comment_ime_allowed => match ime {
                Ime::Commit(text) => self.comment_input(&text, false),
                Ime::Preedit(text, _) => self.comment_input(&text, true),
                Ime::Disabled => self.comment_input("", true),
                Ime::Enabled => {}
            },
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if !pressed && self.comment_keys.remove(&event.physical_key) {
                    return;
                }
                if self.comment_key(&event) {
                    if !event.repeat {
                        self.comment_keys.insert(event.physical_key);
                    }
                    return;
                }
                if pressed && event.repeat && self.comment_keys.contains(&event.physical_key) {
                    return;
                }
                self.comment_keys.remove(&event.physical_key);
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.game_input
                        .key(event_key(key, &event.logical_key), pressed);
                }
                println!(
                    "# input key={:?} pressed={pressed} repeat={}",
                    event.physical_key, event.repeat
                );
                self.input.push(InputEvent::Key {
                    physical: format!("{:?}", event.physical_key),
                    pressed,
                    repeat: event.repeat,
                });
                if pressed && !event.repeat {
                    if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                        && (self.halted || !self.native_input || self.preview_image.is_some())
                    {
                        el.exit();
                    } else if self.halted {
                        let id = match event.physical_key {
                            PhysicalKey::Code(KeyCode::F1) => Some(0),
                            PhysicalKey::Code(KeyCode::F2) => Some(1),
                            PhysicalKey::Code(KeyCode::F3) => Some(2),
                            PhysicalKey::Code(KeyCode::F4) => Some(3),
                            PhysicalKey::Code(KeyCode::F5) => Some(4),
                            _ => None,
                        };
                        if let Some(id) = id
                            && let Err(error) = self.preview_cursor(el, id)
                        {
                            eprintln!("# cursor preview rejected: {error}");
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if el.exiting() {
            return;
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            println!("# diagnostic exit deadline reached");
            el.exit();
            return;
        }
        if !self.halted
            && let Some(window) = &self.window
        {
            let now = Instant::now();
            if now < self.next_frame {
                el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
                return;
            }
            self.next_frame = now + Duration::from_millis(16);
            let window = window.clone();
            let presentation_mode = cmvs_core::render::viewport::presentation_mode_with_full_move(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
                self.startup.platform().window_full_move(),
            );
            self.game_input.set_aspect_mode(presentation_mode);
            let cursor_auto_clear = self.startup.settings().cursor_auto_clear();
            self.driver.sample_input(self.startup.input_mut(), |input| {
                if self.native_input && self.preview_image.is_none() {
                    let size = window.inner_size();
                    self.game_input
                        .frame(&self.bindings, [size.width, size.height], input);
                    let position = self.game_input.cursor_position([size.width, size.height]);
                    let visible = self.cursor_visibility.update(
                        self.cursor_clock.elapsed().as_millis() as u32,
                        cursor_auto_clear,
                        position.is_some(),
                        position.unwrap_or_default(),
                    );
                    // Window-scoped adaptation: always show the host cursor outside the
                    // game viewport; keep the engine's stored visibility while inactive.
                    let visible = position.is_none() || visible;
                    if visible != self.cursor_shown {
                        window.set_cursor_visible(visible);
                        self.cursor_shown = visible;
                        println!("# native cursor visible={visible}");
                    }
                }
            });
            let mut yielded = false;
            let mut inactive = false;
            let outcome = self.driver.run_slice(&mut self.startup, &mut FrameBudget::time(Duration::from_millis(8)), |startup| {
                let mut backend = NativeBackend {
                    logical_size: startup.screen_size(),
                    window: &window,
                    event_loop: el,
                    encoding: self.encoding,
                    aspect_mode: &self.aspect_mode,
                    aspect_scaling: &self.aspect_scaling,
                    render_mode: &self.render_mode,
                    fullscreen_monitor: &self.fullscreen_monitor,
                    windowed_size: &self.windowed_size,
                    fonts: self.font.as_ref(),
                    device_frame: self.device_frame.as_deref(),
                    #[cfg(windows)]
                    masked_cursor: &self.masked_cursor,
                };
                let had_fade = startup.scene().fade().current().is_some();
                let outcome = startup.tick(&mut backend);
                for command in startup.take_unreviewed_notices() {
                    println!(
                        "# UNREVIEWED system command 0x{:04x} allowed with {} (default policy, first use)",
                        0x2000 + command,
                        cmvs_core::vm::dispatch::semantics_name(
                            startup.gate().profile(),
                            command
                        )
                    );
                }
                if !had_fade && let Some(fade) = startup.scene().fade().current() {
                    println!("# fade started {fade:?}");
                }
                if matches!(outcome, Ok(RuntimeEvent::MainEnteredWithoutSnapshot)) {
                    println!("# main entered at step {}; snapshot NOT built, saving unavailable", startup.steps());
                    return Ok(RuntimeEvent::Progress);
                }
                outcome
            });
            match outcome {
                Ok(FrameOutcome::Stopped(RuntimeEvent::Quit)) => {
                    println!("# script requested quit");
                    el.exit();
                    return;
                }
                Ok(FrameOutcome::Inactive) => {
                    inactive = true;
                }
                Ok(FrameOutcome::Yield) => {
                    yielded = true;
                }
                Ok(FrameOutcome::BudgetExhausted) => {}
                Ok(FrameOutcome::Stopped(event)) => {
                    self.halted = true;
                    if self.startup.gate().profile().base.is_some() {
                        for line in self.startup.gate().diagnostic_lines() {
                            println!("{line}");
                        }
                    }
                    window.set_title(&halt_title(
                        event,
                        self.startup.machine().current_name(),
                        self.startup.machine().pc(),
                    ));
                    for (slot, object) in self.startup.scene().objects() {
                        println!(
                            "# scene object[{slot}] draw={:?} children={}",
                            object.draw,
                            object.children.len()
                        );
                    }
                    println!(
                        "# STARTUP STOP {event:?}; steps={} slot={} script={} pc=0x{:x} callbacks={} known_integers={} known_bits={}",
                        self.startup.steps(),
                        self.startup.machine().current_slot(),
                        self.startup.machine().current_name().escape_ascii(),
                        self.startup.machine().pc(),
                        self.startup.control().callbacks().len(),
                        self.startup.machine().state().known_integer_count(),
                        self.startup.machine().state().known_bit_count()
                    );
                }
                Err(error) => {
                    let vm = self.startup.machine();
                    let stack_top: Vec<_> = (1..=5).map(|n| vm.stack().peek_u32(n * 4)).collect();
                    eprintln!(
                        "# VM error script={} pc=0x{:x} stack_bytes={} top_words={stack_top:?}",
                        vm.current_name().escape_ascii(),
                        vm.pc(),
                        vm.stack().len_bytes()
                    );
                    self.fail(el, error);
                    return;
                }
            }
            self.sync_comment_ime(&window, presentation_mode);
            if !yielded && !inactive && !self.halted {
                // Continue the same VM run promptly after processing window events.
                // Do not consume another input edge, advance effects, or draw here.
                self.next_frame = Instant::now();
                el.set_control_flow(ControlFlow::Poll);
                return;
            }
            let presentation_mode = cmvs_core::render::viewport::presentation_mode_with_full_move(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
                self.startup.platform().window_full_move(),
            );
            self.game_input.set_aspect_mode(presentation_mode);
            if let Some(point) = self.startup.input_mut().take_warp() {
                let size = window.inner_size();
                if let Some(position) = self.game_input.warp([size.width, size.height], point)
                    && let Err(error) = warp_native_cursor(&window, el, position)
                {
                    self.fail(el, error);
                    return;
                }
            }
            // Keep the core's inactive audio/movie polling policy. Active VM
            // budget slices, unlike inactivity, must never advance a frame.
            if (yielded || inactive) && !self.halted {
                let now = Instant::now();
                let delta = now
                    .duration_since(self.last_frame)
                    .as_millis()
                    .min(u128::from(u32::MAX)) as u32;
                self.last_frame = now;
                let had_fade = self.startup.scene().fade().current().is_some();
                let advanced = if yielded {
                    self.driver
                        .finish_frame(&mut self.startup, delta)
                        .map(|_| ())
                } else {
                    // Inactivity still services the existing audio/movie policy.
                    self.startup.advance_playback(delta)
                };
                if let Err(error) = advanced {
                    self.fail(el, error);
                    return;
                }
                if had_fade && self.startup.scene().fade().current().is_none() {
                    println!(
                        "# fade completed and removed; elapsed wall time drives frame updates"
                    );
                }
                if yielded && self.startup.capture_due() {
                    if let Err(error) = self.redraw() {
                        self.fail(el, error);
                        return;
                    }
                    // Minimized/zero-sized surfaces skip presentation; still satisfy the
                    // pending capture from the off-screen logical frame when font/GPU owns it.
                    if self.startup.capture_due() {
                        #[cfg(feature = "wgpu-renderer")]
                        let frame = if self.use_gpu {
                            self.gpu_frame().and_then(|gpu| gpu.read())
                        } else {
                            self.frame()
                        };
                        #[cfg(not(feature = "wgpu-renderer"))]
                        let frame = self.frame();
                        if let Err(error) = frame.and_then(|img| self.startup.supply_capture(img)) {
                            self.fail(el, error);
                            return;
                        }
                    }
                }
            }
            window.request_redraw();
        }
        if !self.halted {
            el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        } else if let Some(deadline) = self.deadline {
            el.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn halt_title(event: RuntimeEvent, script: &[u8], pc: usize) -> String {
    match event {
        RuntimeEvent::Unhandled { command, .. } => format!(
            "cmvs-rs paused: unsupported 0x{command:04x} at {} / 0x{pc:x}",
            script.escape_ascii()
        ),
        _ => "cmvs-rs paused: main snapshot was not built".into(),
    }
}
fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let raw_args: Vec<_> = env::args_os().skip(1).collect();
    if raw_args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| *arg == "-h" || *arg == "--help")
    {
        println!("{USAGE}");
        return Ok(());
    }
    let mut root: Option<std::ffi::OsString> = None;
    let mut positional_only = false;
    let mut encoding = None;
    let mut exit_after = None;
    let mut preview_name = None;
    let mut image_archive = None;
    let mut snapshot_policy = SnapshotPolicy::Capture;
    let mut skip_auto_resume = false;
    let mut profile = None;
    let mut block_unreviewed = false;
    let mut block_presumed = false;
    let mut system_save_output = None;
    let mut user_data_dir = None;
    let mut rand_seed: Option<u32> = None;
    let mut save_frame = None;
    let mut use_gpu = false;
    let mut native_audio = false;
    let mut native_input = true;
    let mut record_text = false;
    let mut font_path = None;
    let mut font_face = None;
    let mut args = raw_args.into_iter();
    while let Some(flag) = args.next() {
        if !positional_only && flag == "--" {
            positional_only = true;
            continue;
        }
        if positional_only || !flag.to_string_lossy().starts_with('-') {
            if root.is_some() {
                return Err("too many arguments".into());
            }
            root = Some(flag);
            continue;
        }
        if flag == "--skip-auto-resume" {
            skip_auto_resume = true;
            continue;
        }
        if flag == "--block-presumed" {
            block_presumed = true;
            continue;
        }
        if flag == "--block-unreviewed" {
            block_unreviewed = true;
            continue;
        }
        if flag == "--continue-without-snapshot" {
            snapshot_policy = SnapshotPolicy::ContinueWithoutImage;
            continue;
        }
        if flag == "--record-text" {
            record_text = true;
            continue;
        }
        if ![
            "--profile",
            "--encoding",
            "--exit-after",
            "--rand-seed",
            "--system-save-output",
            "--user-data-dir",
            "--preview-image",
            "--image-archive",
            "--save-frame",
            "--font",
            "--font-face",
            "--input",
            "--renderer",
            "--audio",
        ]
        .iter()
        .any(|name| flag == *name)
        {
            return Err(format!("unknown option {}", flag.to_string_lossy()).into());
        }
        let value = args
            .next()
            .filter(|v| !v.to_string_lossy().starts_with("--") && v != "-h")
            .ok_or_else(|| format!("{} requires a value", flag.to_string_lossy()))?;
        if flag == "--profile" {
            profile = Some(
                value
                    .into_string()
                    .map_err(|_| "profile id must be ASCII")?,
            );
        } else if flag == "--encoding" {
            encoding = Some(EncodingChoice::parse(
                value.to_str().ok_or("invalid encoding")?,
            )?);
        } else if flag == "--exit-after" {
            let seconds = value.to_str().ok_or("invalid seconds")?.parse::<u64>()?;
            if !(1..=3600).contains(&seconds) {
                return Err("exit-after must be 1..3600 seconds".into());
            }
            exit_after = Some(Duration::from_secs(seconds));
        } else if flag == "--rand-seed" {
            rand_seed = Some(value.to_str().ok_or("invalid seed")?.parse::<u32>()?);
        } else if flag == "--system-save-output" {
            system_save_output = Some(PathBuf::from(value));
        } else if flag == "--user-data-dir" {
            user_data_dir = Some(PathBuf::from(value));
        } else if flag == "--preview-image" {
            preview_name = Some(value);
        } else if flag == "--image-archive" {
            image_archive = Some(value);
        } else if flag == "--save-frame" {
            save_frame = Some(PathBuf::from(value));
        } else if flag == "--font" {
            font_path = Some(PathBuf::from(value));
        } else if flag == "--font-face" {
            font_face = Some(value.to_str().ok_or("invalid font face")?.parse::<u32>()?);
        } else if flag == "--input" {
            native_input = match value.to_str() {
                Some("native") => true,
                Some("record") => false,
                _ => return Err("input must be native or record".into()),
            };
        } else if flag == "--renderer" {
            use_gpu = match value.to_str() {
                Some("cpu") => false,
                Some("wgpu") => true,
                _ => return Err("renderer must be cpu or wgpu".into()),
            };
            if use_gpu && !cfg!(feature = "wgpu-renderer") {
                return Err("--renderer wgpu requires the wgpu-renderer build feature".into());
            }
        } else if flag == "--audio" {
            native_audio = match value.to_str() {
                Some("native") => true,
                Some("record") => false,
                _ => return Err("audio must be native or record".into()),
            };
        } else {
            return Err("unknown option".into());
        }
    }
    let root = root.ok_or(
        "usage: cmvs-desktop <game-root> --encoding <auto|shift-jis|gbk|utf-8> [options]; see --help",
    )?;
    let choice =
        encoding.ok_or("select text encoding with --encoding auto, shift-jis, gbk or utf-8")?;
    if font_face.is_some() && font_path.is_none() {
        return Err("--font-face requires --font".into());
    }
    if font_path.is_some() && record_text {
        return Err("--font cannot be combined with --record-text".into());
    }
    if system_save_output.is_some() && user_data_dir.is_some() {
        return Err("--user-data-dir and --system-save-output are mutually exclusive".into());
    }
    if preview_name.is_some() != image_archive.is_some() {
        return Err("preview-image and image-archive must be supplied together".into());
    }
    let root = PathBuf::from(root).canonicalize()?;
    let fs = default_host_fs();
    let game =
        Registry::builtin().identify_game_with_fs(&*fs, root.as_ref(), profile.as_deref())?;
    for line in game.selection.log_lines() {
        println!("{line}");
    }
    let script = load_start_with_fs(&*fs, root.as_ref(), &game.files)?;
    let encoding = resolve_profile_encoding_with_fs(
        &*fs,
        &root,
        &game.files,
        &script,
        choice.into_policy(),
        Some(game.selection.profile),
    )?;
    if choice == EncodingChoice::Auto {
        let name = cmvs_font::text_codec::name(encoding);
        println!(
            "# auto encoding={name}; inferred from game script text; override with --encoding <name> if incorrect"
        );
    }
    let font = font_path
        .as_ref()
        .map(|path| {
            crate::font::FontRenderer::read(path, font_face.unwrap_or(0))
                .map(|font| font.with_text_encoding(encoding))
        })
        .transpose()?;
    let uses_key_cfg = game.selection.profile.uses_key_cfg();
    let key_bindings = if native_input {
        KeyBindingPolicy::Required
    } else {
        KeyBindingPolicy::DefaultOnly
    };
    let preview_image = match (preview_name, image_archive) {
        (None, None) => None,
        (Some(name), Some(path)) => {
            let name = name
                .to_str()
                .filter(|v| v.is_ascii())
                .ok_or("image basename must be ASCII")?;
            let mut file = File::open(path)?;
            let index = ArchiveIndex::read(&mut file)?;
            let bytes = index.read_entry(&mut file, index.find(name.as_bytes())?)?;
            let image = pb3::decode(&bytes)?;
            println!(
                "# explicit PB3 preview={name} dimensions={}x{}; no VM draw opcode executed",
                image.width(),
                image.height()
            );
            Some(image)
        }
        _ => return Err("preview-image and image-archive must be supplied together".into()),
    };
    let saves = match (&user_data_dir, &system_save_output) {
        (Some(path), None) => SaveStorage::UserDirectory(path.clone()),
        (None, Some(path)) => SaveStorage::SystemExport(path.clone()),
        (None, None) => SaveStorage::ReadOnly,
        (Some(_), Some(_)) => return Err("select one save storage mode".into()),
    };
    let bootstrap = GameBootstrap {
        fs,
        root: root.clone(),
        game,
        script,
        encoding,
    };
    let cmvs_host::RuntimeOpenReport {
        #[cfg(feature = "audio-output")]
        mut runtime,
        #[cfg(not(feature = "audio-output"))]
        runtime,
        bindings,
        font_face_reset,
    } = bootstrap.open_runtime_detailed(
        font.as_ref(),
        SessionConfig {
            saves,
            user_font_dir: user_data_dir.clone(),
            snapshot_policy,
            block_unreviewed,
            block_presumed,
            skip_auto_resume,
            record_text: record_text || font.is_some(),
            host_capture: font.is_some() || use_gpu,
            random_seed: Some(rand_seed.unwrap_or_else(cmvs_core::host::random::host_time_seed)),
            key_bindings,
            ..SessionConfig::default()
        },
    )?;
    if native_input {
        if !uses_key_cfg {
            println!(
                "# native input enabled; CVNS uses fixed virtual key and mouse codes (no key.cfg)"
            );
        } else {
            println!(
                "# native input enabled; bindings read only from key.cfg; no registry overrides or gamepad"
            );
        }
    }
    if !block_presumed {
        println!("# research policy: legacy presumed commands allowed (default policy)");
    }
    if skip_auto_resume {
        println!(
            "# skip-auto-resume: clear the script's startup auto-load flag after system-save loading; save files unchanged"
        );
    }
    if native_audio {
        #[cfg(feature = "audio-output")]
        {
            runtime.set_audio_backend(Box::new(cmvs_audio::NativeAudio::new(&root)?))?;
            println!(
                "# native Vorbis audio enabled; movie frames follow the audio source position"
            );
        }
        #[cfg(not(feature = "audio-output"))]
        return Err("native audio requires --features desktop,audio-output".into());
    }
    let startup = runtime;
    if font_face_reset {
        println!("# configured font face not found; using the built-in default face");
    }
    if font.is_some() {
        println!(
            "# OpenType font preview enabled; explicit font file, encoding={encoding:?}, supported single-line layout; glyph masks differ from GDI"
        );
    }
    if record_text {
        println!(
            "# text RECORDING ONLY: supported single-line glyph descriptors and reveal timing; no font pixels or text drawing"
        );
    }
    let display_preferences = startup.display_preferences();
    let fullscreen_monitor_mode = startup.platform().fullscreen_monitor_mode();
    let canvas = startup.screen_size();
    let mut game_input = GameInput::default();
    game_input.set_screen_size(cmvs_core::render::viewport::ScreenSize::new(canvas)?);
    let window_size = [
        960,
        (960u64 * u64::from(canvas[1]) / u64::from(canvas[0])) as u32,
    ];
    let mut app = App {
        #[cfg(windows)]
        masked_cursor: Default::default(),
        driver: FrameDriver::default(),
        use_gpu,
        #[cfg(feature = "wgpu-renderer")]
        gpu: None,
        surface: None,
        context: None,
        window: None,
        startup,
        assets: DirectoryAssets::new(&root)?,
        encoding,
        input: DesktopInput::default(),
        game_input,
        modifiers: ModifiersState::empty(),
        comment_keys: HashSet::new(),
        comment_ime_allowed: false,
        comment_preedit: false,
        bindings,
        native_input,
        halted: false,
        error: None,
        exit_after,
        deadline: None,
        preview_image,
        device_frame: None,
        next_frame: Instant::now(),
        last_frame: Instant::now(),
        cursor_clock: Instant::now(),
        cursor_visibility: Default::default(),
        cursor_shown: true,
        font,
        aspect_mode: std::cell::Cell::new(display_preferences.aspect_mode),
        aspect_scaling: std::cell::Cell::new(display_preferences.aspect_scaling),
        render_mode: std::cell::Cell::new(display_preferences.render_mode),
        fullscreen_monitor: std::cell::RefCell::new(crate::FullscreenMonitor::new(
            fullscreen_monitor_mode,
        )),
        windowed_size: std::cell::Cell::new(window_size),
    };
    let event_loop = EventLoop::new()?;
    let outcome = event_loop.run_app(&mut app);
    if app.startup.gate().profile().base.is_some() {
        for line in app.startup.gate().diagnostic_lines() {
            println!("{line}");
        }
    }
    outcome?;
    println!(
        "# desktop exit; input_events={} dropped={} VM steps={} observed={:?}",
        app.input.events().len(),
        app.input.dropped(),
        app.startup.steps(),
        app.input.counts()
    );
    if let Some(error) = app.error {
        return Err(error.into());
    }
    if app
        .startup
        .save_preferences(cmvs_core::host::settings::DisplayPreferences {
            fullscreen: app
                .window
                .as_ref()
                .is_some_and(|w| w.fullscreen().is_some()),
            aspect_mode: app.aspect_mode.get(),
            aspect_scaling: app.aspect_scaling.get(),
            render_mode: app.render_mode.get(),
        })?
    {
        println!("# user settings saved");
    }
    if let Some(path) = save_frame {
        // Export the last successfully presented canvas, never a partial VM slice.
        let frame = if let Some(image) = &app.preview_image {
            image.clone()
        } else {
            app.device_frame
                .as_deref()
                .ok_or("no completed device frame available for export")?
                .read()?
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&frame.to_png())?;
        println!(
            "# saved frame {}x{} to {}",
            frame.width(),
            frame.height(),
            path.display()
        );
    }
    Ok(())
}
pub fn run_cli() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-desktop --help' for usage.");
        std::process::exit(1)
    }
}

use crate::encoding::EncodingChoice;
use crate::game_input::{GameInput, event_key};
use crate::{DesktopInput, InputEvent, NativeBackend, TextEncoding};
use cmvs_core::{
    cpz::ArchiveIndex,
    cursor::CursorBitmap,
    input::KeyBindings,
    pb3,
    platform::{DirectoryAssets, PlatformAssets, PlatformBackend, PlatformRequest},
    profile::Registry,
    raster::RgbaImage,
    runner::{SnapshotPolicy, Startup, StartupEvent, load_start},
};
use std::{
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
    dpi::LogicalSize,
    event::{ElementState, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const USAGE: &str = "\
Usage: cmvs-desktop <game-root> --encoding <auto|shift-jis|gbk|utf-8> [options]

Open a native research preview. Game files are read only.

Required:
  --encoding <name>       auto, shift-jis, gbk or utf-8
                         auto infers from startup script text and prints the result;
                         ambiguous samples require an explicit encoding

Display, text and audio:
  --renderer <name>       cpu (default) or wgpu (needs wgpu-renderer build feature)
  --font <file>           OpenType font file or collection for message rendering
                         requires shift-jis; cannot combine with --record-text
  --font-face <index>     zero-based collection face; requires --font (default 0)
  --record-text          record glyphs and reveal timing without drawing text
  --audio <name>          record (default) or native (needs audio-output build feature)
  --input <name>          native (default, reads key.cfg) or record (no live input)
  --save-frame <new.png>  export the displayed frame on exit; refuses overwrite
  --image-archive <CPZ> --preview-image <name>
                         show an ASCII-named PB3 image instead of the VM scene

Saves:
  --user-data-dir <dir>   keep system.dat, save slots and cmvs.cfg in this directory
                         directory is locked during use; existing user saves update
  --system-save-output <new.dat>
                         export a CSS1 save; refuses overwrite; excludes --user-data-dir
  --skip-auto-resume      disable startup auto-load in memory (verified games only)
  --continue-without-snapshot
                         research: continue without save images; saving unavailable

Run control and research:
  --exit-after <seconds>  exit after 1..3600 seconds
  --rand-seed <seed>      CRT random seed, decimal u32 (default 0)
  --profile <id>          force an engine profile; otherwise identify the game build
  --block-presumed       stop at commands with only legacy presumed evidence
  --block-unreviewed     stop at unreviewed commands
  -h, --help             show this help

Examples (quote paths containing spaces):
  cmvs-desktop game-root --encoding auto --font font.ttf --user-data-dir user-data
  cmvs-desktop game-root --encoding auto --record-text --exit-after 10

Build:
  cargo build --release --features desktop --bin cmvs-desktop
  Add audio-output and/or wgpu-renderer to enable native audio and GPU rendering.

Notes:
  Without --font or --record-text, message output stops at a barrier.
  --font currently renders Shift-JIS only; encoding selection does not add GBK/UTF-8
  message rendering. Glyph pixels differ from the original GDI backend.
  Auto detection samples complete strings in the first 256 KiB of startup text.
  Known game builds are identified from scripts, archives and executable hashes.
  Presumed/unreviewed commands run by default; absent/no-op commands always stop.
  Save snapshots are built at main entry and scenario points. Unsupported snapshot
  state stops with an error. Gameplay saving requires --user-data-dir.
  Startup auto-load is enabled unless --skip-auto-resume is specified.
  Close the window to exit. Esc follows the game's bindings.
  F1 selects the default cursor; F2..F5 preview registered cursors after a stop.";

#[cfg(feature = "wgpu-renderer")]
struct GpuDesktop {
    renderer: cmvs_wgpu::Renderer,
    presenter: cmvs_wgpu::Presenter,
    instance: cmvs_wgpu::wgpu::Instance,
    adapter: cmvs_wgpu::wgpu::Adapter,
}
struct App<'a> {
    #[cfg(windows)]
    masked_cursor: std::cell::RefCell<Option<cmvs_native::MaskedCursor>>,
    frame_boundary: crate::frame_boundary::FrameBoundary,
    use_gpu: bool,
    #[cfg(feature = "wgpu-renderer")]
    gpu: Option<GpuDesktop>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    window: Option<Arc<Window>>,
    startup: Startup<'a>,
    assets: DirectoryAssets,
    encoding: TextEncoding,
    input: DesktopInput,
    game_input: GameInput,
    bindings: KeyBindings,
    native_input: bool,
    input_due: bool,
    halted: bool,
    error: Option<String>,
    exit_after: Option<Duration>,
    deadline: Option<Instant>,
    preview_image: Option<RgbaImage>,
    device_frame: Option<Box<dyn crate::DeviceFrame>>,
    next_frame: Instant,
    last_frame: Instant,
    cursor_clock: Instant,
    cursor_visibility: cmvs_core::cursor::CursorVisibility,
    cursor_shown: bool,
    font: Option<crate::font::FontRenderer>,
    aspect_mode: std::cell::Cell<u32>,
    aspect_scaling: std::cell::Cell<bool>,
    render_mode: std::cell::Cell<u32>,
    fullscreen_monitor: std::cell::RefCell<crate::FullscreenMonitor>,
    windowed_size: std::cell::Cell<[u32; 2]>,
}
impl App<'_> {
    fn frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        if let Some(image) = &self.preview_image {
            return Ok(image.clone());
        }
        if let Some(image) = self.startup.movie_frame() {
            return Ok(movie_canvas(
                image,
                self.startup.screen_size(),
                self.render_mode.get() == 1,
            )?);
        }
        if let Some(font) = &mut self.font {
            return Ok(font.render(
                self.startup.scene(),
                self.startup.camera(),
                self.startup.messages(),
            )?);
        }
        Ok(self.startup.render_frame()?)
    }
    #[cfg(feature = "wgpu-renderer")]
    fn gpu_frame(&mut self) -> Result<cmvs_wgpu::GpuFrame, Box<dyn Error>> {
        let renderer = &mut self.gpu.as_mut().ok_or("GPU is not initialized")?.renderer;
        if let Some(image) = &self.preview_image {
            return renderer.preview_image(image);
        }
        if let Some(image) = self.startup.movie_frame() {
            return renderer.render_image(
                image,
                self.startup.screen_size(),
                self.render_mode.get() == 1,
            );
        }
        let objects = match &mut self.font {
            Some(font) => font.prepare_objects(self.startup.messages())?,
            None => Vec::new(),
        };
        let mut additional: Vec<_> = objects.iter().collect();
        if self.font.is_some() {
            additional.extend(self.startup.messages().visible_window_objects());
        }
        let frame = cmvs_core::scene_render::prepare_scene(
            self.startup.scene(),
            Some(self.startup.camera()),
            &additional,
            self.startup.screen_size()[0],
            self.startup.screen_size()[1],
        )?;
        renderer.render(&frame)
    }
    #[cfg(feature = "wgpu-renderer")]
    fn redraw_gpu(&mut self) -> Result<(), Box<dyn Error>> {
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
            cmvs_core::viewport::presentation_mode(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
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
            self.device_frame = Some(Box::new(frame));
        }
        Ok(())
    }
    fn fail(&mut self, el: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        el.exit();
    }
    fn redraw(&mut self) -> Result<(), Box<dyn Error>> {
        // A resize/redraw can arrive between VM budget slices. Keep the previous
        // presented canvas until the engine, rather than the host budget, yields.
        if self.preview_image.is_none() && !self.frame_boundary.ready() {
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
        surface.resize(w, h)?;
        let mut buffer = surface.buffer_mut()?;
        for (i, pixel) in buffer.iter_mut().enumerate() {
            let x = i % size.width as usize;
            let y = i / size.width as usize;
            *pixel = if (x / 32 + y / 32).is_multiple_of(2) {
                0x263447
            } else {
                0x2c3c50
            };
        }
        if self.preview_image.is_none() {
            buffer.fill(0);
        }
        if self.preview_image.is_some() {
            image.draw_fit(&mut buffer, size.width, size.height)?;
        } else {
            let mode = cmvs_core::viewport::presentation_mode(
                window.fullscreen().is_some(),
                self.aspect_scaling.get(),
                self.aspect_mode.get(),
            );
            image.draw_scaled(
                &mut buffer,
                size.width,
                size.height,
                mode,
                self.render_mode.get() == 1,
            )?;
        }
        buffer.present()?;
        if self.preview_image.is_none() {
            self.device_frame = Some(Box::new(image));
        }
        Ok(())
    }
    fn preview_cursor(&mut self, el: &ActiveEventLoop, id: u32) -> Result<(), Box<dyn Error>> {
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

/// Movies join the same logical device canvas that readback sees, before window
/// fitting. Videos retain aspect ratio inside the logical game canvas.
pub fn movie_canvas(
    image: &RgbaImage,
    [width, height]: [u32; 2],
    linear: bool,
) -> Result<RgbaImage, cmvs_core::FormatError> {
    if [image.width(), image.height()] == [width, height] {
        return Ok(image.clone());
    }
    let mut target = vec![0; (width * height) as usize];
    image.draw_scaled(&mut target, width, height, 1, linear)?;
    let pixels = target
        .into_iter()
        .flat_map(|word| {
            let [b, g, r, _] = word.to_le_bytes();
            [r, g, b, 255]
        })
        .collect();
    RgbaImage::new(width, height, pixels)
}
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<(), Box<dyn Error>> {
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
                let context = softbuffer::Context::new(window.clone())?;
                let surface = softbuffer::Surface::new(&context, window.clone())?;
                self.surface = Some(surface);
                self.context = Some(context);
            }
            self.window = Some(window.clone());
            // Startup load (0x140013bf1) applies the same capability fallback as 0x216d.
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
                cmvs_core::platform::supported_render_mode(&backend, backend.render_mode()?)?;
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
                    self.input_due = true;
                }
                if !focus {
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
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
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
            self.game_input
                .set_aspect_mode(cmvs_core::viewport::presentation_mode(
                    window.fullscreen().is_some(),
                    self.aspect_scaling.get(),
                    self.aspect_mode.get(),
                ));
            if self.input_due && self.native_input && self.preview_image.is_none() {
                let size = window.inner_size();
                self.game_input.frame(
                    &self.bindings,
                    [size.width, size.height],
                    self.startup.input_mut(),
                );
                let position = self.game_input.cursor_position([size.width, size.height]);
                let visible = self.cursor_visibility.update(
                    self.cursor_clock.elapsed().as_millis() as u32,
                    self.startup.settings().cursor_auto_clear(),
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
                self.input_due = false;
            }
            let mut yielded = false;
            let mut inactive = false;
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(8) {
                let mut backend = NativeBackend {
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
                let had_fade = self.startup.scene().fade().current().is_some();
                let outcome = self.startup.tick(&mut backend);
                for command in self.startup.take_unreviewed_notices() {
                    println!(
                        "# UNREVIEWED system command 0x{:04x} allowed with {} (default policy, first use)",
                        0x2000 + command,
                        cmvs_core::dispatch::semantics_name(
                            self.startup.gate().selection().profile,
                            command
                        )
                    );
                }
                if !had_fade && let Some(fade) = self.startup.scene().fade().current() {
                    println!("# fade started {fade:?}");
                }
                if let Ok(event) = &outcome {
                    self.frame_boundary.observe(*event);
                }
                match outcome {
                    Ok(StartupEvent::Quit) => {
                        println!("# script requested quit");
                        el.exit();
                        return;
                    }
                    Ok(StartupEvent::Inactive) => {
                        inactive = true;
                        break;
                    }
                    Ok(StartupEvent::Progress) => {}
                    Ok(StartupEvent::Yield) => {
                        yielded = true;
                        self.input_due = true;
                        break;
                    }
                    Ok(StartupEvent::SnapshotCaptured) => {}
                    Ok(StartupEvent::MainEnteredWithoutSnapshot) => println!(
                        "# main entered at step {}; snapshot NOT built, saving unavailable",
                        self.startup.steps()
                    ),
                    Ok(event) => {
                        self.halted = true;
                        if self.startup.gate().selection().profile.base.is_some() {
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
                            self.startup.machine().state().integers().len(),
                            self.startup.machine().state().bits().len()
                        );
                        break;
                    }
                    Err(error) => {
                        let vm = self.startup.machine();
                        let stack_top: Vec<_> =
                            (1..=5).map(|n| vm.stack().peek_u32(n * 4)).collect();
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
            }
            if !yielded && !inactive && !self.halted {
                // Continue the same VM run promptly after processing window events.
                // Do not consume another input edge, advance effects, or draw here.
                self.next_frame = Instant::now();
                el.set_control_flow(ControlFlow::Poll);
                return;
            }
            self.game_input
                .set_aspect_mode(cmvs_core::viewport::presentation_mode(
                    window.fullscreen().is_some(),
                    self.aspect_scaling.get(),
                    self.aspect_mode.get(),
                ));
            if let Some(point) = self.startup.input_mut().take_warp() {
                let size = window.inner_size();
                if let Some([x, y]) = self.game_input.warp([size.width, size.height], point)
                    && let Err(error) =
                        window.set_cursor_position(winit::dpi::PhysicalPosition::new(x, y))
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
                if let Err(error) = self.startup.advance_decoded_frame(delta) {
                    self.fail(el, error);
                    return;
                }
                if had_fade && self.startup.scene().fade().current().is_none() {
                    println!(
                        "# fade completed and removed; elapsed wall time drives frame updates"
                    );
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

fn halt_title(event: StartupEvent, script: &[u8], pc: usize) -> String {
    match event {
        StartupEvent::Unhandled { command, .. } => format!(
            "cmvs-rs paused: unsupported 0x{command:04x} at {} / 0x{pc:x}",
            script.escape_ascii()
        ),
        _ => "cmvs-rs paused: main snapshot was not built".into(),
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let root = args.next().ok_or(
        "usage: cmvs-desktop <game-root> --encoding <auto|shift-jis|gbk|utf-8> [options]; see --help",
    )?;
    if root == "--help" || root == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    if root.to_string_lossy().starts_with('-') {
        return Err(format!("unknown option {}", root.to_string_lossy()).into());
    }
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
    let mut rand_seed = 0;
    let mut save_frame = None;
    let mut use_gpu = false;
    let mut native_audio = false;
    let mut native_input = true;
    let mut record_text = false;
    let mut font_path = None;
    let mut font_face = None;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            return Ok(());
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
            rand_seed = value.to_str().ok_or("invalid seed")?.parse::<u32>()?;
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
    let choice =
        encoding.ok_or("select text encoding with --encoding auto, shift-jis, gbk or utf-8")?;
    if font_face.is_some() && font_path.is_none() {
        return Err("--font-face requires --font".into());
    }
    if font_path.is_some()
        && (record_text
            || matches!(choice, EncodingChoice::Explicit(encoding) if encoding != TextEncoding::ShiftJis))
    {
        return Err(
            "--font requires shift-jis encoding and cannot be combined with --record-text".into(),
        );
    }
    if system_save_output.is_some() && user_data_dir.is_some() {
        return Err("--user-data-dir and --system-save-output are mutually exclusive".into());
    }
    if preview_name.is_some() != image_archive.is_some() {
        return Err("preview-image and image-archive must be supplied together".into());
    }
    let root = PathBuf::from(root).canonicalize()?;
    let game = Registry::builtin().identify_game(root.as_ref(), profile.as_deref())?;
    for line in game.selection.log_lines() {
        println!("{line}");
    }
    let script = load_start(root.as_ref(), &game.files)?;
    let encoding = choice.resolve(script.text())?;
    if choice == EncodingChoice::Auto {
        let name = match encoding {
            TextEncoding::ShiftJis => "shift-jis",
            TextEncoding::Gbk => "gbk",
            TextEncoding::Utf8 => "utf-8",
        };
        println!(
            "# auto encoding={name}; inferred from startup script text; override with --encoding <name> if incorrect"
        );
    }
    if font_path.is_some() && (record_text || encoding != TextEncoding::ShiftJis) {
        return Err(
            "--font requires shift-jis encoding and cannot be combined with --record-text".into(),
        );
    }
    let font = font_path
        .as_ref()
        .map(|path| crate::font::FontRenderer::read(path, font_face.unwrap_or(0)))
        .transpose()?;
    let bindings = if native_input {
        use std::io::Read;
        let mut text = String::new();
        File::open(root.join("key.cfg"))?
            .take(65537)
            .read_to_string(&mut text)?;
        let bindings = KeyBindings::parse(&text)?;
        println!(
            "# native input enabled; bindings read only from key.cfg; no registry overrides or gamepad"
        );
        bindings
    } else {
        KeyBindings::default()
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
    let mut startup = Startup::new(&script, root.as_ref(), &game, 0)?;
    startup.set_block_unreviewed(block_unreviewed);
    startup.set_block_presumed(block_presumed);
    if !block_presumed {
        println!("# research policy: legacy presumed commands allowed (default policy)");
    }
    if let Some(path) = &user_data_dir {
        startup.set_user_data_directory(path)?;
    }
    if let Some(path) = &system_save_output {
        startup.set_system_save_output(path)?;
    }
    startup.set_skip_auto_resume(skip_auto_resume)?;
    if skip_auto_resume {
        println!(
            "# skip-auto-resume: explicitly clear Amatsutsumi's in-memory startup auto-load flag after system-save loading; save files unchanged"
        );
    }
    startup.set_decode_movie_frames(true);
    startup.set_mgv_factory(Box::new(crate::video::TheoraFactory));
    if native_audio {
        #[cfg(feature = "audio-output")]
        {
            startup.set_audio_backend(Box::new(cmvs_audio::NativeAudio::new(&root)?))?;
            println!(
                "# native Vorbis audio enabled; movie frames follow the audio source position"
            );
        }
        #[cfg(not(feature = "audio-output"))]
        return Err("native audio requires --features desktop,audio-output".into());
    }
    startup.set_snapshot_policy(snapshot_policy);
    startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
    startup.set_record_text(record_text || font.is_some());
    if let Some(font) = &font {
        if let Some(directory) = &user_data_dir {
            font.set_user_directory(directory)?;
        }
        startup.set_glyph_metrics(font.metrics());
        // 0x14000b666: a configured face that cannot be found becomes the default.
        if !font.has_face(startup.settings().font_face()) {
            println!("# configured font face not found; using the built-in default face");
            startup.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)?;
        }
    }
    if font.is_some() {
        println!(
            "# OpenType font preview enabled; explicit font file, Shift-JIS, supported single-line layout; glyph masks differ from GDI"
        );
    }
    if record_text {
        println!(
            "# text RECORDING ONLY: supported single-line glyph descriptors and reveal timing; no font pixels or text drawing"
        );
    }
    startup.set_random_seed(rand_seed);
    let display_preferences = startup.display_preferences();
    let fullscreen_monitor_mode = startup.platform().fullscreen_monitor_mode();
    let canvas = startup.screen_size();
    let mut game_input = GameInput::default();
    game_input.set_screen_size(cmvs_core::viewport::ScreenSize::new(canvas)?);
    let window_size = [
        960,
        (960u64 * u64::from(canvas[1]) / u64::from(canvas[0])) as u32,
    ];
    let mut app = App {
        #[cfg(windows)]
        masked_cursor: Default::default(),
        frame_boundary: Default::default(),
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
        bindings,
        native_input,
        input_due: true,
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
    if app.startup.gate().selection().profile.base.is_some() {
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
        .save_preferences(cmvs_core::settings::DisplayPreferences {
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

#[cfg(test)]
mod readback_tests {
    use super::*;

    #[test]
    fn movie_canvas_keeps_logical_device_dimensions_and_letterboxes_small_videos() {
        let video = RgbaImage::new(2, 1, [200, 100, 40, 255].repeat(2)).unwrap();
        for [width, height] in [[1280, 720], [1024, 768]] {
            for linear in [false, true] {
                let frame = movie_canvas(&video, [width, height], linear).unwrap();
                assert_eq!([frame.width(), frame.height()], [width, height]);
                let pixel = |y| {
                    let at = ((y * width + width / 2) * 4) as usize;
                    &frame.pixels()[at..at + 4]
                };
                let top = (height - width / 2) / 2;
                assert_eq!(pixel(top - 1), [0, 0, 0, 255]);
                assert_eq!(pixel(top), [200, 100, 40, 255]);
                assert_eq!(pixel(height - top - 1), [200, 100, 40, 255]);
                assert_eq!(pixel(height - top), [0, 0, 0, 255]);
                assert_eq!(
                    movie_canvas(&frame, [width, height], linear).unwrap(),
                    frame
                );
            }
        }
    }
}

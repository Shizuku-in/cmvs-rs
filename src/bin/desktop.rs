use cmvs_rs::{
    cpz::Cpz6Index,
    cursor::CursorBitmap,
    desktop::{DesktopInput, InputEvent, NativeBackend, TextEncoding},
    pb3,
    platform::{DirectoryAssets, PlatformAssets, PlatformBackend, PlatformRequest},
    raster::RgbaImage,
    runner::{Startup, StartupEvent, load_start},
};
use std::{
    env,
    error::Error,
    fs::File,
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

struct App<'a> {
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    window: Option<Arc<Window>>,
    startup: Startup<'a>,
    assets: DirectoryAssets,
    encoding: TextEncoding,
    input: DesktopInput,
    halted: bool,
    error: Option<String>,
    exit_after: Option<Duration>,
    deadline: Option<Instant>,
    preview_image: Option<RgbaImage>,
}
impl App<'_> {
    fn fail(&mut self, el: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        el.exit();
    }
    fn redraw(&mut self) -> Result<(), Box<dyn Error>> {
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
        if let Some(image) = &self.preview_image {
            image.draw_fit(&mut buffer, size.width, size.height)?;
        }
        let bar_start = size.height.saturating_sub(10) as usize * size.width as usize;
        buffer[bar_start..].fill(if self.halted { 0x9c7529 } else { 0x3c7fa2 });
        buffer.present()?;
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
            window: self.window.as_ref().ok_or("no window")?,
            event_loop: el,
            encoding: self.encoding,
        };
        backend.apply(&PlatformRequest::Cursor { id, bitmap })?;
        println!("# preview-only cursor shortcut={id}; VM state unchanged");
        Ok(())
    }
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
                        .with_inner_size(LogicalSize::new(960.0, 540.0))
                        .with_visible(false),
                )?,
            );
            let context = softbuffer::Context::new(window.clone())?;
            let surface = softbuffer::Surface::new(&context, window.clone())?;
            self.surface = Some(surface);
            self.context = Some(context);
            self.window = Some(window.clone());
            self.deadline = self.exit_after.map(|d| Instant::now() + d);
            window.set_visible(true);
            window.request_redraw();
            println!(
                "# native window created; explicit_image_preview={}; no CMVS scene rendering",
                self.preview_image.is_some()
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
                el.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(el, error)
                }
            }
            WindowEvent::Resized(_) => self.window.as_ref().expect("window").request_redraw(),
            WindowEvent::Focused(focus) => {
                self.input.push(InputEvent::Focus(focus));
                println!("# input focus={focus}");
            }
            WindowEvent::CursorMoved { position, .. } => self.input.push(InputEvent::Pointer {
                x: position.x,
                y: position.y,
            }),
            WindowEvent::MouseInput { state, button, .. } => {
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
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
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
                    if event.physical_key == PhysicalKey::Code(KeyCode::Escape) {
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
            let window = window.clone();
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(8) {
                let mut backend = NativeBackend {
                    window: &window,
                    event_loop: el,
                    encoding: self.encoding,
                };
                match self.startup.tick(&mut backend) {
                    Ok(StartupEvent::Progress) => {}
                    Ok(StartupEvent::Yield) => break,
                    Ok(event) => {
                        self.halted = true;
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
                        self.fail(el, error);
                        return;
                    }
                }
            }
            window.request_redraw();
        }
        if !self.halted {
            el.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(1),
            ));
        } else if let Some(deadline) = self.deadline {
            el.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let root = args.next().ok_or(
        "usage: cmvs-desktop <game-root> --encoding <shift-jis|gbk|utf-8> [--exit-after <seconds>]",
    )?;
    if root == "--help" {
        println!(
            "Usage: cmvs-desktop <game-root> --encoding <shift-jis|gbk|utf-8> [--exit-after <1..3600 seconds>] [--image-archive <CPZ6> --preview-image <ASCII-basename>]\nNative research preview only. Runs bounded startup with virtual tick 0; stops at unknown commands or main snapshot barrier. Without an explicit image, shows a diagnostic checkerboard. Image preview supports PB3 type 1 only; aspect fit and straight-alpha blending are preview policies, not VM scene commands. F1=default cursor, F2..F5=registered cursor previews after stop, Esc/close=exit. Raw window input is observed but not injected into CMVS opcodes. All game files read-only."
        );
        return Ok(());
    }
    let mut encoding = None;
    let mut exit_after = None;
    let mut preview_name = None;
    let mut image_archive = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        if flag == "--encoding" {
            encoding = Some(TextEncoding::parse(
                value.to_str().ok_or("invalid encoding")?,
            )?);
        } else if flag == "--exit-after" {
            let seconds = value.to_str().ok_or("invalid seconds")?.parse::<u64>()?;
            if !(1..=3600).contains(&seconds) {
                return Err("exit-after must be 1..3600 seconds".into());
            }
            exit_after = Some(Duration::from_secs(seconds));
        } else if flag == "--preview-image" {
            preview_name = Some(value);
        } else if flag == "--image-archive" {
            image_archive = Some(value);
        } else {
            return Err("unknown option".into());
        }
    }
    let encoding = encoding.ok_or("select title encoding explicitly with --encoding")?;
    let root = PathBuf::from(root).canonicalize()?;
    let preview_image = match (preview_name, image_archive) {
        (None, None) => None,
        (Some(name), Some(path)) => {
            let name = name
                .to_str()
                .filter(|v| v.is_ascii())
                .ok_or("image basename must be ASCII")?;
            let mut file = File::open(path)?;
            let index = Cpz6Index::read(&mut file)?;
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
    let script = load_start(&root)?;
    let startup = Startup::new(&script, &root, 0)?;
    let mut app = App {
        surface: None,
        context: None,
        window: None,
        startup,
        assets: DirectoryAssets::new(&root)?,
        encoding,
        input: DesktopInput::default(),
        halted: false,
        error: None,
        exit_after,
        deadline: None,
        preview_image,
    };
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
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
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1)
    }
}

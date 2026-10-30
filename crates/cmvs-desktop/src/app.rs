use crate::game_input::{GameInput, event_key};
use crate::{DesktopInput, InputEvent, NativeBackend, TextEncoding};
use cmvs_core::{
    cpz::Cpz6Index,
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

struct App<'a> {
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
    next_frame: Instant,
    last_frame: Instant,
    cursor_clock: Instant,
    cursor_visibility: cmvs_core::cursor::CursorVisibility,
    cursor_shown: bool,
    font: Option<crate::font::FontRenderer>,
    aspect_mode: std::cell::Cell<u32>,
}
impl App<'_> {
    fn frame(&mut self) -> Result<RgbaImage, Box<dyn Error>> {
        if let Some(image) = &self.preview_image {
            return Ok(image.clone());
        }
        if let Some(image) = self.startup.movie_frame() {
            return Ok(image.clone());
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
    fn fail(&mut self, el: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        el.exit();
    }
    fn redraw(&mut self) -> Result<(), Box<dyn Error>> {
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
            let mode = if window.fullscreen().is_some() {
                self.aspect_mode.get()
            } else {
                1
            };
            image.draw_aspect(&mut buffer, size.width, size.height, mode)?;
        }
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
            aspect_mode: &self.aspect_mode,
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
            self.last_frame = Instant::now();
            self.cursor_clock = self.last_frame;
            self.next_frame = self.last_frame;
            window.set_visible(true);
            window.request_redraw();
            println!(
                "# native window created; explicit_image_preview={}; 2D scene viewport=1280x720",
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
                self.game_input.focus(focus, self.startup.input_mut());
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
            let delta = now
                .duration_since(self.last_frame)
                .as_millis()
                .min(u128::from(u32::MAX)) as u32;
            self.last_frame = now;
            self.next_frame = now + Duration::from_millis(16);
            let window = window.clone();
            self.game_input
                .set_aspect_mode(if window.fullscreen().is_some() {
                    self.aspect_mode.get()
                } else {
                    1
                });
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
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(8) {
                let mut backend = NativeBackend {
                    window: &window,
                    event_loop: el,
                    encoding: self.encoding,
                    aspect_mode: &self.aspect_mode,
                };
                let had_fade = self.startup.scene().fade().current().is_some();
                let outcome = self.startup.tick(&mut backend);
                for command in self.startup.take_unreviewed_notices() {
                    println!(
                        "# UNREVIEWED system command 0x{:04x} allowed with base semantics (--allow-unreviewed, first use)",
                        0x2000 + command
                    );
                }
                if !had_fade && let Some(fade) = self.startup.scene().fade().current() {
                    println!("# fade started {fade:?}");
                }
                match outcome {
                    Ok(StartupEvent::Progress) => {}
                    Ok(StartupEvent::Yield) => {
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
            self.game_input
                .set_aspect_mode(if window.fullscreen().is_some() {
                    self.aspect_mode.get()
                } else {
                    1
                });
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
            if !self.halted {
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
/// Save labels use UTC: the standard library has no local time zone without platform calls.
fn utc_save_time() -> cmvs_core::runner::SaveTime {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Civil date from days since 1970-01-01 (proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [
        year as u16,
        month as u16,
        day as u16,
        (rest / 3600) as u16,
        (rest / 60 % 60) as u16,
        (rest % 60) as u16,
    ]
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
        "usage: cmvs-desktop <game-root> --encoding <shift-jis|gbk|utf-8> [--exit-after <seconds>]",
    )?;
    if root == "--help" {
        println!(
            "Usage: cmvs-desktop <game-root> --encoding <shift-jis|gbk|utf-8> [--exit-after <1..3600 seconds>] [--image-archive <CPZ6> --preview-image <ASCII-basename>] [--profile <id>] [--allow-unreviewed] [--continue-without-snapshot] [--skip-auto-resume] [--system-save-output <new.dat> | --user-data-dir <directory>] [--rand-seed <seed>] [--save-frame <new.png>] [--audio <record|native>] [--input <native|record>] [--record-text | --font <file> [--font-face <index>]]\nNative research preview only. The engine profile is identified read-only from the start script, script archive and executables; --profile selects one explicitly for research, and --allow-unreviewed runs commands not yet reviewed for that profile with the base semantics, logging each command's first use. Absent and no-op commands always stop. Runs bounded startup with virtual callback tick 0 and explicit wall-time scene updates; stops at unknown commands. The engine save snapshot is built at main entry (0x2080) and scenario points (0x22b2); live state without a save format stops with an error. --continue-without-snapshot (research) enters main without building a save image; gameplay saving then stays unavailable. --system-save-output explicitly enables one mode-1 CSS1 export to a new file outside the game root after loading a header template; existing files are never replaced, and later system-save reads use that export. --user-data-dir instead enables a locked independent user directory, reads its system.dat on restart (falling back to the read-only game template only if absent), reads and writes gameplay slots saveNNN.dat there (labels in UTC; slot 999 keeps an overwritten slot, slot 998 the state before a load), and permits repeated CSS1 saves with a system.bak backup. These two output modes are mutually exclusive. Without either option 0x2129 stays a barrier and gameplay saves fail. --skip-auto-resume explicitly disables startup auto-load in memory for games whose record verifies it (Amatsutsumi), preserving all save files and other system-save settings; without it the startup auto-load restores the saved slot. --rand-seed sets the CRT rand() research seed (default 0). Without an explicit image, presents the verified 2D scene (top-level objects and their children) and CMV6 JBPD movies on a 1280x720 research viewport; includes timed full-screen color fades; use a release build for movie decoding; transforms, other transitions and alternate blend modes remain unsupported. --save-frame writes the frame shown at exit, in script coordinates, to a new PNG file (never overwritten); use it instead of screen captures to check rendering. Image preview supports PB3 type 1 only; aspect fit and straight-alpha blending are preview policies, not VM scene commands. F1=default cursor, F2..F5=registered cursor previews after stop, Esc/close=exit. --audio record (default) records only; --audio native requires the audio-output feature and opens the default device for Vorbis SE/BGM/movie playback, using audio source position for movie frames. Device-buffer latency is not calibrated. --input native (default) maps key.cfg bindings and pointer events to CMVS actions; --input record observes input without injecting actions. --record-text explicitly records only the supported message glyph descriptors and reveal timing; no font pixels or text drawing. --font loads an explicit OpenType file/collection face and previews the supported Shift-JIS messages with wrapping and ruby, with flat colors and outlines. Its glyph masks differ from GDI. --font-face defaults to 0. Default message output stays a barrier. All game files read-only."
        );
        return Ok(());
    }
    let mut encoding = None;
    let mut exit_after = None;
    let mut preview_name = None;
    let mut image_archive = None;
    let mut snapshot_policy = SnapshotPolicy::Capture;
    let mut skip_auto_resume = false;
    let mut profile = None;
    let mut allow_unreviewed = false;
    let mut system_save_output = None;
    let mut user_data_dir = None;
    let mut rand_seed = 0;
    let mut save_frame = None;
    let mut native_audio = false;
    let mut native_input = true;
    let mut record_text = false;
    let mut font_path = None;
    let mut font_face = None;
    while let Some(flag) = args.next() {
        if flag == "--skip-auto-resume" {
            skip_auto_resume = true;
            continue;
        }
        if flag == "--allow-unreviewed" {
            allow_unreviewed = true;
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
        let value = args.next().ok_or("missing option value")?;
        if flag == "--profile" {
            profile = Some(
                value
                    .into_string()
                    .map_err(|_| "profile id must be ASCII")?,
            );
        } else if flag == "--encoding" {
            encoding = Some(TextEncoding::parse(
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
    let encoding = encoding.ok_or("select title encoding explicitly with --encoding")?;
    if font_face.is_some() && font_path.is_none() {
        return Err("--font-face requires --font".into());
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
    let root = PathBuf::from(root).canonicalize()?;
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
    let game = Registry::builtin().identify_game(root.as_ref(), profile.as_deref())?;
    for line in game.selection.log_lines() {
        println!("{line}");
    }
    let script = load_start(root.as_ref(), &game.files)?;
    if system_save_output.is_some() && user_data_dir.is_some() {
        return Err("--user-data-dir and --system-save-output are mutually exclusive".into());
    }
    let mut startup = Startup::new(&script, root.as_ref(), &game, 0)?;
    startup.set_allow_unreviewed(allow_unreviewed);
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
    startup.set_save_clock(Box::new(utc_save_time));
    startup.set_record_text(record_text || font.is_some());
    if let Some(font) = &font {
        startup.set_glyph_metrics(font.metrics());
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
    let mut app = App {
        surface: None,
        context: None,
        window: None,
        startup,
        assets: DirectoryAssets::new(&root)?,
        encoding,
        input: DesktopInput::default(),
        game_input: GameInput::default(),
        bindings,
        native_input,
        input_due: true,
        halted: false,
        error: None,
        exit_after,
        deadline: None,
        preview_image,
        next_frame: Instant::now(),
        last_frame: Instant::now(),
        cursor_clock: Instant::now(),
        cursor_visibility: Default::default(),
        cursor_shown: true,
        font,
        aspect_mode: std::cell::Cell::new(1),
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
    if let Some(path) = save_frame {
        // Script coordinates before window fitting: exactly what the renderer produced.
        let frame = app.frame()?;
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
        eprintln!("Error: {error}");
        std::process::exit(1)
    }
}

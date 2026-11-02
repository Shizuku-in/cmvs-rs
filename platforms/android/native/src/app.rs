#![forbid(unsafe_code)]
use crate::backend::MobileBackend;
use android_activity::{
    AndroidApp, InputStatus, MainEvent, PollEvent,
    input::{InputEvent, MotionAction},
};
use cmvs_audio::{HostAudioControl, NativeAudio};
use cmvs_core::{
    input::KeyBindings,
    platform::resolve_under,
    profile::Registry,
    runner::{Startup, StartupEvent, load_start},
};
use cmvs_font::font::FontRenderer;
use cmvs_input::{TouchEvent, TouchInput, TouchPhase, Viewport};
use cmvs_native::AndroidWindow;
use cmvs_wgpu::{GpuFrame, Presenter, Renderer, wgpu};
use std::{
    cell::RefCell,
    error::Error,
    fs::File,
    io::Read,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const FRAME_TIME: Duration = Duration::from_millis(16);
struct Graphics {
    // Drop the surface before its owning native window.
    presenter: Presenter,
    frame: Option<Arc<GpuFrame>>,
    renderer: Renderer,
    adapter: wgpu::Adapter,
    instance: wgpu::Instance,
    window: Arc<AndroidWindow>,
}
struct Player<'a> {
    audio_control: HostAudioControl,
    graphics: Option<Graphics>,
    startup: Startup<'a>,
    backend: MobileBackend,
    touch: TouchInput,
    bindings: KeyBindings,
    resumed: bool,
    focused: bool,
    next_frame: Instant,
    last_frame: Instant,
    input_due: bool,
    draw_ready: bool,
    exit: bool,
}
impl Player<'_> {
    fn active(&self) -> bool {
        self.resumed && self.focused && self.graphics.is_some()
    }
    fn sync_input(&mut self) {
        let active = self.active();
        self.audio_control.set_active(active);
        self.touch.focus(active, self.startup.input_mut());
        self.startup.set_focused(active);
        self.next_frame = Instant::now();
        self.last_frame = self.next_frame;
    }
    fn release_surface(&mut self) {
        self.audio_control.set_active(false);
        self.touch.focus(false, self.startup.input_mut());
        self.startup.set_focused(false);
        self.graphics = None;
        log::info!("Surface released");
    }
    fn initialize(&mut self, app: &AndroidApp) -> Result<()> {
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = Arc::new(AndroidWindow::new(
            app.native_window().ok_or("no native window")?,
        ));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let (mut renderer, adapter) = Renderer::new(&instance, Some(&surface))?;
        log::info!("GPU: {}", adapter.get_info().name);
        let presenter = Presenter::new(surface, &adapter, &renderer)?;
        // Restore the last completed frame into the new device after suspension.
        let frame = self
            .backend
            .frame
            .as_ref()
            .map(|frame| -> Result<Arc<GpuFrame>> {
                Ok(Arc::new(renderer.preview_image(&frame.read()?)?))
            })
            .transpose()?;
        self.graphics = Some(Graphics {
            presenter,
            frame,
            renderer,
            adapter,
            instance,
            window,
        });
        self.sync_input();
        Ok(())
    }
    fn render_game(&mut self) -> Result<Arc<GpuFrame>> {
        let graphics = self.graphics.as_mut().ok_or("no graphics device")?;
        if let Some(image) = self.startup.movie_frame() {
            return Ok(Arc::new(graphics.renderer.render_image(
                image,
                self.startup.screen_size(),
                self.backend.display.render_mode == 1,
            )?));
        }
        let objects = self
            .backend
            .font
            .borrow_mut()
            .prepare_objects(self.startup.messages())?;
        let mut additional: Vec<_> = objects.iter().collect();
        additional.extend(self.startup.messages().visible_window_objects());
        let frame = cmvs_core::scene_render::prepare_scene(
            self.startup.scene(),
            Some(self.startup.camera()),
            &additional,
            self.startup.screen_size()[0],
            self.startup.screen_size()[1],
        )?;
        Ok(Arc::new(graphics.renderer.render(&frame)?))
    }
    fn redraw(&mut self) -> Result<()> {
        if self.graphics.is_none() {
            return Ok(());
        }
        // A VM budget slice is not a completed engine frame. Resizes and redraw
        // requests continue to present the previous complete canvas.
        if self.draw_ready {
            let frame = self.render_game()?;
            self.graphics.as_mut().expect("graphics checked").frame = Some(frame);
            self.draw_ready = false;
        }
        let graphics = self.graphics.as_mut().expect("graphics checked");
        let Some(frame) = &graphics.frame else {
            return Ok(());
        };
        if graphics.presenter.lost {
            graphics.presenter = Presenter::new(
                graphics.instance.create_surface(graphics.window.clone())?,
                &graphics.adapter,
                &graphics.renderer,
            )?;
        }
        if graphics.presenter.present(
            &graphics.renderer,
            frame,
            graphics.window.dimensions(),
            self.backend.mode(),
            self.backend.display.render_mode == 1,
            false,
        )? {
            self.backend.frame = Some(frame.clone());
        }
        Ok(())
    }
    fn execute(&mut self) -> Result<()> {
        if self.input_due {
            let size = self
                .graphics
                .as_ref()
                .ok_or("no graphics device")?
                .window
                .dimensions();
            if let Some(viewport) =
                Viewport::for_canvas(self.startup.screen_size(), size, self.backend.mode())
            {
                self.touch
                    .frame(&self.bindings, viewport, self.startup.input_mut());
                self.input_due = false;
            } else {
                self.next_frame = Instant::now() + FRAME_TIME;
                return Ok(());
            }
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(8) {
            match self.startup.tick(&mut self.backend)? {
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::Quit => {
                    self.exit = true;
                    return Ok(());
                }
                StartupEvent::Yield => {
                    self.input_due = true;
                    if self.backend.font_pending {
                        // Keep the last canvas while the asynchronous modal dialog
                        // runs, and continue polling Android lifecycle events.
                        self.last_frame = Instant::now();
                        self.next_frame = self.last_frame + FRAME_TIME;
                        return Ok(());
                    }
                    let now = Instant::now();
                    let delta = now
                        .duration_since(self.last_frame)
                        .as_millis()
                        .min(u128::from(u32::MAX)) as u32;
                    self.last_frame = now;
                    self.startup.advance_decoded_frame(delta)?;
                    self.draw_ready = true;
                    self.redraw()?;
                    self.next_frame = Instant::now() + FRAME_TIME;
                    return Ok(());
                }
                StartupEvent::Inactive => {
                    self.last_frame = Instant::now();
                    self.next_frame = self.last_frame + FRAME_TIME;
                    return Ok(());
                }
                event => return Err(format!("game stopped: {event:?}").into()),
            }
        }
        self.next_frame = Instant::now();
        Ok(())
    }
    fn persist(&mut self) -> Result<()> {
        self.startup.save_preferences(self.backend.display)?;
        Ok(())
    }
    fn main_event(&mut self, app: &AndroidApp, event: MainEvent<'_>) -> Result<()> {
        match event {
            MainEvent::InitWindow { .. } => self.initialize(app)?,
            MainEvent::TerminateWindow { .. } => self.release_surface(),
            MainEvent::Resume { .. } => {
                self.resumed = true;
                self.sync_input();
            }
            MainEvent::Pause => {
                self.resumed = false;
                self.sync_input();
                self.persist()?;
            }
            MainEvent::Stop => {
                self.resumed = false;
                self.sync_input();
            }
            MainEvent::GainedFocus => {
                self.focused = true;
                self.sync_input();
            }
            MainEvent::LostFocus => {
                self.focused = false;
                self.sync_input();
            }
            MainEvent::WindowResized { .. } | MainEvent::ContentRectChanged { .. } => {
                self.touch.cancel(self.startup.input_mut());
            }
            MainEvent::InputAvailable => {
                let mut events = app.input_events_iter()?;
                while events.next(|event| self.input_event(event)) {}
            }
            MainEvent::RedrawNeeded { .. } => self.redraw()?,
            MainEvent::Destroy => {
                self.persist()?;
                self.release_surface();
                self.exit = true;
            }
            _ => {}
        }
        Ok(())
    }
    fn input_event(&mut self, event: &InputEvent<'_>) -> InputStatus {
        let InputEvent::MotionEvent(motion) = event else {
            return InputStatus::Unhandled;
        };
        let Some(graphics) = &self.graphics else {
            return InputStatus::Unhandled;
        };
        let Some(viewport) = Viewport::for_canvas(
            self.startup.screen_size(),
            graphics.window.dimensions(),
            self.backend.mode(),
        ) else {
            return InputStatus::Unhandled;
        };
        let phase = match motion.action() {
            MotionAction::Down | MotionAction::PointerDown => TouchPhase::Started,
            MotionAction::Up | MotionAction::PointerUp => TouchPhase::Ended,
            MotionAction::Move => TouchPhase::Moved,
            MotionAction::Cancel => {
                self.touch.cancel(self.startup.input_mut());
                return InputStatus::Handled;
            }
            _ => return InputStatus::Unhandled,
        };
        for pointer in motion.pointers() {
            if phase != TouchPhase::Moved
                && pointer.pointer_id()
                    != motion.pointer_at_index(motion.pointer_index()).pointer_id()
            {
                continue;
            }
            self.touch.event(
                TouchEvent {
                    id: pointer.pointer_id() as u64,
                    phase,
                    position: [f64::from(pointer.x()), f64::from(pointer.y())],
                },
                viewport,
                self.startup.input_mut(),
            );
        }
        InputStatus::Handled
    }
}

pub(super) fn run(app: AndroidApp) -> Result<()> {
    let directory = app
        .internal_data_path()
        .ok_or("application directory unavailable")?
        .canonicalize()?;
    let mut text = String::new();
    File::open(directory.join("launch.cfg"))?
        .take(4097)
        .read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err("launch configuration too large".into());
    }
    let mut config = text.lines();
    let root = resolve_under(
        &directory,
        config.next().ok_or("no game selected")?.as_bytes(),
    )?;
    let font_path = resolve_under(
        &directory,
        config.next().ok_or("no font selected")?.as_bytes(),
    )?;
    let face = config.next().ok_or("no font collection index")?.parse()?;
    let font = Rc::new(RefCell::new(FontRenderer::read(&font_path, face)?));
    let game = Registry::builtin().identify_game(&root, None)?;
    let record = game
        .selection
        .record
        .ok_or("game does not have a registered identity")?;
    log::info!(
        "Loading game {} with profile {}",
        record.game,
        game.selection.profile.id
    );
    let user = directory.join("users").join(&record.game);
    let script = load_start(&root, &game.files)?;
    let mut startup = Startup::new(&script, &root, &game, 0)?;
    // Match the desktop host's existing command policy; identity and unsupported
    // commands still pass through the core's normal validation gates.
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    startup.set_user_data_directory(&user)?;
    let audio = NativeAudio::new(&root)?;
    let audio_control = audio.host_control();
    audio_control.set_active(false);
    startup.set_audio_backend(Box::new(audio))?;
    log::info!("Native Vorbis audio enabled");
    startup.set_decode_movie_frames(true);
    startup.set_record_text(true);
    startup.set_glyph_metrics(font.borrow().metrics());
    font.borrow().set_user_directory(&user)?;
    if !font.borrow().has_face(startup.settings().font_face()) {
        startup.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)?;
    }
    startup.set_save_clock(Box::new(utc_save_time));
    let mut keys = String::new();
    File::open(root.join("key.cfg"))?
        .take(65537)
        .read_to_string(&mut keys)?;
    let bindings = KeyBindings::parse(&keys)?;
    let mut backend = MobileBackend::new(
        app.clone(),
        directory,
        font,
        startup.display_preferences(),
        startup.screen_size(),
    );
    backend.display.render_mode =
        cmvs_core::platform::supported_render_mode(&backend, backend.display.render_mode)?;
    let mut player = Player {
        audio_control,
        graphics: None,
        startup,
        backend,
        touch: TouchInput::default(),
        bindings,
        resumed: false,
        focused: false,
        next_frame: Instant::now(),
        last_frame: Instant::now(),
        input_due: true,
        draw_ready: false,
        exit: false,
    };
    while !player.exit {
        let timeout = player
            .active()
            .then(|| player.next_frame.saturating_duration_since(Instant::now()));
        let mut outcome = Ok(());
        app.poll_events(timeout, |event| {
            if outcome.is_ok()
                && let PollEvent::Main(event) = event
            {
                outcome = player.main_event(&app, event);
            }
        });
        if let Err(error) = outcome {
            player.release_surface();
            return Err(error);
        }
        if player.active()
            && !player.exit
            && Instant::now() >= player.next_frame
            && let Err(error) = player.execute()
        {
            let vm = player.startup.machine();
            log::error!(
                "Runtime at {} / 0x{:x} after {} steps: {error}",
                vm.current_name().escape_ascii(),
                vm.pc(),
                player.startup.steps()
            );
            player.release_surface();
            return Err(error);
        }
    }
    player.persist()?;
    Ok(())
}

// Save labels use UTC consistently with the desktop host.
fn utc_save_time() -> cmvs_core::runner::SaveTime {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = ((seconds / 86400) as i64, seconds % 86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    [
        (yoe + era * 400 + i64::from(month <= 2)) as u16,
        month as u16,
        (doy - (153 * mp + 2) / 5 + 1) as u16,
        (rest / 3600) as u16,
        (rest / 60 % 60) as u16,
        (rest % 60) as u16,
    ]
}

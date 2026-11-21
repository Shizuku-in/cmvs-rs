#![forbid(unsafe_code)]
use crate::backend::MobileBackend;
use android_activity::{
    AndroidApp, InputStatus, MainEvent, PollEvent,
    input::{InputEvent, MotionAction},
};
use cmvs_audio::{HostAudioControl, NativeAudio};
use cmvs_core::{
    host::filesystem::resolve_under, host::input::KeyBindings, render::raster::ScaleFilter,
    runtime::FrameBudget, runtime::FrameDriver, runtime::FrameOutcome, runtime::Runtime,
    runtime::RuntimeEvent, runtime::SaveStorage,
};
use cmvs_font::font::FontRenderer;
use cmvs_host::{
    AndroidHostFs, EncodingPolicy, GameBootstrap, GpuTarget, KeyBindingPolicy, SessionConfig,
    compose_cpu_frame, with_gpu_target,
};
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

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
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
struct Player {
    audio_control: HostAudioControl,
    graphics: Option<Graphics>,
    startup: Runtime,
    backend: MobileBackend,
    scale_filter: Option<ScaleFilter>,
    touch: TouchInput,
    bindings: KeyBindings,
    resumed: bool,
    focused: bool,
    next_frame: Instant,
    last_frame: Instant,
    driver: FrameDriver,
    draw_ready: bool,
    exit: bool,
}
impl Player {
    fn active(&self) -> bool {
        self.resumed && self.focused && self.graphics.is_some()
    }
    fn sync_input(&mut self) {
        let active = self.active();
        self.driver.suspend();
        self.audio_control.set_active(active);
        self.touch.focus(active, self.startup.input_mut());
        self.startup.set_focused(active);
        self.next_frame = Instant::now();
        self.last_frame = self.next_frame;
    }
    fn release_surface(&mut self) {
        self.driver.suspend();
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
    fn render_software(&self) -> Result<cmvs_core::render::raster::RgbaImage> {
        let mut font = self.backend.font.borrow_mut();
        compose_cpu_frame(
            &self.startup,
            Some(&mut *font),
            self.backend.display.render_mode == 1,
        )
    }
    fn render_game(&mut self) -> Result<Arc<GpuFrame>> {
        let graphics = self.graphics.as_mut().ok_or("no graphics device")?;
        let mut font = self.backend.font.borrow_mut();
        let frame = with_gpu_target(
            &self.startup,
            Some(&mut *font),
            self.backend.display.render_mode == 1,
            |target| match target {
                GpuTarget::Movie {
                    image,
                    size,
                    render_mode,
                } => graphics.renderer.render_image(image, size, render_mode),
                GpuTarget::Scene(frame) => graphics.renderer.render(&frame),
            },
        )?;
        Ok(Arc::new(frame))
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
            self.scale_filter
                .unwrap_or_else(|| (self.backend.display.render_mode == 1).into()),
            false,
        )? {
            self.backend.frame = Some(frame.clone());
        }
        Ok(())
    }
    fn execute(&mut self) -> Result<()> {
        if self.driver.needs_input() {
            let size = self
                .graphics
                .as_ref()
                .ok_or("no graphics device")?
                .window
                .dimensions();
            if let Some(viewport) =
                Viewport::for_canvas(self.startup.screen_size(), size, self.backend.mode())
            {
                self.driver.sample_input(self.startup.input_mut(), |input| {
                    self.touch.frame(&self.bindings, viewport, input);
                });
            } else {
                self.next_frame = Instant::now() + FRAME_TIME;
                return Ok(());
            }
        }
        match self.driver.run_slice(
            &mut self.startup,
            &mut FrameBudget::time(Duration::from_millis(8)),
            |runtime| runtime.tick(&mut self.backend),
        )? {
            FrameOutcome::Stopped(RuntimeEvent::Quit) => {
                self.exit = true;
                return Ok(());
            }
            FrameOutcome::Yield => {
                if self.backend.font_pending {
                    self.driver.skip_frame();
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
                self.driver.finish_frame(&mut self.startup, delta)?;
                self.draw_ready = true;
                self.redraw()?;
                if self.startup.capture_due() {
                    let captured = match self.graphics.as_ref().and_then(|g| g.frame.as_ref()) {
                        Some(frame) => frame.read()?,
                        None => self.render_software()?,
                    };
                    self.startup.supply_capture(captured)?;
                }
                self.next_frame = Instant::now() + FRAME_TIME;
                return Ok(());
            }
            FrameOutcome::Inactive => {
                self.last_frame = Instant::now();
                self.next_frame = self.last_frame + FRAME_TIME;
                return Ok(());
            }
            FrameOutcome::Stopped(event) => return Err(format!("game stopped: {event:?}").into()),
            FrameOutcome::BudgetExhausted => {}
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
    let scale_filter = config.next().filter(|s| !s.is_empty());
    let fs = Arc::new(AndroidHostFs);
    let bootstrap = GameBootstrap::load(fs, &root, None, EncodingPolicy::AutoDefaultShiftJis)?;
    let record = bootstrap
        .game
        .selection
        .record
        .ok_or("game does not have a registered identity")?;
    log::info!(
        "Loading game {} with profile {}",
        record.game,
        bootstrap.game.selection.profile.id
    );
    let encoding = bootstrap.encoding;
    let user = directory.join("users").join(&record.game);
    let font = Rc::new(RefCell::new(
        FontRenderer::read(&font_path, face)?.with_text_encoding(encoding),
    ));
    let (mut startup, bindings) = bootstrap.open_runtime(
        Some(&*font.borrow()),
        SessionConfig {
            saves: SaveStorage::UserDirectory(user.clone()),
            user_font_dir: Some(user),
            decoded_image_cache: Some(128 << 20),
            key_bindings: KeyBindingPolicy::Required,
            ..SessionConfig::default()
        },
    )?;
    let scale_filter = startup.scale_filter_override(scale_filter)?;
    let audio = NativeAudio::new(&root)?;
    let audio_control = audio.host_control();
    audio_control.set_active(false);
    startup.set_audio_backend(Box::new(audio))?;
    log::info!("Native Vorbis audio enabled");
    let mut backend = MobileBackend::new(
        app.clone(),
        directory,
        font,
        encoding,
        startup.display_preferences(),
        startup.screen_size(),
    );
    backend.display.render_mode =
        cmvs_core::host::platform::supported_render_mode(&backend, backend.display.render_mode)?;
    let mut player = Player {
        audio_control,
        graphics: None,
        startup,
        backend,
        scale_filter,
        touch: TouchInput::default(),
        bindings,
        resumed: false,
        focused: false,
        next_frame: Instant::now(),
        last_frame: Instant::now(),
        driver: FrameDriver::default(),
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

mod audio;
mod boost;
mod gpu;
#[cfg(target_os = "horizon")]
mod sys {
    pub use cmvs_native::{
        SwitchAudioOut, SwitchGpu, SwitchWindow, prefer_core, set_cpu_boost, shared_font,
        show_system_error, software_keyboard, total_memory,
    };
}
#[cfg(not(target_os = "horizon"))]
#[path = "stub.rs"]
mod sys;

use cmvs_console::{
    ButtonMap, Console, Labels,
    backend::{ConsoleBackend, ConsoleFrame},
    controls::{self, Controls},
    encoding::{self, SystemFont, UNCERTAIN_ENCODING_MESSAGE},
    error_screen, format_tick_error, library, menu, millis,
};
use cmvs_core::{
    render::raster::RgbaImage, render::viewport::Viewport, render::viewport::presentation_mode,
    runtime::FrameBudget, runtime::FrameOutcome, runtime::RuntimeEvent, runtime::SaveStorage,
};
use cmvs_host::{
    EncodingPolicy, GameBootstrap, GpuTarget, HostSession, KeyBindingPolicy, MovieConfig,
    SessionConfig,
};
use cmvs_native::switch_button as button;
use std::{
    error::Error,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const BASE: &str = "sdmc:/switch/cmvs-rs";
const LOG_PATH: &str = "sdmc:/switch/cmvs-rs/error.log";
/// `LOG_PATH` as the player sees it on the SD card.
const LOG: &str = "switch/cmvs-rs/error.log";
pub(crate) const SCREEN: [u32; 2] = [1280, 720];
/// VM budget per loop iteration before polling input and the applet again.
const BUDGET: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(16);

/// The combined standard controller. In game, the D-pad uses keyboard navigation and
/// the left stick moves the pointer; A clicks at the pointer and Y is Enter for the
/// navigation selection.
fn console() -> Console {
    Console {
        screen: SCREEN,
        buttons: ButtonMap {
            confirm: button::A,
            exit: button::PLUS,
            up: button::UP,
            down: button::DOWN,
            page_up: button::L,
            page_down: button::R,
            codes: vec![
                (button::A, controls::MOUSE_LEFT),
                (button::B, controls::MOUSE_RIGHT),
                (button::X, controls::ESCAPE),
                (button::Y, controls::ENTER),
                (button::ZL, controls::SHIFT),
                (button::ZR, controls::CTRL),
                (button::UP, controls::ARROW_UP),
                (button::DOWN, controls::ARROW_DOWN),
                (button::LEFT, controls::ARROW_LEFT),
                (button::RIGHT, controls::ARROW_RIGHT),
            ],
            wheel_up: button::L,
            wheel_down: button::R,
        },
        labels: Labels {
            confirm: "A",
            exit: "+",
            pages: "L / R",
        },
    }
}

/// Decoded images kept for reloads; skip mode and scene changes load the same sprites
/// repeatedly. An application has over 3 GiB, where high-resolution games (about
/// 15 MiB a sprite) keep reloading with less than this; an applet has a few hundred
/// MiB in all and keeps the size of the GPU texture cache.
fn decoded_cache_bytes(total_memory: u64) -> usize {
    if total_memory >= 2 << 30 {
        512 << 20
    } else {
        128 << 20
    }
}

/// How a game session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The game quit, or the player held + to leave it.
    Quit,
    /// The system asked the application to close.
    Exit,
}

fn render_gpu(session: &mut HostSession, gpu: &mut gpu::GpuRenderer, linear: bool) -> Result<()> {
    session.with_gpu_target(linear, |target| match target {
        GpuTarget::Movie { image, size, .. } => gpu.render_image(image, size, linear),
        GpuTarget::Scene(frame) => gpu.render(&frame),
    })
}

/// Host-only crosshair; the engine screenshot readback never includes it.
fn draw_cursor(surface: &mut [u32], view: Viewport, pointer: [i32; 2]) {
    let at = view.to_target(pointer);
    for d in -5..=5 {
        for (x, y) in [
            (at[0] as i32 + d, at[1] as i32),
            (at[0] as i32, at[1] as i32 + d),
        ] {
            if (0..SCREEN[0] as i32).contains(&x) && (0..SCREEN[1] as i32).contains(&y) {
                surface[y as usize * SCREEN[0] as usize + x as usize] = 0xffffff;
            }
        }
    }
}

fn run(
    window: &mut sys::SwitchWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    root: &Path,
) -> Result<Ended> {
    // Boosted through game loading; normal clocks return when this drops.
    let mut load = boost::LoadBoost::starting();
    let mut cpu_boost = boost::CpuBoost::default();
    cpu_boost.apply(load.wanted(true));
    let base = Path::new(BASE);
    let bootstrap = GameBootstrap::load(
        std::sync::Arc::new(cmvs_console::ConsoleFs),
        root,
        None,
        EncodingPolicy::AutoStrict(UNCERTAIN_ENCODING_MESSAGE),
    )?;
    let record = bootstrap.game.selection.record.ok_or("unregistered game")?;
    let encoding = bootstrap.encoding;
    let user = base.join("users").join(&record.game);
    let font = encoding::load_game_font(
        base,
        encoding,
        "no font.ttf and no system shared font",
        |font| sys::shared_font(font == SystemFont::SimplifiedChinese),
    )?;
    let mut session = bootstrap.into_session(
        font,
        SessionConfig {
            saves: SaveStorage::UserDirectory(user.clone()),
            user_font_dir: Some(user),
            // Core 0 runs the VM and rendering, core 1 the audio thread, core 2 prefetch/decode.
            movie: MovieConfig::Prefetch {
                budget_bytes: 192 << 20,
                max_frames: 144,
                on_thread_start: || {
                    sys::prefer_core(2);
                },
            },
            image_decode_threads: Some(cmvs_core::host::image::DecodeThreads {
                extra: 1,
                on_start: Some(|_| {
                    sys::prefer_core(2);
                }),
            }),
            decoded_image_cache: Some(decoded_cache_bytes(sys::total_memory())),
            key_bindings: KeyBindingPolicy::Required,
            ..SessionConfig::default()
        },
    )?;
    let scale_filter = session.runtime.scale_filter_override(None)?;
    let (audio, audio_thread) = audio::start(root)?;
    let audio_control = audio.host_control();
    session.runtime.set_audio_backend(Box::new(audio))?;
    let size = session.runtime.screen_size();
    let mut backend = ConsoleBackend {
        display: session.runtime.display_preferences(),
        size,
        frame: None,
        gpu,
        modal_shown: false,
        encoding,
        platform_name: "Switch",
        text_dialog: |t, p, i, l, c| sys::software_keyboard(t, p, i, l, c),
    };
    backend.display.render_mode =
        cmvs_core::host::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut surface = match backend.gpu {
        Some(_) => Vec::new(),
        None => vec![0u32; (SCREEN[0] * SCREEN[1]) as usize],
    };
    let mut controls = Controls::new(size, console().buttons);
    // The button that started the game must not click it.
    controls.suppress_held(session.runtime.input_mut());
    let mut focused = true;
    let mut last_poll = Instant::now();
    let mut last_frame = last_poll;
    let ended = loop {
        let sample = window.poll();
        if sample.exit {
            break Ended::Exit;
        }
        audio_thread.check()?;
        let now = Instant::now();
        let dt = millis(now.duration_since(last_poll));
        last_poll = now;
        if sample.focused != focused {
            focused = sample.focused;
            audio_control.set_active(focused);
            session.runtime.set_focused(focused);
            controls.focus(focused, session.runtime.input_mut());
            session.driver.suspend();
            last_frame = now;
        }
        cpu_boost.apply(load.wanted(focused));
        let mode = presentation_mode(
            backend.display.fullscreen,
            backend.display.aspect_scaling,
            backend.display.aspect_mode,
        );
        let view = Viewport::new(size, SCREEN, mode)?;
        controls.poll(&sample, view, dt);
        if controls.quit_requested() {
            break Ended::Quit;
        }
        let bindings = &session.bindings;
        session
            .driver
            .sample_input(session.runtime.input_mut(), |input| {
                if let Some(warp) = input.take_warp() {
                    controls.warp(warp);
                }
                controls.frame(bindings, input);
            });
        let outcome = session.driver.run_slice(
            &mut session.runtime,
            &mut FrameBudget::time(BUDGET),
            |startup| {
                let tick = Instant::now();
                let event = startup
                    .tick(&mut backend)
                    .map_err(|error| format_tick_error(startup, error))?;
                if std::mem::take(&mut backend.modal_shown) {
                    // Time inside the keyboard is not game time, and the button that
                    // closed it must not click the game.
                    controls.suppress_held(startup.input_mut());
                    last_frame = Instant::now();
                    last_poll = last_frame;
                    load.interrupted();
                } else {
                    load.ran(tick.elapsed());
                }
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(event)
            },
        )?;
        match outcome {
            FrameOutcome::Stopped(RuntimeEvent::Quit) => {
                session.runtime.save_preferences(backend.display)?;
                return Ok(Ended::Quit);
            }
            FrameOutcome::Inactive => {
                load.interrupted();
                std::thread::sleep(IDLE);
                last_frame = Instant::now();
            }
            FrameOutcome::Yield => {
                let now = Instant::now();
                load.frame(now);
                session
                    .driver
                    .finish_frame(&mut session.runtime, millis(now.duration_since(last_frame)))?;
                last_frame = now;
                let linear = backend.display.render_mode == 1;
                let filter = scale_filter
                    .filter(|_| [view.width, view.height] != size)
                    .unwrap_or_else(|| linear.into());
                let cursor = controls
                    .cursor_visible()
                    .then(|| view.to_target(controls.pointer()));
                if let (Some(gpu), Some(sampler_linear)) =
                    (&mut backend.gpu, filter.linear_sampler())
                {
                    render_gpu(&mut session, gpu, linear)?;
                    gpu.present(mode, sampler_linear, cursor)?;
                    if session.runtime.capture_due() {
                        session.runtime.supply_capture(gpu.read()?)?;
                    }
                    backend.frame = None;
                } else {
                    let frame = session.compose_cpu_frame(linear, false)?;
                    if let Some(gpu) = &mut backend.gpu {
                        // The deko3d swapchain owns the window, so upload the CPU-scaled canvas.
                        let display = frame.presentation_canvas(SCREEN, mode, filter)?;
                        gpu.render_image(&display, SCREEN, false)?;
                        gpu.present(1, false, cursor)?;
                        gpu.forget_presented();
                    } else {
                        surface.fill(0);
                        frame.draw_filtered(&mut surface, SCREEN[0], SCREEN[1], mode, filter)?;
                        if cursor.is_some() {
                            draw_cursor(&mut surface, view, controls.pointer());
                        }
                        window.present(&surface)?;
                    }
                    if session.runtime.capture_due() {
                        session.runtime.supply_capture(frame.clone())?;
                    }
                    backend.frame = Some(ConsoleFrame::Canvas(frame));
                }
            }
            FrameOutcome::Stopped(event) => return Err(format!("game stopped: {event:?}").into()),
            FrameOutcome::BudgetExhausted => {}
        }
    };
    session.runtime.save_preferences(backend.display)?;
    Ok(ended)
}

/// Shows a full-screen host image through whichever renderer is active.
fn show(
    window: &mut sys::SwitchWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    image: &RgbaImage,
) -> Result<()> {
    if let Some(gpu) = gpu {
        gpu.render_image(image, SCREEN, false)?;
        return gpu.present(1, false, None);
    }
    let mut surface = vec![0u32; (SCREEN[0] * SCREEN[1]) as usize];
    image.draw_scaled(&mut surface, SCREEN[0], SCREEN[1], 1, false)?;
    Ok(window.present(&surface)?)
}

/// Runs the selection screen; `None` when the player or the system closes it.
fn choose(
    window: &mut sys::SwitchWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    games: &[library::GameEntry],
    font: &ab_glyph::FontVec,
    selected: usize,
) -> Result<Option<usize>> {
    let first = window.poll();
    if first.exit {
        return Ok(None);
    }
    let mut menu = menu::Menu::new(selected, games.len(), &first, &console());
    let mut dirty = true;
    let mut last = Instant::now();
    loop {
        let sample = window.poll();
        if sample.exit {
            return Ok(None);
        }
        let now = Instant::now();
        let dt = millis(now.duration_since(last));
        last = now;
        match menu.poll(&sample, dt) {
            menu::MenuAction::Idle => {}
            menu::MenuAction::Moved => dirty = true,
            menu::MenuAction::Exit => return Ok(None),
            menu::MenuAction::Launch(index) => {
                // Identification and loading take a while; say what is happening.
                show(
                    window,
                    gpu.as_deref_mut(),
                    &menu::draw(&menu, games, font, true),
                )?;
                return Ok(Some(index));
            }
        }
        if dirty && sample.focused {
            show(
                window,
                gpu.as_deref_mut(),
                &menu::draw(&menu, games, font, false),
            )?;
            dirty = false;
        }
        std::thread::sleep(IDLE);
    }
}

/// Shows `message` until the player leaves the page. When the page itself cannot be
/// shown (for example after a GPU failure), the system error dialog stands in.
fn show_error(
    window: &mut sys::SwitchWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    font: &ab_glyph::FontVec,
    game: Option<&str>,
    message: &str,
    can_return: bool,
) -> error_screen::ErrorChoice {
    // The page titles the game; the log and the system dialog name it inline.
    let logged = match game {
        Some(game) => format!("{game}: {message}"),
        None => message.to_owned(),
    };
    report(&logged);
    let page = |window: &mut sys::SwitchWindow| -> Result<error_screen::ErrorChoice> {
        let first = window.poll();
        if first.exit {
            return Ok(error_screen::ErrorChoice::Exit);
        }
        let console = console();
        let image = error_screen::draw(&console, font, game, message, LOG, can_return);
        show(window, gpu, &image)?;
        let mut prompt = error_screen::ErrorPrompt::new(&first, can_return, &console.buttons);
        loop {
            let sample = window.poll();
            if sample.exit {
                return Ok(error_screen::ErrorChoice::Exit);
            }
            if let Some(choice) = prompt.poll(&sample) {
                return Ok(choice);
            }
            std::thread::sleep(IDLE);
        }
    };
    page(window).unwrap_or_else(|page_error| {
        system_error(&format!("{logged}\n\nThe error page failed: {page_error}"));
        error_screen::ErrorChoice::Exit
    })
}

/// Logs and shows an error through the system dialog, for failures without a page.
fn system_error(message: &str) {
    report(message);
    let summary: String = message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    sys::show_system_error(&summary, &format!("{message}\n\nSaved to {LOG}"));
}

/// One game starts directly; several are offered in a menu that games return to.
/// Errors are shown here; with a game list, the player may go back to it.
fn session(
    window: &mut sys::SwitchWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    font: &ab_glyph::FontVec,
) {
    let base = Path::new(BASE);
    let games = match library::discover(base) {
        Ok(games) if !games.is_empty() => games,
        Ok(_) => {
            let message = format!("no games found: put each game directory in {BASE}/games/");
            show_error(window, gpu, font, None, &message, false);
            return;
        }
        Err(error) => {
            let message = format!("cannot list games: {error}");
            show_error(window, gpu, font, None, &message, false);
            return;
        }
    };
    let single = games.len() == 1;
    let mut selected = library::last_started(base, &games);
    loop {
        if !single {
            match choose(window, gpu.as_deref_mut(), &games, font, selected) {
                Ok(Some(index)) => selected = index,
                Ok(None) => return,
                Err(error) => {
                    show_error(window, gpu, font, None, &error.to_string(), false);
                    return;
                }
            }
            library::remember(base, &games[selected]);
        }
        let game = &games[selected];
        let result = run(window, gpu.as_deref_mut(), &game.root);
        if let Some(gpu) = gpu.as_deref_mut() {
            gpu.clear_cache();
        }
        match result {
            Ok(Ended::Exit) => return,
            Ok(Ended::Quit) if single => return,
            Ok(Ended::Quit) => {}
            Err(error) => {
                let name = Some(game.name.as_str());
                let choice = show_error(
                    window,
                    gpu.as_deref_mut(),
                    font,
                    name,
                    &error.to_string(),
                    !single,
                );
                if choice == error_screen::ErrorChoice::Exit {
                    return;
                }
            }
        }
    }
}

fn main() {
    // A panic aborts the process; leave its message where errors are expected.
    std::panic::set_hook(Box::new(|info| report(&format!("panic: {info}"))));
    // The window lives until main returns, after the error log is written: dropping
    // it unlocks exit, after which a pending HOME close may end the process.
    let gpu = match cmvs_console::gpu::use_gpu(Path::new(BASE)) {
        Ok(gpu) => gpu,
        Err(error) => return system_error(&error.to_string()),
    };
    let opened = if gpu {
        sys::SwitchWindow::open_gpu().map(|(window, gpu)| (window, Some(gpu)))
    } else {
        sys::SwitchWindow::open()
            .map(|window| (window, None))
            .map_err(String::from)
    };
    let (mut window, gpu) = match opened {
        Ok(opened) => opened,
        Err(error) => return system_error(&error),
    };
    let mut gpu = match gpu.map(gpu::GpuRenderer::new).transpose() {
        Ok(gpu) => gpu,
        Err(error) => return system_error(&error.to_string()),
    };
    let font = match encoding::load_host_font(Path::new(BASE), || {
        Ok(sys::shared_font(false).ok_or("no font.ttf and no system shared font")?)
    }) {
        Ok(font) => font,
        Err(error) => return system_error(&format!("cannot load the font: {error}")),
    };
    session(&mut window, gpu.as_mut(), &font);
}

fn report(error: &str) {
    eprintln!("Switch player: {error}");
    let _ = std::fs::write(LOG_PATH, error);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_application_memory_budget_gets_the_large_decoded_cache() {
        assert_eq!(decoded_cache_bytes(0), 128 << 20);
        assert_eq!(decoded_cache_bytes(442 << 20), 128 << 20);
        assert_eq!(decoded_cache_bytes(3276 << 20), 512 << 20);
    }

    #[test]
    fn a_clicks_zr_is_ctrl_and_the_shoulders_scroll() {
        let console = console();
        let codes = &console.buttons.codes;
        assert!(codes.contains(&(button::A, controls::MOUSE_LEFT)));
        assert!(codes.contains(&(button::ZR, controls::CTRL)));
        assert_eq!(
            (console.buttons.wheel_up, console.buttons.wheel_down),
            (button::L, button::R)
        );
    }
}

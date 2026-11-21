mod audio;
mod gpu;
#[cfg(target_os = "vita")]
mod sys {
    pub use cmvs_native::{
        VitaAudioOut, VitaGpu, VitaWindow, confirm_is_cross, ime_dialog, prefer_core,
        set_max_clocks, show_message,
    };
}
#[cfg(not(target_os = "vita"))]
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
use cmvs_native::vita_button as button;
use std::{
    error::Error,
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const BASE: &str = "ux0:data/cmvs-rs";
const LOG_PATH: &str = "ux0:data/cmvs-rs/error.log";
/// The system's Japanese gothic face (OpenType CFF), used without `font.ttf`.
const SYSTEM_FONT: &str = "sa0:data/font/pvf/jpn0.pvf";
/// The system's simplified Chinese gothic face (TrueType), for GBK and UTF-8 games.
const CHINESE_FONT: &str = "sa0:data/font/pvf/cn0.pvf";
pub(crate) const SCREEN: [u32; 2] = [960, 544];
const SCREEN_RECT: [f32; 4] = [0.0, 0.0, SCREEN[0] as f32, SCREEN[1] as f32];
/// VM budget per loop iteration before polling input and the system again.
const BUDGET: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(16);
/// The VM, glyph rasterization and the software compositor share this core; audio
/// runs on core 1.
const MAIN_CORE: u32 = 0;

/// The handheld's buttons. Confirm and cancel follow the system's enter button
/// (circle in Japan and Asia, cross elsewhere). In game, the D-pad uses keyboard
/// navigation and the left stick moves the pointer; confirm clicks at the pointer and
/// square is Enter for the navigation selection. R holds Ctrl (skip), L and the right
/// stick scroll the wheel.
fn console(confirm_is_cross: bool) -> Console {
    let (confirm, cancel, confirm_label) = if confirm_is_cross {
        (button::CROSS, button::CIRCLE, "×")
    } else {
        (button::CIRCLE, button::CROSS, "○")
    };
    Console {
        screen: SCREEN,
        buttons: ButtonMap {
            confirm,
            exit: button::START,
            up: button::UP,
            down: button::DOWN,
            page_up: button::L,
            page_down: button::R,
            codes: vec![
                (confirm, controls::MOUSE_LEFT),
                (cancel, controls::MOUSE_RIGHT),
                (button::TRIANGLE, controls::ESCAPE),
                (button::SQUARE, controls::ENTER),
                (button::SELECT, controls::SHIFT),
                (button::R, controls::CTRL),
                (button::UP, controls::ARROW_UP),
                (button::DOWN, controls::ARROW_DOWN),
                (button::LEFT, controls::ARROW_LEFT),
                (button::RIGHT, controls::ARROW_RIGHT),
            ],
            wheel_up: button::L,
            wheel_down: 0,
        },
        labels: Labels {
            confirm: confirm_label,
            exit: "START",
            pages: "L / R",
        },
    }
}

/// Composites the scene on the GPU; the caller presents the canvas.
fn render_gpu(session: &mut HostSession, gpu: &mut gpu::GpuRenderer) -> Result<()> {
    session.with_gpu_target(false, |target| match target {
        GpuTarget::Scene(frame) => gpu.render(frame),
        GpuTarget::Movie { .. } => Err("Vita movie frames use window presentation".into()),
    })
}

/// The screen rectangle a viewport covers, as the window draws it.
fn view_rect(view: Viewport) -> [f32; 4] {
    [
        view.left as f32,
        view.top as f32,
        view.width as f32,
        view.height as f32,
    ]
}

/// Maps the movie's letterboxed rectangle from the logical canvas to the screen.
fn movie_view_rect(view: Viewport, canvas: [u32; 2], movie: [u32; 2]) -> Result<[f32; 4]> {
    let fitted = Viewport::new(movie, canvas, 1)?;
    let [left, top, width, height] = view_rect(view);
    let sx = width / canvas[0] as f32;
    let sy = height / canvas[1] as f32;
    Ok([
        left + fitted.left as f32 * sx,
        top + fitted.top as f32 * sy,
        fitted.width as f32 * sx,
        fitted.height as f32 * sy,
    ])
}

fn run(
    window: &mut sys::VitaWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    console: &Console,
    root: &Path,
) -> Result<()> {
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
        "no font.ttf and no system font in sa0:data/font/pvf",
        |font| {
            std::fs::read(match font {
                SystemFont::Japanese => SYSTEM_FONT,
                SystemFont::SimplifiedChinese => CHINESE_FONT,
            })
            .ok()
        },
    )?;
    let mut session = bootstrap.into_session(
        font,
        SessionConfig {
            saves: SaveStorage::UserDirectory(user.clone()),
            user_font_dir: Some(user),
            // Core 0 runs the VM and rendering, core 1 the audio thread, core 2 prefetch/decode.
            movie: MovieConfig::Prefetch {
                budget_bytes: 16 << 20,
                max_frames: 8,
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
            raw_image_cache: Some(8 << 20),
            decoded_image_cache: Some(48 << 20),
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
        platform_name: "Vita",
        text_dialog: |t, p, i, l, c| sys::ime_dialog(t, p, i, l, c),
    };
    backend.display.render_mode =
        cmvs_core::host::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut controls = Controls::new(size, console.buttons.clone());
    // The button that started the game must not click it.
    controls.suppress_held(session.runtime.input_mut());
    let mut focused = true;
    let mut last_poll = Instant::now();
    let mut last_frame = last_poll;
    loop {
        let sample = window.poll();
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
        let mode = presentation_mode(
            backend.display.fullscreen,
            backend.display.aspect_scaling,
            backend.display.aspect_mode,
        );
        let view = Viewport::new(size, SCREEN, mode)?;
        controls.poll(&sample, view, dt);
        if controls.quit_requested() {
            break;
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
                let event = startup
                    .tick(&mut backend)
                    .map_err(|error| format_tick_error(startup, error))?;
                if std::mem::take(&mut backend.modal_shown) {
                    // Time inside the IME is not game time, and the button that closed
                    // it must not click the game.
                    controls.suppress_held(startup.input_mut());
                    last_frame = Instant::now();
                    last_poll = last_frame;
                }
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(event)
            },
        )?;
        match outcome {
            FrameOutcome::Stopped(RuntimeEvent::Quit) => {
                session.runtime.save_preferences(backend.display)?;
                return Ok(());
            }
            FrameOutcome::Inactive => {
                std::thread::sleep(IDLE);
                last_frame = Instant::now();
            }
            FrameOutcome::Yield => {
                let now = Instant::now();
                session
                    .driver
                    .finish_frame(&mut session.runtime, millis(now.duration_since(last_frame)))?;
                last_frame = now;
                let linear = backend.display.render_mode == 1;
                let scale_override = scale_filter.filter(|_| [view.width, view.height] != size);
                let filter = scale_override.unwrap_or_else(|| linear.into());
                let cursor = controls.cursor_visible().then(|| {
                    let at = view.to_target(controls.pointer());
                    [at[0] as f32, at[1] as f32]
                });
                match (&mut backend.gpu, filter.linear_sampler()) {
                    (Some(gpu), Some(sampler_linear))
                        if session.runtime.movie_frame().is_none() =>
                    {
                        render_gpu(&mut session, gpu)?;
                        gpu.present(view_rect(view), sampler_linear, cursor)?;
                        if session.runtime.capture_due()
                            && let Some(frame) = gpu.read()
                        {
                            session.runtime.supply_capture(frame?)?;
                        }
                        backend.frame = None;
                    }
                    (gpu, None) => {
                        let frame = session.compose_cpu_frame(linear, false)?;
                        let display = frame.presentation_canvas(SCREEN, mode, filter)?;
                        window.present(display.pixels(), SCREEN, SCREEN_RECT, false, cursor)?;
                        if let Some(gpu) = gpu {
                            gpu.forget_presented();
                        }
                        if session.runtime.capture_due() {
                            session.runtime.supply_capture(frame.clone())?;
                        }
                        backend.frame = Some(ConsoleFrame::Canvas(frame));
                    }
                    (gpu, Some(sampler_linear)) => {
                        let movie = session
                            .runtime
                            .movie_frame()
                            .filter(|_| scale_override.is_none());
                        let (mut frame, rect) = if let Some(image) = movie {
                            let rect =
                                movie_view_rect(view, size, [image.width(), image.height()])?;
                            (
                                ConsoleFrame::Movie {
                                    image: image.clone(),
                                    linear,
                                },
                                rect,
                            )
                        } else {
                            let image = session.compose_cpu_frame(linear, false)?;
                            (ConsoleFrame::Canvas(image), view_rect(view))
                        };
                        let image = frame.image();
                        let dimensions = [image.width(), image.height()];
                        if let Err(error) =
                            window.present(image.pixels(), dimensions, rect, sampler_linear, cursor)
                        {
                            if !matches!(frame, ConsoleFrame::Movie { .. }) || dimensions == size {
                                return Err(error.into());
                            }
                            // Large decoded frames may exceed the Vita texture limits.
                            let canvas = frame.readback(size)?;
                            window.present(
                                canvas.pixels(),
                                size,
                                view_rect(view),
                                sampler_linear,
                                cursor,
                            )?;
                            frame = ConsoleFrame::Canvas(canvas);
                        }
                        if let Some(gpu) = gpu {
                            gpu.forget_presented();
                        }
                        if session.runtime.capture_due() {
                            session.runtime.supply_capture(frame.readback(size)?)?;
                        }
                        backend.frame = Some(frame);
                    }
                }
            }
            FrameOutcome::Stopped(event) => return Err(format!("game stopped: {event:?}").into()),
            FrameOutcome::BudgetExhausted => {}
        }
    }
    session.runtime.save_preferences(backend.display)?;
    Ok(())
}

/// Shows a full-screen host page.
fn show(window: &mut sys::VitaWindow, image: &RgbaImage) -> Result<()> {
    Ok(window.present(image.pixels(), SCREEN, SCREEN_RECT, false, None)?)
}

/// Runs the selection screen; `None` when the player closes it.
fn choose(
    window: &mut sys::VitaWindow,
    console: &Console,
    games: &[library::GameEntry],
    font: &ab_glyph::FontVec,
    selected: usize,
) -> Result<Option<usize>> {
    let first = window.poll();
    let mut menu = menu::Menu::new(selected, games.len(), &first, console);
    let mut dirty = true;
    let mut last = Instant::now();
    loop {
        let sample = window.poll();
        let now = Instant::now();
        let dt = millis(now.duration_since(last));
        last = now;
        match menu.poll(&sample, dt) {
            menu::MenuAction::Idle => {}
            menu::MenuAction::Moved => dirty = true,
            menu::MenuAction::Exit => return Ok(None),
            menu::MenuAction::Launch(index) => {
                // Identification and loading take a while; say what is happening.
                show(window, &menu::draw(&menu, games, font, true))?;
                return Ok(Some(index));
            }
        }
        // vita2d presents with vsync, so redrawing paces the loop as well.
        if dirty && sample.focused {
            show(window, &menu::draw(&menu, games, font, false))?;
            dirty = false;
        } else {
            std::thread::sleep(IDLE);
        }
    }
}

/// Shows `message` until the player leaves the page. When the page itself cannot be
/// shown, the system message dialog stands in.
fn show_error(
    window: &mut sys::VitaWindow,
    console: &Console,
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
    let mut page = || -> Result<error_screen::ErrorChoice> {
        let first = window.poll();
        let image = error_screen::draw(console, font, game, message, LOG_PATH, can_return);
        show(window, &image)?;
        let mut prompt = error_screen::ErrorPrompt::new(&first, can_return, &console.buttons);
        loop {
            if let Some(choice) = prompt.poll(&window.poll()) {
                return Ok(choice);
            }
            std::thread::sleep(IDLE);
        }
    };
    page().unwrap_or_else(|page_error| {
        system_error(&format!("{logged}\n\nThe error page failed: {page_error}"));
        error_screen::ErrorChoice::Exit
    })
}

/// Logs and shows an error through the system dialog, for failures without a page.
/// Before the window opens, only the log receives it.
fn system_error(message: &str) {
    report(message);
    sys::show_message(&format!("{message}\n\nSaved to {LOG_PATH}"));
}

/// One game starts directly; several are offered in a menu that games return to.
/// Errors are shown here; with a game list, the player may go back to it.
fn session(
    window: &mut sys::VitaWindow,
    mut gpu: Option<&mut gpu::GpuRenderer>,
    console: &Console,
    font: &ab_glyph::FontVec,
) {
    let base = Path::new(BASE);
    let games = match library::discover(base) {
        Ok(games) if !games.is_empty() => games,
        Ok(_) => {
            let message = format!("no games found: put each game directory in {BASE}/games/");
            show_error(window, console, font, None, &message, false);
            return;
        }
        Err(error) => {
            let message = format!("cannot list games: {error}");
            show_error(window, console, font, None, &message, false);
            return;
        }
    };
    let single = games.len() == 1;
    let mut selected = library::last_started(base, &games);
    loop {
        if !single {
            match choose(window, console, &games, font, selected) {
                Ok(Some(index)) => selected = index,
                Ok(None) => return,
                Err(error) => {
                    show_error(window, console, font, None, &error.to_string(), false);
                    return;
                }
            }
            library::remember(base, &games[selected]);
        }
        let game = &games[selected];
        let result = run(window, gpu.as_deref_mut(), console, &game.root);
        if let Some(gpu) = gpu.as_deref_mut() {
            gpu.clear_cache();
        }
        match result {
            Ok(()) if single => return,
            Ok(()) => {}
            Err(error) => {
                let name = Some(game.name.as_str());
                let choice = show_error(window, console, font, name, &error.to_string(), !single);
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
    if let Err(error) = std::fs::create_dir_all(BASE) {
        return report(&format!("cannot create {BASE}: {error}"));
    }
    sys::prefer_core(MAIN_CORE);
    // The VM and glyphs need every cycle; the system restores clocks on exit.
    sys::set_max_clocks();
    let mut window = match sys::VitaWindow::open() {
        Ok(window) => window,
        Err(error) => return system_error(error),
    };
    // Dropped before the window, which owns its GXM context.
    let mut gpu = match open_gpu(&mut window) {
        Ok(gpu) => gpu,
        Err(error) => return system_error(&error.to_string()),
    };
    let console = console(sys::confirm_is_cross());
    let font = match encoding::load_host_font(Path::new(BASE), || {
        std::fs::read(SYSTEM_FONT).map_err(|error| {
            format!("no font.ttf and no system font {SYSTEM_FONT}: {error}").into()
        })
    }) {
        Ok(font) => font,
        Err(error) => return system_error(&format!("cannot load the font: {error}")),
    };
    session(&mut window, gpu.as_mut(), &console, &font);
}

/// The GPU compositor unless `renderer.txt` asks for software rendering. A GPU that
/// fails to start is an error, not a silent fallback.
fn open_gpu(window: &mut sys::VitaWindow) -> Result<Option<gpu::GpuRenderer>> {
    if !cmvs_console::gpu::use_gpu(Path::new(BASE))? {
        return Ok(None);
    }
    let device = window.open_gpu().map_err(|error| {
        format!("{error}\nWrite cpu to {BASE}/renderer.txt to render in software.")
    })?;
    Ok(Some(gpu::GpuRenderer::new(device)?))
}

fn report(error: &str) {
    eprintln!("Vita player: {error}");
    let _ = std::fs::write(LOG_PATH, error);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirm_and_cancel_follow_the_system_enter_button() {
        for (cross, confirm, cancel) in [
            (true, button::CROSS, button::CIRCLE),
            (false, button::CIRCLE, button::CROSS),
        ] {
            let console = console(cross);
            assert_eq!(console.buttons.confirm, confirm);
            let codes = &console.buttons.codes;
            assert!(codes.contains(&(confirm, controls::MOUSE_LEFT)));
            assert!(codes.contains(&(cancel, controls::MOUSE_RIGHT)));
        }
        assert_eq!(console(false).labels.confirm, "○");
        let codes = console(true).buttons.codes;
        assert!(codes.contains(&(button::R, controls::CTRL)));
    }

    #[test]
    fn the_viewport_rectangle_is_in_screen_pixels() {
        // 800x600 contained in 960x544: a 725 px wide picture centred.
        let view = Viewport::new([800, 600], SCREEN, 1).unwrap();
        assert_eq!(view_rect(view), [117.0, 0.0, 725.0, 544.0]);
    }

    #[test]
    fn movie_presentation_preserves_letterboxing_inside_the_screen_viewport() {
        let canvas = [1280, 720];
        let view = Viewport::new(canvas, [960, 544], 1).unwrap();
        assert_eq!(
            movie_view_rect(view, canvas, [640, 480]).unwrap(),
            [120.0, 2.0, 720.0, 540.0]
        );
    }
}

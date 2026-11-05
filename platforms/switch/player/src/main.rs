mod audio;
mod backend;
mod boost;
mod canvas;
mod controls;
mod error_screen;
mod gpu;
mod library;
mod menu;
mod text_input;
#[cfg(target_os = "horizon")]
mod sys {
    pub use cmvs_native::{
        SwitchAudioOut, SwitchGpu, SwitchWindow, set_cpu_boost, shared_font, show_system_error,
        software_keyboard,
    };
}
#[cfg(not(target_os = "horizon"))]
#[path = "stub.rs"]
mod sys;

use cmvs_core::{
    input::KeyBindings,
    profile::Registry,
    raster::RgbaImage,
    runner::{Startup, StartupEvent, load_start},
    viewport::{Viewport, presentation_mode},
};
use cmvs_font::font::FontRenderer;
use controls::Controls;
use std::{
    error::Error,
    io::{ErrorKind, Read},
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "sdmc:/switch/cmvs-rs";
const LOG_PATH: &str = "sdmc:/switch/cmvs-rs/error.log";
/// `LOG_PATH` as the player sees it on the SD card.
const LOG: &str = "switch/cmvs-rs/error.log";
pub(crate) const SCREEN: [u32; 2] = [1280, 720];
/// VM budget per loop iteration before polling input and the applet again.
const BUDGET: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(16);

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// `font.ttf` beside the program wins; otherwise the system's shared Japanese font.
fn font_bytes(base: &Path) -> Result<Vec<u8>> {
    match std::fs::read(base.join("font.ttf")) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Ok(sys::shared_font().ok_or("no font.ttf and no system shared font")?)
        }
        Err(error) => Err(error.into()),
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

/// `renderer.txt` beside the program holds `gpu` (the default when absent) or `cpu`.
fn use_gpu(setting: Option<&str>) -> Result<bool> {
    match setting.map(str::trim) {
        None | Some("gpu") => Ok(true),
        Some("cpu") => Ok(false),
        Some(other) => Err(format!("renderer.txt must be gpu or cpu, not {other:?}").into()),
    }
}

fn read_renderer_setting() -> Result<Option<String>> {
    match std::fs::read_to_string(Path::new(BASE).join("renderer.txt")) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn render_gpu(
    startup: &Startup<'_>,
    font: &mut FontRenderer,
    gpu: &mut gpu::GpuRenderer,
    linear: bool,
) -> Result<()> {
    let size = startup.screen_size();
    if let Some(frame) = startup.movie_frame() {
        return gpu.render_image(frame, size, linear);
    }
    let objects = font.prepare_objects(startup.messages())?;
    let mut extra: Vec<_> = objects.iter().collect();
    extra.extend(startup.messages().visible_window_objects());
    let frame = cmvs_core::scene_render::prepare_scene(
        startup.scene(),
        Some(startup.camera()),
        &extra,
        size[0],
        size[1],
    )?;
    gpu.render(&frame)
}

fn render(startup: &Startup<'_>, font: &mut FontRenderer) -> Result<RgbaImage> {
    if let Some(frame) = startup.movie_frame() {
        return Ok(frame.clone());
    }
    let objects = font.prepare_objects(startup.messages())?;
    let mut extra: Vec<_> = objects.iter().collect();
    extra.extend(startup.messages().visible_window_objects());
    let size = startup.screen_size();
    Ok(cmvs_core::scene_render::render_scene(
        startup.scene(),
        Some(startup.camera()),
        &extra,
        size[0],
        size[1],
    )?)
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
    let game = Registry::builtin().identify_game(root, None)?;
    let record = game.selection.record.ok_or("unregistered game")?;
    let script = load_start(root, &game.files)?;
    let mut startup = Startup::new(&script, root, &game, 0)?;
    let user = base.join("users").join(&record.game);
    startup.set_user_data_directory(&user)?;
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    let (audio, audio_thread) = audio::AudioThread::start(root)?;
    let audio_control = audio.host_control();
    startup.set_audio_backend(Box::new(audio))?;
    startup.set_decode_movie_frames(true);
    startup.set_record_text(true);
    startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
    let mut font = FontRenderer::from_bytes(font_bytes(base)?, 0)?;
    font.set_user_directory(&user)?;
    startup.set_glyph_metrics(font.metrics());
    if !font.has_face(startup.settings().font_face()) {
        startup.set_font_face(cmvs_core::settings::DEFAULT_FONT_FACE)?;
    }
    let mut keys = String::new();
    std::fs::File::open(root.join("key.cfg"))?
        .take(65537)
        .read_to_string(&mut keys)?;
    let bindings = KeyBindings::parse(&keys)?;
    let size = startup.screen_size();
    let mut backend = backend::Backend {
        display: startup.display_preferences(),
        size,
        frame: None,
        gpu,
        modal_shown: false,
    };
    backend.display.render_mode =
        cmvs_core::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut surface = match backend.gpu {
        Some(_) => Vec::new(),
        None => vec![0u32; (SCREEN[0] * SCREEN[1]) as usize],
    };
    let mut controls = Controls::new(size);
    // The button that started the game must not click it.
    controls.suppress_held(startup.input_mut());
    let mut focused = true;
    let mut input_due = true;
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
            startup.set_focused(focused);
            controls.focus(focused, startup.input_mut());
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
        if input_due {
            if let Some(warp) = startup.input_mut().take_warp() {
                controls.warp(warp);
            }
            controls.frame(&bindings, startup.input_mut());
            input_due = false;
        }
        let budget = Instant::now();
        while budget.elapsed() < BUDGET {
            let tick = Instant::now();
            let event = startup.tick(&mut backend).map_err(|error| {
                let vm = startup.machine();
                format!(
                    "{error}\nat {} / 0x{:x} after {} steps",
                    vm.current_name().escape_ascii(),
                    vm.pc(),
                    startup.steps()
                )
            })?;
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
            match event {
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::Quit => {
                    startup.save_preferences(backend.display)?;
                    return Ok(Ended::Quit);
                }
                StartupEvent::Inactive => {
                    load.interrupted();
                    std::thread::sleep(IDLE);
                    last_frame = Instant::now();
                    break;
                }
                StartupEvent::Yield => {
                    let now = Instant::now();
                    load.frame(now);
                    startup.advance_decoded_frame(millis(now.duration_since(last_frame)))?;
                    last_frame = now;
                    let linear = backend.display.render_mode == 1;
                    if let Some(gpu) = &mut backend.gpu {
                        render_gpu(&startup, &mut font, gpu, linear)?;
                        let cursor = controls
                            .cursor_visible()
                            .then(|| view.to_target(controls.pointer()));
                        gpu.present(mode, linear, cursor)?;
                    } else {
                        let frame = render(&startup, &mut font)?;
                        surface.fill(0);
                        frame.draw_scaled(&mut surface, SCREEN[0], SCREEN[1], mode, linear)?;
                        if controls.cursor_visible() {
                            draw_cursor(&mut surface, view, controls.pointer());
                        }
                        window.present(&surface)?;
                        backend.frame = Some(frame);
                    }
                    input_due = true;
                    break;
                }
                event => return Err(format!("game stopped: {event:?}").into()),
            }
        }
    };
    startup.save_preferences(backend.display)?;
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
    let mut menu = menu::Menu::new(selected, games.len(), &first);
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
        let image = error_screen::draw(font, game, message, LOG, can_return);
        show(window, gpu, &image)?;
        let mut prompt = error_screen::ErrorPrompt::new(&first, can_return);
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
    let gpu = match read_renderer_setting().and_then(|text| use_gpu(text.as_deref())) {
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
    let font = match font_bytes(Path::new(BASE)).and_then(|bytes| {
        Ok(ab_glyph::FontVec::try_from_vec_and_index(bytes, 0).map_err(|_| "invalid font")?)
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
    fn renderer_setting_defaults_to_gpu_and_rejects_unknown_names() {
        assert!(use_gpu(None).unwrap());
        assert!(use_gpu(Some("gpu\n")).unwrap());
        assert!(!use_gpu(Some(" cpu\r\n")).unwrap());
        assert!(use_gpu(Some("vulkan")).is_err());
    }
}

mod audio;
mod backend;
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
    controls::{self, Controls},
    encoding::{self, SystemFont},
    error_screen, library, menu,
};
use cmvs_core::{
    input::KeyBindings,
    profile::Registry,
    raster::RgbaImage,
    runner::{Startup, StartupEvent, load_start},
    viewport::{Viewport, presentation_mode},
};
use cmvs_font::font::FontRenderer;
use cmvs_native::vita_button as button;
use std::{
    error::Error,
    io::{ErrorKind, Read},
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BASE: &str = "ux0:data/cmvs-rs";
const LOG_PATH: &str = "ux0:data/cmvs-rs/error.log";
/// The system's Japanese gothic face (OpenType CFF), used without `font.ttf`.
const SYSTEM_FONT: &str = "sa0:data/font/pvf/jpn0.pvf";
/// The system's simplified Chinese gothic face (TrueType), for GBK and UTF-8 games.
const CHINESE_FONT: &str = "sa0:data/font/pvf/cn0.pvf";
pub(crate) const SCREEN: [u32; 2] = [960, 544];
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

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// `font.ttf` beside the program wins; otherwise the system's Japanese font.
fn font_bytes(base: &Path) -> Result<Vec<u8>> {
    match std::fs::read(base.join("font.ttf")) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            std::fs::read(SYSTEM_FONT).map_err(|error| {
                format!("no font.ttf and no system font {SYSTEM_FONT}: {error}").into()
            })
        }
        Err(error) => Err(error.into()),
    }
}

/// `font.ttf` beside the program alone wins; otherwise the system fonts the game's text needs,
/// the first available one primary and the rest as fallbacks.
fn game_font(base: &Path, encoding: cmvs_core::TextEncoding) -> Result<FontRenderer> {
    let renderer = |bytes| -> Result<FontRenderer> {
        Ok(FontRenderer::from_bytes(bytes, 0)?.with_text_encoding(encoding))
    };
    match std::fs::read(base.join("font.ttf")) {
        Ok(bytes) => return renderer(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut fonts = encoding::system_fonts(encoding).iter().filter_map(|&font| {
        std::fs::read(match font {
            SystemFont::Japanese => SYSTEM_FONT,
            SystemFont::SimplifiedChinese => CHINESE_FONT,
        })
        .ok()
    });
    let font = renderer(
        fonts
            .next()
            .ok_or("no font.ttf and no system font in sa0:data/font/pvf")?,
    )?;
    for bytes in fonts {
        font.add_fallback(bytes, 0)?;
    }
    Ok(font)
}

/// Composites the scene on the GPU; the caller presents the canvas.
fn render_gpu(
    startup: &Startup<'_>,
    font: &mut FontRenderer,
    gpu: &mut gpu::GpuRenderer,
) -> Result<()> {
    let objects = font.prepare_objects(startup.messages())?;
    let mut extra: Vec<_> = objects.iter().collect();
    extra.extend(startup.messages().visible_window_objects());
    let size = startup.screen_size();
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

/// The screen rectangle a viewport covers, as the window draws it.
fn view_rect(view: Viewport) -> [f32; 4] {
    [
        view.left as f32,
        view.top as f32,
        view.width as f32,
        view.height as f32,
    ]
}

fn run(
    window: &mut sys::VitaWindow,
    gpu: Option<&mut gpu::GpuRenderer>,
    console: &Console,
    root: &Path,
) -> Result<()> {
    let base = Path::new(BASE);
    let game = Registry::builtin().identify_game(root, None)?;
    let record = game.selection.record.ok_or("unregistered game")?;
    let script = load_start(root, &game.files)?;
    let encoding = cmvs_console::encoding::select(root, &game.files, &script)?;
    let mut startup = Startup::new_with_encoding(&script, root, &game, 0, encoding)?;
    let user = base.join("users").join(&record.game);
    startup.set_user_data_directory(&user)?;
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    let (audio, audio_thread) = audio::AudioThread::start(root)?;
    let audio_control = audio.host_control();
    startup.set_audio_backend(Box::new(audio))?;
    startup.set_decode_movie_frames(true);
    // Core 0 runs the VM and rendering, core 1 the audio thread.
    startup.set_mgv_factory(Box::new(cmvs_video::PrefetchTheoraFactory {
        on_thread_start: || {
            sys::prefer_core(2);
        },
    }));
    startup.set_scene_prefetch(|| {
        sys::prefer_core(2);
    });
    // Scenes reload the same images; decoding dominates their load time. Entries share
    // pixels with the scene objects showing them, within the 256 MiB heap.
    startup.set_decoded_image_cache(64 << 20);
    startup.set_record_text(true);
    startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
    let mut font = game_font(base, encoding)?;
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
        encoding,
    };
    backend.display.render_mode =
        cmvs_core::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut controls = Controls::new(size, console.buttons.clone());
    // The button that started the game must not click it.
    controls.suppress_held(startup.input_mut());
    let mut focused = true;
    let mut input_due = true;
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
            startup.set_focused(focused);
            controls.focus(focused, startup.input_mut());
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
        if input_due {
            if let Some(warp) = startup.input_mut().take_warp() {
                controls.warp(warp);
            }
            controls.frame(&bindings, startup.input_mut());
            input_due = false;
        }
        let budget = Instant::now();
        while budget.elapsed() < BUDGET {
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
                // Time inside the IME is not game time, and the button that closed
                // it must not click the game.
                controls.suppress_held(startup.input_mut());
                last_frame = Instant::now();
                last_poll = last_frame;
            }
            match event {
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::Quit => {
                    startup.save_preferences(backend.display)?;
                    return Ok(());
                }
                StartupEvent::Inactive => {
                    std::thread::sleep(IDLE);
                    last_frame = Instant::now();
                    break;
                }
                StartupEvent::Yield => {
                    let now = Instant::now();
                    startup.advance_decoded_frame(millis(now.duration_since(last_frame)))?;
                    last_frame = now;
                    let linear = backend.display.render_mode == 1;
                    let cursor = controls.cursor_visible().then(|| {
                        let at = view.to_target(controls.pointer());
                        [at[0] as f32, at[1] as f32]
                    });
                    match &mut backend.gpu {
                        // Movie frames are shown as decoded, like software frames.
                        Some(gpu) if startup.movie_frame().is_none() => {
                            render_gpu(&startup, &mut font, gpu)?;
                            gpu.present(view_rect(view), linear, cursor)?;
                            backend.frame = None;
                        }
                        gpu => {
                            let frame = render(&startup, &mut font)?;
                            window.present(
                                frame.pixels(),
                                [frame.width(), frame.height()],
                                view_rect(view),
                                linear,
                                cursor,
                            )?;
                            if let Some(gpu) = gpu {
                                gpu.forget_presented();
                            }
                            backend.frame = Some(frame);
                        }
                    }
                    input_due = true;
                    break;
                }
                event => return Err(format!("game stopped: {event:?}").into()),
            }
        }
    }
    startup.save_preferences(backend.display)?;
    Ok(())
}

/// Shows a full-screen host page.
fn show(window: &mut sys::VitaWindow, image: &RgbaImage) -> Result<()> {
    let rect = [0.0, 0.0, SCREEN[0] as f32, SCREEN[1] as f32];
    Ok(window.present(image.pixels(), SCREEN, rect, false, None)?)
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
    let font = match font_bytes(Path::new(BASE)).and_then(|bytes| {
        Ok(ab_glyph::FontVec::try_from_vec_and_index(bytes, 0).map_err(|_| "invalid font")?)
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
}

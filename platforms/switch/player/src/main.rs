mod audio;
mod backend;
mod controls;
#[cfg(target_os = "horizon")]
mod sys {
    pub use cmvs_native::{SwitchAudioOut, SwitchWindow, shared_font};
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
const SCREEN: [u32; 2] = [1280, 720];
/// VM budget per loop iteration before polling input and the applet again.
const BUDGET: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(16);

fn millis(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// `font.ttf` beside the program wins; otherwise the system's shared Japanese font.
fn load_font(base: &Path) -> Result<FontRenderer> {
    let path = base.join("font.ttf");
    match std::fs::metadata(&path) {
        Ok(_) => FontRenderer::read(&path, 0),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let bytes = sys::shared_font().ok_or("no font.ttf and no system shared font")?;
            Ok(FontRenderer::from_bytes(bytes, 0)?)
        }
        Err(error) => Err(error.into()),
    }
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

fn run(window: &mut sys::SwitchWindow) -> Result<()> {
    let base = Path::new(BASE);
    let root = base.join("game");
    let game = Registry::builtin().identify_game(&root, None)?;
    let record = game.selection.record.ok_or("unregistered game")?;
    let script = load_start(&root, &game.files)?;
    let mut startup = Startup::new(&script, &root, &game, 0)?;
    let user = base.join("users").join(&record.game);
    startup.set_user_data_directory(&user)?;
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    let (audio, audio_thread) = audio::AudioThread::start(&root)?;
    let audio_control = audio.host_control();
    startup.set_audio_backend(Box::new(audio))?;
    startup.set_decode_movie_frames(true);
    startup.set_record_text(true);
    startup.set_save_clock(Box::new(cmvs_core::runner::utc_save_time));
    let mut font = load_font(base)?;
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
    };
    backend.display.render_mode =
        cmvs_core::platform::supported_render_mode(&backend, backend.display.render_mode)?;

    let mut surface = vec![0u32; (SCREEN[0] * SCREEN[1]) as usize];
    let mut controls = Controls::new(size);
    let mut focused = true;
    let mut input_due = true;
    let mut last_poll = Instant::now();
    let mut last_frame = last_poll;
    loop {
        let sample = window.poll();
        if sample.exit {
            break;
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
                    let frame = render(&startup, &mut font)?;
                    surface.fill(0);
                    frame.draw_scaled(
                        &mut surface,
                        SCREEN[0],
                        SCREEN[1],
                        mode,
                        backend.display.render_mode == 1,
                    )?;
                    if controls.cursor_visible() {
                        draw_cursor(&mut surface, view, controls.pointer());
                    }
                    window.present(&surface)?;
                    backend.frame = Some(frame);
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

fn main() {
    // The window lives until main returns, after the error log is written: dropping
    // it unlocks exit, after which a pending HOME close may end the process.
    let mut window = match sys::SwitchWindow::open() {
        Ok(window) => window,
        Err(error) => return report(error),
    };
    if let Err(error) = run(&mut window) {
        report(&error.to_string());
    }
}

fn report(error: &str) {
    eprintln!("Switch player: {error}");
    let _ = std::fs::write(format!("{BASE}/error.log"), error);
}

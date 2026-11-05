mod backend;
use cmvs_core::{
    input::{CodeInput, KeyBindings},
    profile::Registry,
    runner::{Startup, StartupEvent, load_start},
};
use cmvs_font::font::FontRenderer;
#[cfg(target_os = "horizon")]
use cmvs_native::SwitchWindow;
use std::{
    error::Error,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
// Allows the complete host to be type-checked on development machines.
#[cfg(not(target_os = "horizon"))]
struct SwitchWindow;
#[cfg(not(target_os = "horizon"))]
impl SwitchWindow {
    fn open() -> Result<Self, &'static str> {
        Err("Switch host requires Horizon")
    }
    fn poll(&mut self) -> u32 {
        1
    }
    fn present(&mut self, _: &[u32]) -> Result<(), &'static str> {
        Err("Switch host requires Horizon")
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let base = Path::new("sdmc:/switch/cmvs-rs");
    let root = base.join("game");
    let game = Registry::builtin().identify_game(&root, None)?;
    let record = game.selection.record.ok_or("unregistered game")?;
    let script = load_start(&root, &game.files)?;
    let mut startup = Startup::new(&script, &root, &game, 0)?;
    let user = base.join("users").join(&record.game);
    startup.set_user_data_directory(&user)?;
    startup.set_block_unreviewed(false);
    startup.set_block_presumed(false);
    startup.set_decode_movie_frames(true);
    startup.set_record_text(true);
    let mut font = FontRenderer::read(&base.join("font.ttf"), 0)?;
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
    let mut window = SwitchWindow::open()?;
    let mut surface = vec![0u32; 1280 * 720];
    let mut codes = CodeInput::default();
    let mut pointer = [size[0] as i32 / 2, size[1] as i32 / 2];
    let mut input_due = true;
    let mut last = Instant::now();
    loop {
        let buttons = window.poll();
        if buttons & 1 != 0 {
            break;
        }
        if input_due {
            if let Some(warp) = startup.input_mut().take_warp() {
                pointer = warp;
            }
            pointer[0] = (pointer[0] + if buttons & 16 != 0 { 8 } else { 0 }
                - if buttons & 8 != 0 { 8 } else { 0 })
            .clamp(0, size[0] as i32 - 1);
            pointer[1] = (pointer[1] + if buttons & 64 != 0 { 8 } else { 0 }
                - if buttons & 32 != 0 { 8 } else { 0 })
            .clamp(0, size[1] as i32 - 1);
            let mut held = [false; 128];
            held[1] = buttons & 2 != 0;
            held[3] = buttons & 4 != 0;
            codes.update(&bindings, &held, startup.input_mut());
            startup.input_mut().set_pointer(pointer[0], pointer[1]);
            input_due = false;
        }
        let budget = Instant::now();
        let mut quit = false;
        while budget.elapsed() < Duration::from_millis(8) {
            match startup.tick(&mut backend)? {
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
                StartupEvent::Quit => {
                    quit = true;
                    break;
                }
                StartupEvent::Yield => {
                    let now = Instant::now();
                    startup.advance_decoded_frame(
                        now.duration_since(last).as_millis().min(u32::MAX as u128) as u32,
                    )?;
                    last = now;
                    let frame = if let Some(frame) = startup.movie_frame() {
                        frame.clone()
                    } else {
                        let objects = font.prepare_objects(startup.messages())?;
                        let mut extra: Vec<_> = objects.iter().collect();
                        extra.extend(startup.messages().visible_window_objects());
                        cmvs_core::scene_render::render_scene(
                            startup.scene(),
                            Some(startup.camera()),
                            &extra,
                            size[0],
                            size[1],
                        )?
                    };
                    surface.fill(0);
                    frame.draw_scaled(
                        &mut surface,
                        1280,
                        720,
                        cmvs_core::viewport::presentation_mode(
                            backend.display.fullscreen,
                            backend.display.aspect_scaling,
                            backend.display.aspect_mode,
                        ),
                        backend.display.render_mode == 1,
                    )?;
                    // Cursor is host-only; exclude it from engine screenshot readback.
                    let mode = cmvs_core::viewport::presentation_mode(
                        backend.display.fullscreen,
                        backend.display.aspect_scaling,
                        backend.display.aspect_mode,
                    );
                    let view = cmvs_core::viewport::Viewport::new(size, [1280, 720], mode)?;
                    let at = view.to_target(pointer);
                    for d in -5..=5 {
                        for (x, y) in [
                            (at[0] as i32 + d, at[1] as i32),
                            (at[0] as i32, at[1] as i32 + d),
                        ] {
                            if (0..1280).contains(&x) && (0..720).contains(&y) {
                                surface[y as usize * 1280 + x as usize] = 0xffffff;
                            }
                        }
                    }
                    window.present(&surface)?;
                    backend.frame = Some(frame);
                    input_due = true;
                    break;
                }
                event => return Err(format!("game stopped: {event:?}").into()),
            }
        }
        if quit {
            break;
        }
    }
    startup.save_preferences(backend.display)?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Switch player: {error}");
        let _ = std::fs::write("sdmc:/switch/cmvs-rs/error.log", error.to_string());
    }
}

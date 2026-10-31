#![cfg(feature = "wgpu-renderer")]
use cmvs_core::{
    input::{CodeInput, KeyBindings},
    message::MESSAGE_WINDOWS,
    platform::RecordingBackend,
    profile::Registry,
    runner::{SnapshotPolicy, Startup, StartupEvent, load_start},
    scene_render::prepare_scene,
};
use cmvs_desktop::font::FontRenderer;
use cmvs_wgpu::{Renderer, wgpu};
use std::{error::Error, path::PathBuf, time::Instant};

#[test]
#[ignore = "requires CMVS_GAME_ROOT, CMVS_FONT_PATH and GPU; game data stays outside Git"]
fn external_fixed_clock_gpu_scene_comparison() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").ok_or("CMVS_GAME_ROOT")?);
    let font_path = PathBuf::from(std::env::var_os("CMVS_FONT_PATH").ok_or("CMVS_FONT_PATH")?);
    let game = Registry::builtin().identify_game(&root, None)?;
    let script = load_start(&root, &game.files)?;
    let mut run = Startup::new(&script, &root, &game, 0)?;
    run.set_snapshot_policy(SnapshotPolicy::ContinueWithoutImage);
    run.set_skip_auto_resume(true)?;
    run.set_record_text(true);
    run.set_decode_movie_frames(true);
    let mut font = FontRenderer::read(&font_path, 0)?;
    run.set_glyph_metrics(font.metrics());
    let bindings = KeyBindings::parse(&std::fs::read_to_string(root.join("key.cfg"))?)?;
    let mut codes = CodeInput::default();
    let mut backend = RecordingBackend::default();
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let (mut gpu, adapter) = Renderer::new(&instance, None)?;
    println!("adapter={:?}", adapter.get_info());
    let mut yields = 0u64;
    let mut phase = 0;
    let mut compared = 0;
    let mut worst = 0u8;
    let mut cpu_time = std::time::Duration::ZERO;
    let mut gpu_time = std::time::Duration::ZERO;
    let mut read_time = std::time::Duration::ZERO;
    let mut warm_time = std::time::Duration::ZERO;
    for iteration in 0u64..40_000_000 {
        if iteration.is_multiple_of(100_000) {
            run.set_audio_backend(Box::new(cmvs_core::audio::RecordingAudio::default()))?;
            backend = RecordingBackend::default();
        }
        if phase == 0
            && run.machine().current_name() == b"menu.ps3"
            && run.machine().pc() == 0x38f00
        {
            let group = run.buttons().group(0).ok_or("title buttons missing")?;
            let b = &group.buttons[0];
            let point = [b.rect[0] + b.rect[2] / 2, b.rect[1] + b.rect[3] / 2];
            run.input_mut().set_pointer(point[0], point[1]);
            phase = 1;
        }
        let event = run.tick(&mut backend)?;
        if !matches!(
            event,
            StartupEvent::Progress
                | StartupEvent::Yield
                | StartupEvent::SnapshotCaptured
                | StartupEvent::MainEnteredWithoutSnapshot
        ) {
            return Err(format!("unexpected {event:?}").into());
        }
        if event != StartupEvent::Yield {
            continue;
        }
        yields += 1;
        run.advance_frame(16)?;
        if yields.is_multiple_of(200) {
            if let Some(image) = run.movie_frame() {
                let frame = gpu.render_image(image, [1280, 720], true)?;
                assert_eq!(frame.read()?.width(), 1280);
            } else {
                let objects = font.prepare_objects(run.messages())?;
                let mut additional: Vec<_> = objects.iter().collect();
                additional.extend(
                    (0..MESSAGE_WINDOWS)
                        .filter_map(|i| run.messages().window(i))
                        .filter(|w| w.visible && !w.hidden)
                        .map(|w| &w.object),
                );
                let frame = prepare_scene(run.scene(), Some(run.camera()), &additional, 1280, 720)?;
                let clock = Instant::now();
                let output = gpu.render(&frame)?;
                gpu_time += clock.elapsed();
                let clock = Instant::now();
                let actual = output.read()?;
                read_time += clock.elapsed();
                let clock = Instant::now();
                let _warm = gpu.render(&frame)?;
                gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                warm_time += clock.elapsed();
                let clock = Instant::now();
                let expected = frame.render_cpu()?;
                cpu_time += clock.elapsed();
                let max = actual
                    .pixels()
                    .iter()
                    .zip(expected.pixels())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                worst = worst.max(max);
                let changed = actual
                    .pixels()
                    .iter()
                    .zip(expected.pixels())
                    .filter(|(a, b)| a.abs_diff(**b) > 2)
                    .count();
                println!(
                    "sample={compared} yields={yields} script={} pc={:x} max={max} bytes_over_2={changed}",
                    run.machine().current_name().escape_ascii(),
                    run.machine().pc()
                );
                compared += 1;
            }
        }
        let waiting = matches!(run.machine().pending_system(), Some((_, 0x153)))
            && run.machine().repeat_count() == 2;
        let mut now = [false; 128];
        if phase > 0 && phase < 4 {
            phase += 1;
            now[1] = phase == 2;
        }
        if phase >= 4 {
            now[24] = true;
            now[1] = waiting || yields.is_multiple_of(80);
        }
        codes.update(&bindings, &now, run.input_mut());
        if yields >= 8000 {
            break;
        }
    }
    assert!(phase >= 4 && compared >= 20, "insufficient scene coverage");
    println!(
        "RESULT samples={compared} yields={yields} worst={worst} cpu={cpu_time:?} gpu_submit={gpu_time:?} readback={read_time:?} warm_render_and_wait={warm_time:?} uploaded={}",
        gpu.uploaded_bytes
    );
    gpu.check()?;
    Ok(())
}

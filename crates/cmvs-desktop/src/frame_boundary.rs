#[cfg(all(test, feature = "wgpu-renderer"))]
mod external {
    use crate::font::FontRenderer;
    use cmvs_core::{
        host::input::CodeInput, host::input::KeyBindings, host::platform::RecordingBackend,
        profile::Registry, render::scene_render::prepare_scene, runtime::FrameBudget,
        runtime::FrameDriver, runtime::FrameOutcome, runtime::Runtime, runtime::load_start,
    };
    use cmvs_wgpu::{Renderer, wgpu};
    use std::{error::Error, path::PathBuf};

    #[test]
    #[ignore = "requires CMVS_GAME_ROOT, CMVS_FONT_PATH, original save000.dat, and GPU"]
    fn external_save_menu_renders_only_completed_vm_frames()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let root = PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").ok_or("CMVS_GAME_ROOT")?);
        let font_path = PathBuf::from(std::env::var_os("CMVS_FONT_PATH").ok_or("CMVS_FONT_PATH")?);
        let user = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "save-menu-frame-boundary-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
        std::fs::create_dir(&user)?;
        let source = std::env::var_os("CMVS_SAVE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("save"));
        for name in ["system.dat", "save000.dat"] {
            std::fs::copy(source.join(name), user.join(name))?;
        }
        let game = Registry::builtin().identify_game(&root, None)?;
        let script = load_start(&root, &game.files)?;
        let mut run = Runtime::new(&script, &root, &game, 0)?;
        run.set_user_data_directory(&user)?;
        run.set_skip_auto_resume(true)?;
        run.set_record_text(true);
        let mut font = FontRenderer::read(&font_path, 0)?;
        run.set_glyph_metrics(font.metrics());
        let bindings = KeyBindings::parse(&std::fs::read_to_string(root.join("key.cfg"))?)?;
        let mut codes = CodeInput::default();
        let mut backend = RecordingBackend::default();
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let (mut gpu, adapter) = Renderer::new(&instance, None)?;
        println!("adapter={:?}", adapter.get_info());
        let mut boundary = FrameDriver::default();
        let (mut loaded, mut click_frame, mut point, mut checked, mut partial, mut old_errors) =
            (false, 0u32, [-1, -1], 0u32, 0u32, 0u32);
        let mut menu_frames = 0;
        for iteration in 0u64..8_000_000 {
            if iteration.is_multiple_of(100_000) {
                run.set_audio_backend(Box::new(cmvs_core::host::audio::RecordingAudio::default()))?;
                backend = RecordingBackend::default();
            }
            if !loaded
                && run.machine().current_name() == b"menu.ps3"
                && run.machine().pc() == 0x38f00
            {
                run.load_slot(0, &mut backend)?;
                loaded = true;
            }
            boundary.sample_input(run.input_mut(), |_| {});
            let event = boundary.run_slice(&mut run, &mut FrameBudget::steps(1), |runtime| {
                runtime.tick(&mut backend)
            })?;
            // Deliberately exhaust the host budget after every single VM tick.
            // Detect, but never render, children between allocation and initialization.
            if loaded
                && click_frame > 0
                && !boundary.ready()
                && run.scene().objects().values().any(|o| {
                    o.shown
                        && o.draw.is_some()
                        && o.flags & 0x8000_0000 != 0
                        && o.children.values().any(|c| c.shown && c.draw.is_none())
                })
            {
                partial += 1;
                if old_errors == 0
                    && prepare_scene(run.scene(), Some(run.camera()), &[], 1280, 720)
                        .err()
                        .is_some_and(|e| e.0 == "shown child object has no draw state")
                {
                    old_errors += 1;
                    println!(
                        "old error reproduced at {} / 0x{:x}",
                        run.machine().current_name().escape_ascii(),
                        run.machine().pc()
                    );
                }
            }
            match event {
                FrameOutcome::BudgetExhausted => continue,
                FrameOutcome::Yield => {}
                other => return Err(format!("unexpected {other:?}").into()),
            }
            boundary.finish_frame(&mut run, 16)?;
            assert!(boundary.ready());
            if loaded
                && click_frame == 0
                && run.machine().current_name() == b"sn1001.ps3"
                && matches!(run.machine().pending_system(), Some((_, 0x153)))
                && run.machine().repeat_count() >= 2
            {
                let window = run.messages().window(7).ok_or("toolbar window missing")?;
                let [x, y, w, h] = window.buttons[1].hit.map(i32::from);
                point = [
                    window.position[0] + x + w / 2,
                    window.position[1] + y + h / 2,
                ];
                click_frame = 1;
            }
            if click_frame > 0 {
                let composition = font.compose(run.scene(), run.camera(), run.messages())?;
                let frame = composition.prepare()?;
                let output = gpu.render(&frame)?;
                if checked.is_multiple_of(10) {
                    let actual = output.read()?;
                    let expected = frame.render_cpu()?;
                    let max = actual
                        .pixels()
                        .iter()
                        .zip(expected.pixels())
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    assert!(max <= 2, "save menu pixel difference {max}");
                }
                checked += 1;
                if run
                    .buttons()
                    .group(1)
                    .is_some_and(|g| g.buttons.iter().any(|b| (70..80).contains(&b.id)))
                {
                    menu_frames += 1;
                }
                click_frame += 1;
            }
            let mut now = [false; 128];
            now[1] = (4..7).contains(&click_frame);
            codes.update(&bindings, &now, run.input_mut());
            run.input_mut().set_pointer(point[0], point[1]);
            if menu_frames >= 100 {
                break;
            }
        }
        println!(
            "RESULT completed={checked} save_frames={menu_frames} partial_ticks={partial} old_errors={old_errors}"
        );
        assert!(
            menu_frames >= 100 && partial > 0 && old_errors == 1,
            "save transition coverage missing"
        );
        gpu.check()?;
        drop(run);
        std::fs::remove_dir_all(user)?;
        Ok(())
    }
}

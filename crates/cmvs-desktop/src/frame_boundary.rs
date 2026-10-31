//! A cooperative host budget is not an engine frame boundary.
use cmvs_core::runner::StartupEvent;

#[derive(Default)]
pub(crate) struct FrameBoundary {
    ready: bool,
}
impl FrameBoundary {
    pub(crate) fn observe(&mut self, event: StartupEvent) {
        if event != StartupEvent::Inactive {
            self.ready = event == StartupEvent::Yield;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::{
        scene::{DrawState, SceneHost, SceneObject},
        scene_render::prepare_scene,
    };

    #[test]
    fn budget_slices_do_not_publish_partly_initialized_menu_children() {
        let scene = SceneHost::default();
        let mut parent = SceneObject {
            draw: Some(DrawState::default()),
            flags: 0x8000_0000,
            ..SceneObject::default()
        };
        let mut boundary = FrameBoundary::default();
        boundary.observe(StartupEvent::Yield);
        assert!(boundary.ready());
        // 0x2022 creates a shown child; 0x2041 initializes its draw state later.
        parent.children.insert(0, SceneObject::default());
        boundary.observe(StartupEvent::Progress);
        assert!(!boundary.ready());
        assert_eq!(
            prepare_scene(&scene, None, &[&parent], 8, 4)
                .err()
                .unwrap()
                .0,
            "shown child object has no draw state"
        );
        boundary.observe(StartupEvent::SnapshotCaptured);
        assert!(!boundary.ready());
        parent.children.get_mut(&0).unwrap().draw = Some(DrawState::default());
        boundary.observe(StartupEvent::Progress);
        assert!(!boundary.ready());
        boundary.observe(StartupEvent::Yield);
        assert!(boundary.ready());
        prepare_scene(&scene, None, &[&parent], 8, 4)
            .unwrap()
            .render_cpu()
            .unwrap();
        boundary.observe(StartupEvent::Inactive);
        assert!(boundary.ready());
        boundary.observe(StartupEvent::Progress);
        boundary.observe(StartupEvent::Inactive);
        assert!(!boundary.ready());
    }

    #[test]
    fn a_real_yield_does_not_suppress_invalid_scene_errors() {
        let scene = SceneHost::default();
        let mut parent = SceneObject {
            draw: Some(DrawState::default()),
            flags: 0x8000_0000,
            ..SceneObject::default()
        };
        parent.children.insert(0, SceneObject::default());
        let mut boundary = FrameBoundary::default();
        boundary.observe(StartupEvent::Yield);
        assert!(boundary.ready());
        assert!(prepare_scene(&scene, None, &[&parent], 8, 4).is_err());
        boundary.observe(StartupEvent::Quit);
        assert!(!boundary.ready());
    }
}

#[cfg(all(test, feature = "wgpu-renderer"))]
mod external {
    use super::*;
    use crate::font::FontRenderer;
    use cmvs_core::{
        input::{CodeInput, KeyBindings},
        message::MESSAGE_WINDOWS,
        platform::RecordingBackend,
        profile::Registry,
        runner::{Startup, load_start},
        scene_render::prepare_scene,
    };
    use cmvs_wgpu::{Renderer, wgpu};
    use std::{error::Error, path::PathBuf};

    #[test]
    #[ignore = "requires CMVS_GAME_ROOT, CMVS_FONT_PATH, original save000.dat, and GPU"]
    fn external_save_menu_renders_only_completed_vm_frames() -> Result<(), Box<dyn Error>> {
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
        let mut run = Startup::new(&script, &root, &game, 0)?;
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
        let mut boundary = FrameBoundary::default();
        let (mut loaded, mut click_frame, mut point, mut checked, mut partial, mut old_errors) =
            (false, 0u32, [-1, -1], 0u32, 0u32, 0u32);
        let mut menu_frames = 0;
        for iteration in 0u64..8_000_000 {
            if iteration.is_multiple_of(100_000) {
                run.set_audio_backend(Box::new(cmvs_core::audio::RecordingAudio::default()))?;
                backend = RecordingBackend::default();
            }
            if !loaded
                && run.machine().current_name() == b"menu.ps3"
                && run.machine().pc() == 0x38f00
            {
                run.load_slot(0, &mut backend)?;
                loaded = true;
            }
            let event = run.tick(&mut backend)?;
            boundary.observe(event);
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
                StartupEvent::Progress | StartupEvent::SnapshotCaptured => continue,
                StartupEvent::Yield => {}
                other => return Err(format!("unexpected {other:?}").into()),
            }
            assert!(boundary.ready());
            run.advance_frame(16)?;
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
                let objects = font.prepare_objects(run.messages())?;
                let mut additional: Vec<_> = objects.iter().collect();
                additional.extend(
                    (0..MESSAGE_WINDOWS)
                        .filter_map(|i| run.messages().window(i))
                        .filter(|w| w.visible && !w.hidden)
                        .map(|w| &w.object),
                );
                let frame = prepare_scene(run.scene(), Some(run.camera()), &additional, 1280, 720)?;
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

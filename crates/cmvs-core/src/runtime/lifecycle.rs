//! QUIT and RESET for the hash-bound cmvs-20160201 layout.
use super::{Runtime, RuntimeEvent};
use crate::{dispatch::RuntimeCall, script_control::ScriptSource};
use std::error::Error;

impl Runtime {
    pub(super) fn handle_lifecycle(
        &mut self,
        request: &RuntimeCall,
    ) -> Result<RuntimeEvent, Box<dyn Error + Send + Sync>> {
        let command = request.command();
        // Load/validate the replacement before destructive cleanup. Failure leaves RESET
        // pending, with the old VM and hosts intact; no game file is ever written.
        let replacement = if command == 0x03 {
            let script = self.source.load(self.main_script_name)?;
            Some(crate::machine::Machine::prepare_main(
                self.main_script_name.to_vec(),
                script,
            )?)
        } else {
            None
        };
        if self.movies.has_player() {
            self.audio_backend.stop_movie_audio()?;
        }
        self.audio
            .stop_for_lifecycle(self.audio_backend.as_mut(), command == 0x03)?;
        if command == 0x01 {
            request.complete(&mut self.machine, 0x4000)?;
            self.movies.restore(Default::default());
            self.quit = true;
            return Ok(RuntimeEvent::Quit);
        }
        // The rebuilt scene drops the legacy effects; their destructors release the
        // prepared sounds and stop the walk's channel 1, which would otherwise play on.
        self.scene
            .release_legacy_effects(self.audio_backend.as_mut())?;
        self.finish_reset(replacement.expect("RESET replacement prepared"));
        Ok(RuntimeEvent::Progress)
    }

    fn finish_reset(&mut self, replacement: crate::machine::PreparedMain) {
        self.machine.replace_main(replacement);
        self.control.main_replaced();
        self.save_image = Default::default();
        self.thumbnail = None;
        let screen_size =
            crate::viewport::ScreenSize::new(self.screen_size()).expect("validated at startup");
        self.scene = crate::scene::SceneHost::for_profile(self.gate.profile());
        self.scene.set_screen_size(screen_size);
        self.camera.reset();
        self.buttons = crate::buttons::ButtonHost::for_profile(self.gate.profile());
        self.buttons.set_screen_size(screen_size);
        self.movies.restore(Default::default());
        self.messages.reset_objects();
        self.history = Default::default();
        self.sequence.reset_header();
        self.message_flags.clear_for_load();
        self.save_menu.reset_popup();
        self.timers.reset_frame_delta();
        self.frame_delta = 0;
        // RESET returns 0 in the dispatcher: do not advance or pop the new script.
        self.resume_run = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        machine::Machine, platform::RecordingBackend, profile::Registry, ps3::Script,
        snapshot::SaveImage,
    };

    /// A synthetic game whose start script requests a screen capture (0x2304), then
    /// asks whether one is held (0x2306).
    fn capture_fixture(tag: &str) -> (std::path::PathBuf, Vec<u8>) {
        let root = std::env::temp_dir().join(format!("cmvs-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/pack")).unwrap();
        std::fs::create_dir_all(root.join("save")).unwrap();
        let archive: Vec<_> = include_str!("../../tests/fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        std::fs::write(root.join("data/pack/script.cpz"), archive).unwrap();
        let code: Vec<u8> = [0x2304u16, 0x2306, 0x2306]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        std::fs::write(root.join("data/pack/start.ps3"), &raw).unwrap();
        (root, raw)
    }

    #[test]
    fn a_host_supplies_a_due_capture_or_the_next_step_draws_it() {
        let (root, raw) = capture_fixture("host-capture");
        let registry = Registry::builtin();
        let game = registry
            .identify_game(&root, Some("cmvs-20160201"))
            .unwrap();
        let script = Script::decode(&raw).unwrap();
        let mut backend = RecordingBackend::default();
        let requested = |host: bool| {
            let mut run = Runtime::new(&script, &root, &game, 0).unwrap();
            run.set_host_capture(host);
            let mut backend = RecordingBackend::default();
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
            assert!(run.capture().requested());
            run.advance_frame(16).unwrap();
            run
        };
        // Without a host frame the frame advance draws the capture itself.
        let run = requested(false);
        assert!(!run.capture_due() && !run.capture().requested());
        let [width, height] = run.screen_size();
        let drawn = run.capture().image().unwrap().clone();
        assert_eq!([drawn.width(), drawn.height()], [width, height]);

        let mut run = requested(true);
        assert!(run.capture_due() && run.capture().requested());
        assert!(run.capture().image().is_none());
        let wrong = crate::raster::RgbaImage::new(1, 1, vec![0; 4]).unwrap();
        assert!(run.supply_capture(wrong).is_err());
        assert!(run.capture_due());
        let pixels = [9, 8, 7, 0].repeat((width * height) as usize);
        let frame = crate::raster::RgbaImage::new(width, height, pixels).unwrap();
        run.supply_capture(frame.clone()).unwrap();
        assert!(!run.capture_due() && !run.capture().requested());
        // Held like a drawn capture: opaque.
        assert_eq!(
            &run.capture().image().unwrap().pixels()[..4],
            &[9, 8, 7, 255]
        );
        assert!(run.supply_capture(frame).is_err());

        // A host that supplies nothing leaves the capture to the next VM step.
        let mut run = requested(true);
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        assert!(!run.capture_due());
        assert_eq!(run.capture().image(), Some(&drawn));
        assert_eq!(run.machine.state().system_integer(0), Some(1));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn host_capture_allows_visible_and_auxiliary_glyphs_when_supplied_and_errors_if_drawn() {
        let (root, _) = capture_fixture("host-capture-glyphs");
        let registry = Registry::builtin();

        let build_script = |setup: &[(u16, &[u32])]| {
            let mut code = Vec::new();
            for &(op, args) in setup
                .iter()
                .chain([&(0x2304u16, &[][..]), &(0x2306u16, &[][..])])
            {
                for &arg in args.iter().rev() {
                    code.extend(0x440u16.to_le_bytes());
                    code.extend(4u16.to_le_bytes());
                    code.extend(arg.to_le_bytes());
                }
                code.extend(op.to_le_bytes());
            }
            let strings = b"\0\x82\xa0\0";
            let mut payload = code.clone();
            payload.extend_from_slice(strings);
            let mut raw = b"PS2A".to_vec();
            for word in [
                48u32,
                0,
                0,
                0,
                code.len() as u32,
                0,
                strings.len() as u32,
                0,
                0,
                payload.len() as u32,
                0,
            ] {
                raw.extend(word.to_le_bytes());
            }
            raw.extend(
                payload
                    .into_iter()
                    .map(|b| b.rotate_left(1).wrapping_add(124)),
            );
            raw
        };

        let visible_setup: &[(u16, &[u32])] = &[
            (0x2100, &[0, 0, 0]),
            (0x2103, &[0, 0, 0, 200, 200]),
            (0x2105, &[0, 24, 0, 0]),
            (0x210b, &[0, 1]),
        ];
        let auxiliary_setup: &[(u16, &[u32])] = &[
            (0x2380, &[0, 0, 24, 64]),
            (0x2385, &[0, 0, 0, 200, 200]),
            (0x238b, &[0, 1]),
        ];

        for (label, setup) in [("visible", visible_setup), ("auxiliary", auxiliary_setup)] {
            let raw = build_script(setup);
            std::fs::write(root.join("data/pack/start.ps3"), &raw).unwrap();
            let game = registry
                .identify_game(&root, Some("cmvs-20160201"))
                .unwrap();
            let script = Script::decode(&raw).unwrap();

            let request_capture = |host: bool| {
                let mut run = Runtime::new(&script, &root, &game, 0).unwrap();
                run.set_record_text(true);
                run.set_host_capture(host);
                let mut backend = RecordingBackend::default();
                while !run.capture().requested() {
                    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
                }
                run
            };

            // Software capture (`host_capture == false`) fails in advance_frame over glyphs.
            let mut software = request_capture(false);
            let err = software.advance_frame(16).unwrap_err().to_string();
            assert!(
                err.contains("needs the font backend"),
                "{label} software error: {err}"
            );

            // Host capture (`host_capture == true`) defers capture in advance_frame and accepts a matching frame.
            let mut supplied = request_capture(true);
            supplied.advance_frame(16).unwrap();
            assert!(supplied.capture_due() && supplied.capture().requested());
            if label == "visible" {
                assert!(
                    supplied
                        .messages
                        .text(0)
                        .unwrap()
                        .glyphs
                        .iter()
                        .any(|g| g.shown && g.opacity != 0)
                );
            } else {
                assert!(
                    supplied
                        .messages
                        .auxiliary_text(0)
                        .unwrap()
                        .attached_glyphs()
                        .next()
                        .is_some()
                );
            }
            let [width, height] = supplied.screen_size();
            let pixels = [4, 5, 6, 0].repeat((width * height) as usize);
            let frame = crate::raster::RgbaImage::new(width, height, pixels).unwrap();
            supplied.supply_capture(frame).unwrap();
            assert!(!supplied.capture_due() && !supplied.capture().requested());
            assert_eq!(
                &supplied.capture().image().unwrap().pixels()[..4],
                &[4, 5, 6, 255]
            );
            let mut backend = RecordingBackend::default();
            assert_eq!(supplied.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
            assert_eq!(supplied.machine.state().system_integer(0), Some(1));

            // Leaving capture_due unsupplied falls back to software draw on the next tick and errors.
            let mut unsupplied = request_capture(true);
            unsupplied.advance_frame(16).unwrap();
            assert!(unsupplied.capture_due());
            let next_err = unsupplied.tick(&mut backend).unwrap_err().to_string();
            assert!(
                next_err.contains("needs the font backend"),
                "{label} unsupplied tick error: {next_err}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires read-only CMVS_GAME_ROOT Amatsutsumi sample; writes only a fresh temporary user directory"]
    fn external_reset_reenters_title_and_quit_terminates() {
        let root =
            std::path::PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
        let registry = Registry::builtin();
        let game = registry.identify_game(&root, None).unwrap();
        assert_eq!(game.selection.profile.id, "cmvs-20160201");
        let script = crate::runtime::load_start(&root, &game.files).unwrap();
        let user =
            std::env::temp_dir().join(format!("cmvs-lifecycle-external-{}", std::process::id()));
        let mut run = Runtime::new(&script, &root, &game, 0).unwrap();
        run.set_user_data_directory(&user).unwrap();
        run.set_skip_auto_resume(true).unwrap();
        run.set_record_text(true);
        let mut backend = RecordingBackend::default();
        fn title(run: &mut Runtime, backend: &mut RecordingBackend, after: u64) {
            for _ in 0..3_000_000 {
                match run.tick(backend).unwrap() {
                    RuntimeEvent::Yield => {
                        run.advance_frame(16).unwrap();
                        if run.machine.current_name() == b"menu.ps3" && run.steps > after + 200_000
                        {
                            return;
                        }
                    }
                    RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured => {}
                    other => panic!("{other:?}"),
                }
            }
            panic!("title budget exhausted");
        }
        title(&mut run, &mut backend, 0);
        let names = run.machine.capture_vm().unwrap().script_names;
        let common = names
            .iter()
            .position(|name| name == b"intproc.ps3")
            .unwrap() as u8;
        let settings = run.settings.clone();
        let callbacks = run.control.callbacks().clone();
        // Execute the original opcodes at hash-bound intproc boundaries. This bypasses
        // the confirmation UI and tests engine lifecycle plus real-script continuation.
        run.machine
            .dispatch_callbacks(&[(common, 0x41328)])
            .unwrap();
        run.resume_run = false;
        let before = run.steps;
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        assert_eq!(run.machine.current_name(), b"main.ps3");
        assert_eq!(run.machine.stack().len_bytes(), 0);
        assert_eq!(run.held_snapshot(), &SaveImage::Absent);
        assert!(run.scene.objects().is_empty());
        assert_eq!(run.settings, settings);
        for (index, callback) in callbacks.iter().filter(|(_, c)| c.script_slot != 0) {
            assert_eq!(run.control.callbacks().get(index), Some(callback));
        }
        title(&mut run, &mut backend, before);
        run.machine
            .dispatch_callbacks(&[(common, 0x41800)])
            .unwrap();
        run.resume_run = false;
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Quit);
        let before = run.steps;
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Quit);
        assert_eq!(run.steps, before);
        println!(
            "real RESET returned to title; QUIT terminal at {} steps",
            run.steps
        );
        drop(run);
        std::fs::remove_dir_all(user).unwrap();
    }

    #[test]
    fn reset_clears_transient_hosts_and_preserves_settings_and_fonts() {
        // Synthetic archive fixture contains fixture.ps3, not original game data.
        let root = std::env::temp_dir().join(format!("cmvs-reset-hosts-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/pack")).unwrap();
        std::fs::create_dir_all(root.join("save")).unwrap();
        let archive: Vec<_> = include_str!("../../tests/fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        std::fs::write(root.join("data/pack/script.cpz"), archive).unwrap();
        let mut code = Vec::new();
        for (op, args) in [
            (0x22cbu16, vec![0x123456u32]),
            (0x2250, vec![1]),
            (0x22eb, vec![1, 1]),
            (0x22e5, vec![1, 1]),
            (0x22cd, vec![0]),
        ] {
            for arg in args.into_iter().rev() {
                code.extend(0x440u16.to_le_bytes());
                code.extend(4u16.to_le_bytes());
                code.extend(arg.to_le_bytes());
            }
            code.extend(op.to_le_bytes());
        }
        let reset_pc = code.len();
        code.extend(0x2003u16.to_le_bytes());
        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        std::fs::write(root.join("data/pack/start.ps3"), &raw).unwrap();
        let registry = Registry::builtin();
        let game = registry
            .identify_game(&root, Some("cmvs-20160201"))
            .unwrap();
        let script = Script::decode(&raw).unwrap();
        let mut run = Runtime::new(&script, &root, &game, 0).unwrap();
        run.machine.state_mut().set_integer(7, 42).unwrap();
        let mut backend = RecordingBackend::default();
        while run.machine.pc() != reset_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        assert!(run.message_flags.skip());
        assert!(run.message_flags.auto_advance());
        assert!(!run.save_menu.popup_enabled());
        assert_eq!(run.scene.clear().color, 0x123456);
        run.save_image = SaveImage::Unavailable {
            script: b"old.ps3".to_vec(),
            pc: 12,
        };
        run.thumbnail = Some(vec![1, 2, 3]);
        run.frame_delta = 16;
        run.timers.begin_run(16);
        run.messages.set_record_output(true);
        run.messages.set_default_face(b"Retained Face");
        run.scene.particle_effects.insert(
            0,
            crate::effect_objects::ParticleEffect {
                template: None,
                x87_arithmetic: false,
                mode: 0,
                parameters: [0; 4],
                pool: Default::default(),
                particles: Vec::new(),
            },
        );
        assert!(
            run.inspect_snapshot()
                .unwrap_err()
                .to_string()
                .contains("active particle effects")
        );
        assert!(
            run.load_slot(0, &mut backend)
                .unwrap_err()
                .to_string()
                .contains("active particle effects")
        );
        run.scene.particle_effects.clear();
        for kind in 0..3 {
            run.scene.falling_effects.clear();
            run.scene.collision_effects.clear();
            if kind == 2 {
                run.scene.fragment_effects.insert(
                    3,
                    crate::effect_objects::FragmentEffect {
                        parameters: [20, 1000],
                        elapsed: 0,
                        complete: false,
                        pool: Default::default(),
                        fragments: Vec::new(),
                    },
                );
            } else if kind == 1 {
                run.scene.collision_effects.insert(
                    2,
                    crate::effect_objects::CollisionEffect::new(
                        0,
                        [0; 2],
                        0,
                        Default::default(),
                        &mut crate::random::CrtRandom::new(0),
                    ),
                );
            } else {
                run.scene.falling_effects.insert(
                    1,
                    crate::effect_objects::FallingEffect {
                        template: None,
                        parameters: [0; 4],
                        follow: true,
                        image: None,
                        particles: Vec::new(),
                        x87_arithmetic: false,
                    },
                );
            }
            if kind != 2 {
                assert!(
                    run.inspect_snapshot()
                        .unwrap_err()
                        .to_string()
                        .contains("active particle effects")
                );
                assert!(
                    run.load_slot(0, &mut backend)
                        .unwrap_err()
                        .to_string()
                        .contains("active particle effects")
                );
            }
        }
        run.scene.collision_effects.insert(
            2,
            crate::effect_objects::CollisionEffect::new(
                0,
                [0; 2],
                0,
                Default::default(),
                &mut crate::random::CrtRandom::new(0),
            ),
        );
        run.scene.falling_effects.insert(
            1,
            crate::effect_objects::FallingEffect {
                template: None,
                parameters: [0; 4],
                follow: true,
                image: None,
                particles: Vec::new(),
                x87_arithmetic: false,
            },
        );
        let settings = run.settings.clone();
        let audio = run.audio.settings();
        let gesture = run.gesture.clone();
        run.machine.step().unwrap();
        let prepared = Machine::prepare_main(b"main.ps3".to_vec(), script.clone()).unwrap();
        run.finish_reset(prepared);
        assert_eq!(run.machine.current_name(), b"main.ps3");
        assert_eq!(run.machine.pc(), 0);
        assert_eq!(run.machine.state().integer(7), Some(42));
        assert!(matches!(run.save_image, SaveImage::Absent));
        assert!(run.thumbnail.is_none());
        assert_eq!(run.frame_delta, 0);
        assert_eq!(run.timers.delta_ms(), 0);
        assert_eq!(run.timers.values(), &[16; 10]);
        assert_eq!(run.settings, settings);
        assert_eq!(run.audio.settings(), audio);
        assert_eq!(run.gesture, gesture);
        assert_eq!(&*run.messages.default_face(), b"Retained Face");
        assert_eq!(run.scene.clear(), crate::scene::ClearState::default());
        assert!(run.scene.particle_effects().is_empty());
        assert!(run.scene.falling_effects().is_empty());
        assert!(run.scene.collision_effects().is_empty());
        assert!(run.scene.fragment_effects().is_empty());
        assert!(!run.message_flags.skip());
        assert!(!run.message_flags.auto_advance());
        assert!(run.save_menu.popup_enabled());
        drop(run);
        drop(game);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_profile_operations_use_profile_fs_not_runtime_fs() {
        use crate::{
            filesystem::{File, HostFs, Metadata, OpenOptions, StdFs},
            runtime::{RuntimeOptions, SaveStorage},
            save_menu::SaveSource,
            snapshot::CapturedSave,
            system_save::{ProfileSystemSave, SystemSaveSource},
        };
        use std::{
            io,
            path::{Path, PathBuf},
            sync::Arc,
        };

        struct BlockPrefixFs(PathBuf);

        impl BlockPrefixFs {
            fn guard(&self, path: &Path) -> io::Result<()> {
                if path.starts_with(&self.0) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("runtime fs accessed profile path {}", path.display()),
                    ));
                }
                Ok(())
            }
        }

        impl HostFs for BlockPrefixFs {
            fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
                self.guard(path)?;
                StdFs.open(path, options)
            }
            fn metadata(&self, path: &Path) -> io::Result<Metadata> {
                self.guard(path)?;
                StdFs.metadata(path)
            }
            fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata> {
                self.guard(path)?;
                StdFs.symlink_metadata(path)
            }
            fn try_exists(&self, path: &Path) -> io::Result<bool> {
                self.guard(path)?;
                StdFs.try_exists(path)
            }
            fn is_file(&self, path: &Path) -> bool {
                !path.starts_with(&self.0) && StdFs.is_file(path)
            }
            fn is_dir(&self, path: &Path) -> bool {
                !path.starts_with(&self.0) && StdFs.is_dir(path)
            }
            fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
                self.guard(path)?;
                StdFs.canonicalize(path)
            }
            fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
                self.guard(path)?;
                StdFs.absolute(path)
            }
            fn create_dir_all(&self, path: &Path) -> io::Result<()> {
                self.guard(path)?;
                StdFs.create_dir_all(path)
            }
            fn remove_file(&self, path: &Path) -> io::Result<()> {
                self.guard(path)?;
                StdFs.remove_file(path)
            }
            fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
                self.guard(from)?;
                self.guard(to)?;
                StdFs.rename(from, to)
            }
            fn session_id(&self) -> u32 {
                StdFs.session_id()
            }
        }

        let (root, _) = capture_fixture("profile-fs-isolation");
        let mut code = Vec::new();
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(1u32.to_le_bytes());
        code.extend(0x201du16.to_le_bytes());
        code.extend(0x2003u16.to_le_bytes());
        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        std::fs::write(root.join("data/pack/start.ps3"), &raw).unwrap();

        let user =
            std::env::temp_dir().join(format!("cmvs-profile-fs-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let registry = Registry::builtin();
        let game = registry
            .identify_game(&root, Some("cmvs-20160201"))
            .unwrap();
        let script = Script::decode(&raw).unwrap();

        let mut header = [0u8; crate::save::HEADER_SIZE];
        header[..4].copy_from_slice(b"CSV2");
        header[4..8].copy_from_slice(&(crate::save::HEADER_SIZE as u32).to_le_bytes());
        header[548..550].copy_from_slice(&0u16.to_le_bytes());

        // Phase 1: seed user/system.dat and user/save000.dat via a normal session.
        {
            let profile = ProfileSystemSave::open_with_fs(&user, &root, Arc::new(StdFs)).unwrap();
            let mut seed = Runtime::with_options(
                &script,
                &root,
                &game,
                RuntimeOptions {
                    saves: SaveStorage::Profile(profile),
                    ..Default::default()
                },
            )
            .unwrap();
            let mut backend = RecordingBackend::default();
            assert_eq!(seed.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
            assert_eq!(seed.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
            seed.save_image = SaveImage::Captured(Box::new(CapturedSave {
                header,
                snapshot: vec![1, 2, 3, 4],
                script_image: vec![1],
            }));
            assert!(seed.save_slot(0).unwrap());
        }

        // Phase 2: reinstall the profile onto a Runtime whose own fs forbids profile paths.
        let profile = ProfileSystemSave::open_with_fs(&user, &root, Arc::new(StdFs)).unwrap();
        let user_canon = profile.directory().to_path_buf();

        let mut run = Runtime::with_options(
            &script,
            &root,
            &game,
            RuntimeOptions {
                saves: SaveStorage::Profile(profile),
                fs: Some(Arc::new(BlockPrefixFs(user_canon))),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(run.system_source.read().unwrap().is_some());
        assert!(run.saves.read_header(0).unwrap().is_some());

        let mut backend = RecordingBackend::default();
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        run.save_image = SaveImage::Captured(Box::new(CapturedSave {
            header,
            snapshot: vec![5, 6, 7, 8],
            script_image: vec![1],
        }));
        assert!(run.save_slot(0).unwrap());
        assert!(run.saves.read_slot(0).unwrap().is_some());
        assert!(run.saves.read_slot(999).unwrap().is_some());
        assert!(run.system_source.read().unwrap().is_some());

        drop(run);
        drop(game);
        let _ = std::fs::remove_dir_all(&user);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cvns_runtime_snapshot_save_load_slot_info_and_persistent_preservation() {
        let root = std::env::temp_dir().join(format!("cmvs-cvns-save-{}", std::process::id()));
        let user = std::env::temp_dir().join(format!("cmvs-cvns-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(root.join("data/pack")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let archive: Vec<_> = include_str!("../../tests/fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        std::fs::write(root.join("data/pack/script.cpz"), archive).unwrap();

        let mut code = Vec::new();
        let push_int = |code: &mut Vec<u8>, v: u32| {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(v.to_le_bytes());
        };
        let call = |code: &mut Vec<u8>, op: u16, args: &[u32]| {
            for &arg in args.iter().rev() {
                push_int(code, arg);
            }
            code.extend(op.to_le_bytes());
        };
        call(&mut code, 0x2026, &[0, 0x8000_0000 | 7]);
        call(&mut code, 0x210b, &[16]);
        call(&mut code, 0x210c, &[8]);
        call(&mut code, 0x210d, &[3]);
        call(&mut code, 0x2178, &[0x8000_0000 | 6]); // base window title +340
        call(&mut code, 0x21a6, &[2, 2]); // 0x22b0 thumbnail 2x2
        call(&mut code, 0x200a, &[0x223344]);
        call(&mut code, 0x21a0, &[]); // 0x22b2 capture snapshot
        let after_capture_pc = code.len();
        call(&mut code, 0x21a1, &[1]);
        let after_menu_pc = code.len();
        call(&mut code, 0x21ae, &[0]); // slot 0 (save000.dat) -> 0
        let after_exists0_pc = code.len();
        call(&mut code, 0x21ae, &[1]); // slot 1 (CSP1) -> 1
        let after_exists1_pc = code.len();
        call(&mut code, 0x21a7, &[1, 4]); // LatestSlot 1..4 -> 1
        let after_latest_pc = code.len();
        call(&mut code, 0x21ac, &[1, 5, 0x8000_0000 | 4, 0x8000_0000 | 5]);
        let after_info_pc = code.len();
        call(&mut code, 0x2003, &[]);

        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        std::fs::write(root.join("data/pack/start.ps3"), &raw).unwrap();

        let registry = Registry::builtin();
        let game = registry
            .identify_game(&root, Some("cvns-20090203"))
            .unwrap();
        let script = Script::decode(&raw).unwrap();
        let mut run = Runtime::with_options(
            &script,
            &root,
            &game,
            crate::runtime::RuntimeOptions {
                saves: crate::runtime::SaveStorage::UserDirectory(user.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        run.machine.state_mut().set_integer(2, 123).unwrap();
        run.machine.state_mut().set_integer(8, 777).unwrap();
        run.machine.state_mut().set_string(1, b"local1").unwrap();
        run.machine.state_mut().set_string(3, b"persist3").unwrap();
        run.machine.state_mut().set_string(6, b"Miharu").unwrap();
        run.machine
            .state_mut()
            .set_string(7, b"data\\Saves\\")
            .unwrap();
        run.save_title.restore(b"Chapter".to_vec());
        run.save_info.restore([101, 202, 303]);
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&70u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 24]);
        bmp.extend_from_slice(&[10, 20, 30, 40, 50, 60, 0, 0]);
        bmp.extend_from_slice(&[70, 80, 90, 100, 110, 120, 0, 0]);
        run.thumbnail = Some(bmp);

        let mut backend = RecordingBackend::default();
        while run.machine.pc() != after_capture_pc {
            let ev = run.tick(&mut backend).unwrap();
            assert!(matches!(
                ev,
                RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured
            ));
        }
        assert!(run.save_slot(1).unwrap());
        while run.machine.pc() != after_menu_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        assert_eq!(run.save_menu.menu().unwrap().latest, Some(0));

        let system = std::fs::read(user.join("save000.dat")).unwrap();
        assert!(system.starts_with(b"CSS0"));
        assert!(!run.save_slot(0).unwrap());
        assert_eq!(std::fs::read(user.join("save000.dat")).unwrap(), system);
        assert!(!user.join("save999.dat").exists());

        run.save_info = Default::default();
        while run.machine.pc() != after_exists0_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        assert_eq!(run.machine.state().system_integer(0), Some(0));
        while run.machine.pc() != after_exists1_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        assert_eq!(run.machine.state().system_integer(0), Some(1));
        while run.machine.pc() != after_latest_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        assert_eq!(run.machine.state().system_integer(0), Some(1));
        while run.machine.pc() != after_info_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
        }
        let state = run.machine.state();
        assert_eq!(state.system_integer(0), Some(1));
        assert_eq!(state.system_integer(4), Some(101));
        assert_eq!(state.system_integer(5), Some(202));
        assert_eq!(state.system_integer(6), Some(303));
        assert!(state.string(4).unwrap().ends_with(b" Chapter"));
        assert!(run.scene.cvns().unwrap().images[5].is_some());

        run.machine.state_mut().set_integer(2, 0).unwrap();
        run.machine.state_mut().set_integer(8, 888).unwrap();
        run.machine.state_mut().set_string(1, b"changed").unwrap();
        run.machine
            .state_mut()
            .set_string(3, b"persist_new")
            .unwrap();
        run.load_slot(1, &mut backend).unwrap();
        assert_eq!(run.machine.pc(), after_capture_pc);
        assert_eq!(run.machine.state().integer(2), Some(123));
        assert_eq!(run.machine.state().integer(8), Some(888));
        assert_eq!(run.machine.state().string(1), Some(b"local1".as_slice()));
        assert_eq!(
            run.machine.state().string(3),
            Some(b"persist_new".as_slice())
        );
        assert_eq!(run.save_info.values(), [Some(101), Some(202), Some(303)]);

        let registered = root.join("data/Saves");
        std::fs::create_dir_all(&registered).unwrap();
        std::fs::copy(user.join("save001.dat"), registered.join("save001.dat")).unwrap();
        let mut read_only = Runtime::new(&script, &root, &game, 0).unwrap();
        read_only
            .machine
            .state_mut()
            .set_string(7, b"data\\Saves\\")
            .unwrap();
        while read_only.machine.pc() != after_menu_pc {
            let event = read_only.tick(&mut backend).unwrap();
            assert!(matches!(
                event,
                RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured
            ));
        }
        assert_eq!(read_only.save_menu.menu().unwrap().latest, Some(0));
        read_only.load_slot(1, &mut backend).unwrap();
        assert_eq!(read_only.machine.state().integer(2), Some(123));
        assert!(!registered.join("save998.dat").exists());
        drop(run);
        let _ = std::fs::remove_dir_all(user);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cvns_hosts_and_finish_reset_enable_cvns_from_profile() {
        let registry = Registry::builtin();
        let cvns = registry.profile("cvns-20090203").unwrap();
        let cmvs = registry.profile("cmvs-20160201").unwrap();
        assert!(crate::scene::SceneHost::for_profile(cvns).cvns().is_some());
        assert!(crate::scene::SceneHost::for_profile(cmvs).cvns().is_none());
        assert_eq!(
            crate::audio::AudioHost::for_profile(cvns, &[]).se_channel_count(),
            crate::audio::CVNS_SE_CHANNELS
        );
        assert_eq!(
            crate::audio::AudioHost::for_profile(cmvs, &[]).se_channel_count(),
            crate::audio::SE_CHANNELS
        );
        assert!(
            crate::system_save::SystemSaveHost::for_profile(cvns)
                .css0_starts
                .is_some()
        );
        assert!(
            crate::system_save::SystemSaveHost::for_profile(cmvs)
                .css0_starts
                .is_none()
        );

        let (root, raw) = capture_fixture("cvns-finish-reset");
        let game = registry
            .identify_game(&root, Some("cvns-20090203"))
            .unwrap();
        let script = Script::decode(&raw).unwrap();
        let mut run = Runtime::new(&script, &root, &game, 0).unwrap();
        assert!(run.scene.cvns().is_some());
        assert!(run.messages.cvns().is_some());
        assert_eq!(run.audio.se_channel_count(), crate::audio::CVNS_SE_CHANNELS);

        let replacement = Machine::prepare_main(b"main.pss".to_vec(), script.clone()).unwrap();
        run.finish_reset(replacement);
        assert!(run.scene.cvns().is_some());
        assert!(run.messages.cvns().is_some());
        assert_eq!(run.audio.se_channel_count(), crate::audio::CVNS_SE_CHANNELS);
        let _ = std::fs::remove_dir_all(root);
    }
}

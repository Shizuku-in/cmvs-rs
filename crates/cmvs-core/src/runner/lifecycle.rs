//! QUIT and RESET for the hash-bound cmvs-20160201 layout.
use super::{Startup, StartupEvent};
use crate::{dispatch::RuntimeCall, script_control::ScriptSource};
use std::error::Error;

impl<'a> Startup<'a> {
    pub(super) fn handle_lifecycle(
        &mut self,
        request: &RuntimeCall,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        let command = request.command();
        // Load/validate the replacement before destructive cleanup. Failure leaves RESET
        // pending, with the old VM and hosts intact; no game file is ever written.
        let replacement = if command == 0x03 {
            let script = self.source.load(b"main.ps3")?;
            Some(crate::machine::Machine::prepare_main(
                b"main.ps3".to_vec(),
                script,
            )?)
        } else {
            None
        };
        if self.movies.player().is_some() {
            self.audio_backend.stop_movie_audio()?;
        }
        self.audio
            .stop_for_lifecycle(self.audio_backend.as_mut(), command == 0x03)?;
        if command == 0x01 {
            request.complete(&mut self.machine, 0x4000)?;
            self.movies.restore(Default::default());
            self.quit = true;
            return Ok(StartupEvent::Quit);
        }
        self.finish_reset(replacement.expect("RESET replacement prepared"));
        Ok(StartupEvent::Progress)
    }

    fn finish_reset(&mut self, replacement: crate::machine::PreparedMain<'a>) {
        self.machine.replace_main(replacement);
        self.control.main_replaced();
        self.save_image = Default::default();
        self.thumbnail = None;
        self.scene = Default::default();
        self.camera.reset();
        self.buttons = Default::default();
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

    #[test]
    #[ignore = "requires read-only CMVS_GAME_ROOT Amatsutsumi sample; writes only a fresh temporary user directory"]
    fn external_reset_reenters_title_and_quit_terminates() {
        let root =
            std::path::PathBuf::from(std::env::var_os("CMVS_GAME_ROOT").expect("CMVS_GAME_ROOT"));
        let registry = Registry::builtin();
        let game = registry.identify_game(&root, None).unwrap();
        assert_eq!(game.selection.profile.id, "cmvs-20160201");
        let script = crate::runner::load_start(&root, &game.files).unwrap();
        let user =
            std::env::temp_dir().join(format!("cmvs-lifecycle-external-{}", std::process::id()));
        let mut run = Startup::new(&script, &root, &game, 0).unwrap();
        run.set_user_data_directory(&user).unwrap();
        run.set_skip_auto_resume(true).unwrap();
        run.set_record_text(true);
        let mut backend = RecordingBackend::default();
        fn title(run: &mut Startup<'_>, backend: &mut RecordingBackend, after: u64) {
            for _ in 0..3_000_000 {
                match run.tick(backend).unwrap() {
                    StartupEvent::Yield => {
                        run.advance_frame(16).unwrap();
                        if run.machine.current_name() == b"menu.ps3" && run.steps > after + 200_000
                        {
                            return;
                        }
                    }
                    StartupEvent::Progress | StartupEvent::SnapshotCaptured => {}
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
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
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
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Quit);
        let before = run.steps;
        assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Quit);
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
        let archive: Vec<_> = include_str!("../../../../tests/fixtures/cpz6-script.hex")
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
        let mut run = Startup::new(&script, &root, &game, 0).unwrap();
        run.machine.state_mut().set_integer(7, 42).unwrap();
        let mut backend = RecordingBackend::default();
        while run.machine.pc() != reset_pc {
            assert_eq!(run.tick(&mut backend).unwrap(), StartupEvent::Progress);
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
        assert!(!run.message_flags.skip());
        assert!(!run.message_flags.auto_advance());
        assert!(run.save_menu.popup_enabled());
        drop(run);
        drop(game);
        std::fs::remove_dir_all(root).unwrap();
    }
}

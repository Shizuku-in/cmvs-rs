use cmvs_rs::{
    dispatch::ownership::{self, CLAIMS, Chain},
    machine::Machine,
};
mod support;
use support::bytecode::{command, pending, script};

struct Unavailable;
impl cmvs_rs::image::ImageSource for Unavailable {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        Err("unavailable".into())
    }
}
impl cmvs_rs::resources::ResourceSource for Unavailable {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        Err("unavailable".into())
    }
}
impl cmvs_rs::platform::PlatformAssets for Unavailable {
    fn read(&mut self, _: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Err("unavailable".into())
    }
}
impl cmvs_rs::script_control::ScriptSource for Unavailable {
    fn load(&mut self, _: &[u8]) -> Result<cmvs_rs::ps3::Script, Box<dyn std::error::Error>> {
        Err("unavailable".into())
    }
}
impl cmvs_rs::system_save::SystemSaveSource for Unavailable {
    fn read(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
}
impl cmvs_rs::system_save::SystemSaveSink for Unavailable {
    fn write(&mut self, _: &[u8]) -> std::io::Result<()> {
        panic!("routing audit must not write")
    }
}
impl cmvs_rs::save_menu::SaveSource for Unavailable {
    fn read_header(&mut self, _: u16) -> std::io::Result<Option<Vec<u8>>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
}
impl cmvs_rs::movie::MovieSource for Unavailable {
    fn open(
        &mut self,
        _: &[u8],
    ) -> Result<Option<cmvs_rs::movie::OpenedMovie>, Box<dyn std::error::Error>> {
        Err("unavailable".into())
    }
}

/// Empty arguments distinguish refusal from recognition: recognized commands
/// either finish without arguments or report a stack/target/capability error.
/// This intentionally verifies routing, not the command's execution semantics.
fn audit(handler: &str, mut probe: impl FnMut(&mut Machine<'_>) -> bool) {
    let recognizes = |number| {
        use cmvs_rs::dispatch::Capability;
        let capability = match handler {
            "scene::try_handle_image" => Some(Capability::SceneImages),
            "scene::try_handle_mask" => Some(Capability::MaskImages),
            "scene::try_change_image" => Some(Capability::ImageChange),
            "audio::try_handle" => Some(Capability::Audio),
            "timers::try_handle" => Some(Capability::Timers),
            "message_flags::try_handle" => Some(Capability::Flags),
            "random::try_handle" => Some(Capability::Random),
            "settings::try_handle" => Some(Capability::Settings),
            "history::try_handle" => Some(Capability::History),
            "text::try_handle" => Some(Capability::Strings),
            "math::try_handle" => Some(Capability::Math),
            "gesture::try_handle" => Some(Capability::Gesture),
            "save_menu::try_handle_popup" => Some(Capability::Popup),
            "snapshot::SaveTitle::try_handle" => Some(Capability::SaveTitle),
            "snapshot::SaveInfo::try_handle" => Some(Capability::SaveInfo),
            "fade::try_handle_effects" => Some(Capability::Effects),
            "effect_objects::try_handle" => Some(Capability::EffectObjects),
            "fade::try_wait" => Some(Capability::FadeWait),
            "scene::try_wait_mask" => Some(Capability::MaskWait),
            "scene::try_wait_quake" => Some(Capability::QuakeWait),
            "scene::try_wait_change" => Some(Capability::ChangeWait),
            "scene::try_wait_strip" => Some(Capability::StripWait),
            "camera::try_wait_move" => Some(Capability::CameraWait),
            _ => None,
        };
        if let Some(capability) = capability {
            cmvs_rs::dispatch::ROOT
                .iter()
                .any(|entry| entry.command == number && entry.capability == capability)
        } else {
            CLAIMS
                .iter()
                .find(|claim| claim.handler == handler)
                .unwrap()
                .recognizes(number)
        }
    };
    // Command zero yields inside Machine and never reaches either host chain.
    assert!(!recognizes(0));
    for number in 1..0x400 {
        let mut code = vec![];
        command(&mut code, number + 0x2000, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        assert_eq!(
            probe(&mut vm),
            recognizes(number),
            "{handler} 0x{:04x}",
            number + 0x2000
        );
    }
}

#[test]
fn inventory_matches_stateless_and_default_host_recognition_exhaustively() {
    use cmvs_rs::{
        audio::{AudioHost, RecordingAudio},
        bootstrap::Bootstrap,
        buttons::ButtonHost,
        camera::CameraHost,
        capture::ScreenCapture,
        gesture::PointerGesture,
        history::HistoryHost,
        input::{self, ActionInput},
        message::MessageHost,
        message_flags::MessageFlags,
        movie::MovieHost,
        random::CrtRandom,
        scene::SceneHost,
        sequence::SequenceHost,
        settings::SettingsHost,
        snapshot::{SaveInfo, SaveTitle},
        timers::VmTimers,
    };
    audit("bootstrap::try_handle", |vm| {
        Bootstrap::default()
            .try_handle(vm)
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("scene::try_handle", |vm| {
        SceneHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("scene::try_handle_quake", |vm| {
        SceneHost::default().try_handle_quake(vm).unwrap_or(true)
    });
    audit("scene::try_handle_strip", |vm| {
        SceneHost::default().try_handle_strip(vm).unwrap_or(true)
    });
    audit("gesture::try_handle", |vm| {
        PointerGesture::default().try_handle(vm).unwrap_or(true)
    });
    audit("gesture::try_handle_drag_press", |vm| {
        PointerGesture::default()
            .try_handle_drag_press(vm, &ActionInput::default(), 0)
            .unwrap_or(true)
    });
    audit("input::try_handle", |vm| {
        input::try_handle(vm, &mut ActionInput::default()).unwrap_or(true)
    });
    audit("input::try_handle_skipping", |vm| {
        input::try_handle_skipping(vm, &ActionInput::default(), false).unwrap_or(true)
    });
    audit("camera::try_handle", |vm| {
        CameraHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("history::try_handle", |vm| {
        HistoryHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("movie::try_handle", |vm| {
        MovieHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("settings::try_handle", |vm| {
        SettingsHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("message_flags::try_handle", |vm| {
        MessageFlags::default().try_handle(vm).unwrap_or(true)
    });
    audit("timers::try_handle", |vm| {
        VmTimers::default().try_handle(vm).unwrap_or(true)
    });
    audit("random::try_handle", |vm| {
        CrtRandom::new(0).try_handle(vm).unwrap_or(true)
    });
    audit("sequence::try_handle_with_hosts", |vm| {
        SequenceHost::default().try_handle(vm).unwrap_or(true)
    });
    audit("capture::try_handle", |vm| {
        ScreenCapture::default()
            .try_handle(vm, &mut SceneHost::default())
            .unwrap_or(true)
    });
    audit("snapshot::SaveInfo::try_handle", |vm| {
        SaveInfo::default().try_handle(vm).unwrap_or(true)
    });
    audit("snapshot::SaveTitle::try_handle", |vm| {
        SaveTitle::default().try_handle(vm).unwrap_or(true)
    });
    audit("buttons::try_handle", |vm| {
        ButtonHost::default()
            .try_handle(vm, &mut SceneHost::default())
            .unwrap_or(true)
    });
    audit("buttons::try_handle_input", |vm| {
        ButtonHost::default()
            .try_handle_input(vm, &mut SceneHost::default(), &mut ActionInput::default())
            .unwrap_or(true)
    });
    audit("buttons::try_handle_list_input", |vm| {
        ButtonHost::default()
            .try_handle_list_input(
                vm,
                &mut SceneHost::default(),
                &mut ActionInput::default(),
                &PointerGesture::default(),
            )
            .unwrap_or(true)
    });
    audit("audio::try_handle", |vm| {
        AudioHost::default()
            .try_handle(vm, &mut RecordingAudio::default())
            .unwrap_or(true)
    });
    audit("math::try_handle", |vm| {
        cmvs_rs::math::try_handle(vm).unwrap_or(true)
    });
    audit("text::try_handle", |vm| {
        cmvs_rs::text::try_handle(vm).unwrap_or(true)
    });
    audit("effect_objects::try_handle", |vm| {
        cmvs_rs::effect_objects::try_handle(vm).unwrap_or(true)
    });
    audit("fade::try_handle_effects", |vm| {
        cmvs_rs::fade::try_handle_effects(vm, &mut true).unwrap_or(true)
    });
    audit("message::try_handle", |vm| {
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        messages.try_handle(vm).unwrap_or(true)
    });
    audit("message::try_handle_scene", |vm| {
        MessageHost::default()
            .try_handle_scene(vm, &mut SceneHost::default())
            .unwrap_or(true)
    });
    audit("message::try_handle_pointer", |vm| {
        MessageHost::default()
            .try_handle_pointer(vm, &ActionInput::default())
            .unwrap_or(true)
    });
    audit("message::try_handle_buttons", |vm| {
        MessageHost::default()
            .try_handle_buttons(vm, &mut ActionInput::default())
            .unwrap_or(true)
    });
}

#[test]
fn inventory_matches_wait_and_resource_host_recognition_exhaustively() {
    use cmvs_rs::{
        audio::{AudioSettings, RecordingAudio},
        camera::CameraHost,
        capture::ScreenCapture,
        fade::SkipGate,
        input::ActionInput,
        message::MessageHost,
        movie::MovieHost,
        platform::{PlatformHost, RecordingBackend},
        random::CrtRandom,
        save_menu::SaveMenuHost,
        scene::SceneHost,
        script_control::ScriptControl,
        snapshot::SaveImage,
        system_save::SystemSaveHost,
    };
    audit("fade::try_wait", |vm| {
        SceneHost::default()
            .fade_mut()
            .try_wait(vm, SkipGate::default(), &mut ActionInput::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("scene::try_wait_mask", |vm| {
        SceneHost::default()
            .try_wait_mask(vm, SkipGate::default(), &mut ActionInput::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("scene::try_wait_quake", |vm| {
        SceneHost::default()
            .try_wait_quake(vm)
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("scene::try_wait_change", |vm| {
        SceneHost::default()
            .try_wait_change(vm, SkipGate::default(), &mut ActionInput::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("scene::try_wait_strip", |vm| {
        SceneHost::default()
            .try_wait_strip(vm, SkipGate::default(), &mut ActionInput::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("camera::try_wait_move", |vm| {
        CameraHost::default()
            .try_wait_move(vm, SkipGate::default(), &mut ActionInput::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("message::try_wait_with_auto", |vm| {
        MessageHost::default()
            .try_wait(vm, &mut ActionInput::default(), Default::default())
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("message::try_record_poll", |vm| {
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        messages
            .try_record_poll(vm, Default::default())
            .unwrap_or(true)
    });
    audit("dispatch::ROOT", |vm| {
        let mut messages = MessageHost::default();
        messages.set_record_output(true);
        cmvs_rs::dispatch::run(
            cmvs_rs::profile::Semantics::Root,
            vm,
            &mut cmvs_rs::dispatch::Hosts {
                waits: Some(cmvs_rs::dispatch::WaitHosts {
                    scene: Some(&mut SceneHost::default()),
                    camera: Some(&mut CameraHost::default()),
                    input: &mut ActionInput::default(),
                    gate: SkipGate::default(),
                    change_images: true,
                }),
                text: Some(cmvs_rs::dispatch::TextHosts {
                    messages: &mut messages,
                    input: &mut ActionInput::default(),
                    flags: Default::default(),
                }),
                timers: Some(&mut cmvs_rs::timers::VmTimers::default()),
                flags: Some(&mut cmvs_rs::message_flags::MessageFlags::default()),
                random: Some(&mut CrtRandom::new(0)),
                settings: Some(&mut cmvs_rs::settings::SettingsHost::default()),
                history: Some(&mut cmvs_rs::history::HistoryHost::default()),
                strings: true,
                math: true,
                gesture: Some(&mut cmvs_rs::gesture::PointerGesture::default()),
                popup: Some(&mut SaveMenuHost::default()),
                save_title: Some(&mut cmvs_rs::snapshot::SaveTitle::default()),
                save_info: Some(&mut cmvs_rs::snapshot::SaveInfo::default()),
                effects: Some(&mut true),
                effect_objects: true,
                images: Some(cmvs_rs::dispatch::ImageHosts {
                    scene: &mut SceneHost::default(),
                    source: &mut Unavailable,
                    change_images: true,
                }),
                audio: Some(cmvs_rs::dispatch::AudioHosts {
                    audio: &mut cmvs_rs::audio::AudioHost::default(),
                    backend: &mut RecordingAudio::default(),
                }),
            },
        )
        .map(|r| r.is_some())
        .unwrap_or(true)
    });
    audit("scene::try_handle_image", |vm| {
        SceneHost::default()
            .try_handle_image(vm, &mut Unavailable)
            .unwrap_or(true)
    });
    audit("scene::try_change_image", |vm| {
        SceneHost::default()
            .try_change_image(vm, &mut Unavailable)
            .unwrap_or(true)
    });
    audit("scene::try_handle_mask", |vm| {
        SceneHost::default()
            .try_handle_mask(vm, &mut Unavailable)
            .unwrap_or(true)
    });
    audit("message::try_handle_image", |vm| {
        MessageHost::default()
            .try_handle_image(vm, &mut Unavailable)
            .unwrap_or(true)
    });
    audit("capture::try_immediate", |vm| {
        ScreenCapture::default()
            .try_immediate(
                vm,
                &mut SceneHost::default(),
                &mut RecordingBackend::default(),
            )
            .unwrap_or(true)
    });
    audit("platform::try_handle", |vm| {
        PlatformHost::default()
            .try_handle(vm, &mut RecordingBackend::default(), &mut Unavailable, &[])
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("script_control::try_handle", |vm| {
        ScriptControl::new(0)
            .try_handle(vm, &mut Unavailable)
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("system_save::try_handle", |vm| {
        SystemSaveHost::default()
            .try_handle(vm, &mut Unavailable, None)
            .map(|r| r.is_some())
            .unwrap_or(true)
    });
    audit("system_save::try_write", |vm| {
        SystemSaveHost::default()
            .try_write(vm, &mut Unavailable, &mut CrtRandom::new(0), None)
            .unwrap_or(true)
    });
    audit("save_menu::try_handle", |vm| {
        SaveMenuHost::default()
            .try_handle(vm, &mut Unavailable)
            .unwrap_or(true)
    });
    audit("save_menu::try_handle_popup", |vm| {
        SaveMenuHost::default().try_handle_popup(vm).unwrap_or(true)
    });
    audit("snapshot::SaveImage::try_handle", |vm| {
        SaveImage::default().try_handle(vm, true).unwrap_or(true)
    });
    audit("movie::try_create_scene", |vm| {
        MovieHost::default()
            .try_create_scene(vm, &mut SceneHost::default(), &mut Unavailable)
            .unwrap_or(true)
    });
    audit("movie::try_start_scene", |vm| {
        MovieHost::default().try_start_scene(vm).unwrap_or(true)
    });
    audit("movie::try_handle_player", |vm| {
        MovieHost::default()
            .try_handle_player(
                vm,
                &mut Unavailable,
                AudioSettings::default(),
                &mut RecordingAudio::default(),
            )
            .unwrap_or(true)
    });
}

#[test]
fn all_overlaps_are_explicit_and_chain_orders_are_unique() {
    use std::collections::BTreeSet;
    assert_eq!(
        CLAIMS
            .iter()
            .map(|c| c.handler)
            .collect::<BTreeSet<_>>()
            .len(),
        CLAIMS.len()
    );
    for chain in [Chain::Runner, Chain::Trace] {
        let orders: Vec<_> = CLAIMS.iter().filter_map(|c| c.order(chain)).collect();
        assert_eq!(orders.iter().collect::<BTreeSet<_>>().len(), orders.len());
        let overlaps: Vec<_> = (0..0x400)
            .filter(|&command| ownership::candidates(command, chain).len() > 1)
            .collect();
        assert_eq!(
            overlaps,
            if chain == Chain::Runner {
                vec![0x10b, 0x154, 0x159, 0x2b2, 0x34d, 0x383, 0x38b]
            } else {
                vec![0x34d]
            }
        );
    }
    assert!(
        CLAIMS
            .iter()
            .all(|claim| !claim.recognizes(0x400) && !claim.recognizes(u16::MAX))
    );
    for (command, expected) in [
        (0x10b, ["dispatch::ROOT", "message::try_handle"]),
        (0x154, ["dispatch::ROOT", "message::try_handle"]),
        (0x159, ["dispatch::ROOT", "message::try_record_poll"]),
        (
            0x2b2,
            [
                "runner::capture_snapshot",
                "snapshot::SaveImage::try_handle",
            ],
        ),
        (0x34d, ["bootstrap::try_handle", "input::try_handle"]),
        (0x383, ["message::try_handle_scene", "message::try_handle"]),
        (0x38b, ["dispatch::ROOT", "message::try_handle"]),
    ] {
        assert_eq!(
            ownership::candidates(command, Chain::Runner)
                .iter()
                .map(|c| c.handler)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn inventory_tracks_chain_entry_calls() {
    use std::collections::BTreeMap;
    // Catch entry additions/removals independently of the command-set probes.
    // Shared method names are counted; receiver identity and state conditions
    // remain part of the source review documented beside the inventory.
    for (chain, source, start) in [
        (
            Chain::Runner,
            include_str!("../crates/cmvs-core/src/runner.rs"),
            "    fn step_once(",
        ),
        (
            Chain::Trace,
            include_str!("../src/bin/trace.rs"),
            "            let resolved = gate.resolve",
        ),
    ] {
        let source = &source[source.find(start).unwrap()..];
        let mut actual = BTreeMap::<_, usize>::new();
        for tail in source.split("try_").skip(1) {
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            *actual.entry(format!("try_{name}")).or_default() += 1;
        }
        let mut expected = BTreeMap::<_, usize>::new();
        for claim in CLAIMS.iter().filter(|c| c.order(chain).is_some()) {
            let name = claim.handler.rsplit("::").next().unwrap();
            if name.starts_with("try_") {
                *expected.entry(name.to_string()).or_default() += 1;
            }
        }
        if chain == Chain::Trace {
            // The sequence entry has a capability-dependent legacy fallback.
            *expected.entry("try_handle".to_string()).or_default() += 1;
        }
        assert_eq!(actual, expected, "{chain:?} entry inventory drifted");
    }
}

#[test]
fn known_priority_difference_is_visible_without_changing_behavior() {
    let names = |chain| {
        ownership::candidates(0x34d, chain)
            .iter()
            .map(|c| c.handler)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(Chain::Runner),
        ["bootstrap::try_handle", "input::try_handle"]
    );
    assert_eq!(
        names(Chain::Trace),
        ["input::try_handle", "bootstrap::try_handle"]
    );
    // Both recognize the opcode, but their effects differ. This is a dispatch
    // migration decision, not a conflict the inventory can silently resolve.
    let mut code = vec![];
    command(&mut code, 0x234d, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut bootstrap = cmvs_rs::bootstrap::Bootstrap::default();
    assert!(bootstrap.try_handle(&mut vm).unwrap().is_some());
    assert_eq!(bootstrap.settings().get(&0x234d), Some(&1));
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut input = cmvs_rs::input::ActionInput::default();
    assert!(cmvs_rs::input::try_handle(&mut vm, &mut input).unwrap());
    input.press_function_keys(1);
    let mut code = vec![];
    command(&mut code, 0x234e, &[]);
    let query = support::bytecode::script(&code);
    let mut vm = Machine::new(&query).unwrap();
    pending(&mut vm);
    assert!(cmvs_rs::input::try_handle(&mut vm, &mut input).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
}

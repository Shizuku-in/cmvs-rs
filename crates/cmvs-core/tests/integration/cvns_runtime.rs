//! CVNS runtime pieces with synthetic data: CSS0, the save-menu scan, the 256-slot string
//! bank and the translated common-script entry.
use crate::support::shared_storage;
use cmvs_core::{
    format::ps3::Script, format::system_save::Css0Save, format::system_save::PersistentStarts,
    runtime::save_menu::SaveMenu, runtime::save_menu::SaveSource, runtime::save_menu::SlotStatus,
    runtime::script_control::ControlEvent, runtime::script_control::LoadError,
    runtime::script_control::ScriptControl, runtime::script_control::ScriptSource,
    vm::cvns::CommandMap, vm::evaluate::State, vm::machine::Machine, vm::machine::Step,
};
use shared_storage::SharedControl;
use std::{collections::BTreeMap, io};

const KEY: &[u8] = b"PURPLE YUKO KAWAI ARC CHECK";

/// Literal-only LZSS (a 0xff flag byte before every eight bytes), then the CSS0 cipher.
fn css0(payload: &[u8], counts: [u32; 2], keys: [u8; 2]) -> Vec<u8> {
    let mut packed = Vec::new();
    for chunk in payload.chunks(8) {
        packed.push(0xff);
        packed.extend(chunk);
    }
    for (i, byte) in packed.iter_mut().enumerate() {
        *byte =
            keys[1].wrapping_add(keys[0].wrapping_add(KEY[i % KEY.len()]) ^ byte.wrapping_add(64));
    }
    let mut raw = vec![0u8; 600];
    raw[..4].copy_from_slice(b"CSS0");
    raw[4..8].copy_from_slice(&600u32.to_le_bytes());
    raw[540..544].copy_from_slice(&(600 + packed.len() as u32).to_le_bytes());
    raw[544..546].copy_from_slice(&1u16.to_le_bytes());
    raw[546] = keys[0];
    raw[548] = keys[1];
    raw[552..556].copy_from_slice(&counts[0].to_le_bytes());
    raw[556..560].copy_from_slice(&counts[1].to_le_bytes());
    raw[560..564].copy_from_slice(&(packed.len() as u32).to_le_bytes());
    raw[580..584].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    let sum = packed
        .iter()
        .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
    raw.extend(&packed);
    raw.extend(sum.to_le_bytes());
    raw
}

/// Persistent tails of one bit byte, two integers and two strings, then one list name.
fn css0_payload() -> (Vec<u8>, PersistentStarts) {
    let mut payload = vec![0b1000_0001];
    payload.extend((-5i32).to_le_bytes());
    payload.extend(7i32.to_le_bytes());
    payload.extend(b"ab\0\0x\0");
    let starts = PersistentStarts {
        bit: 0x7fff * 8,
        integer: 4094,
        string: 254,
    };
    (payload, starts)
}

#[test]
fn offline_startup_stops_at_unmapped_source_command() {
    use crate::support::bytecode::script;
    use cmvs_core::{profile::Registry, runtime::script_control::Startup};
    let profile = Registry::builtin().profile("cvns-20040515").unwrap();
    let startup = Startup::execute(&script(&0x2011u16.to_le_bytes()), profile).unwrap();
    assert_eq!(startup.pc, 0);
    assert!(startup.stopped.contains("0x2011"));
    assert!(startup.bootstrap.paths().is_empty());
}

#[test]
fn css0_restores_the_persistent_tails_and_rejects_inconsistent_files() {
    let (payload, starts) = css0_payload();
    let raw = css0(&payload, [1, 0], [0x5a, 0xc3]);
    let save = Css0Save::decode(&raw, starts).unwrap();
    assert_eq!(save.lists()[0], [b"x".to_vec()]);
    assert!(save.lists()[1].is_empty());
    let mut state = State::default();
    state.set_string_slots(256);
    let state = save.applied(&state).unwrap();
    let first = 0x7fff * 8;
    assert_eq!(state.bit(first), Some(true));
    assert_eq!(state.bit(first + 1), Some(false));
    assert_eq!(state.bit(first + 7), Some(true));
    assert_eq!(state.integer(4094), Some(-5));
    assert_eq!(state.integer(4095), Some(7));
    assert_eq!(state.string(254), Some(&b"ab"[..]));
    assert_eq!(state.string(255), Some(&b""[..]));
    // A 128-slot bank cannot hold the persistent strings.
    assert!(save.applied(&State::default()).is_err());

    let mut bad_sum = raw.clone();
    let end = bad_sum.len() - 2;
    bad_sum[end] ^= 1;
    let mut longer = payload.clone();
    longer.push(0);
    for invalid in [
        bad_sum,
        raw[..raw.len() - 1].to_vec(),
        css0(&longer, [1, 0], [1, 2]),
        css0(&payload, [2, 0], [1, 2]),
        [b"CSS1", &raw[4..]].concat(),
    ] {
        assert!(Css0Save::decode(&invalid, starts).is_err());
    }
    let beyond = PersistentStarts {
        string: 257,
        ..starts
    };
    assert!(Css0Save::decode(&raw, beyond).is_err());
}

#[derive(Default)]
struct MemorySaves {
    files: BTreeMap<u16, Vec<u8>>,
    calls: Vec<u16>,
}
impl SaveSource for MemorySaves {
    fn read_header(&mut self, file: u16) -> io::Result<Option<Vec<u8>>> {
        self.calls.push(file);
        Ok(self.files.get(&file).cloned())
    }
}

fn header(signature: &[u8; 4], label: &[u8]) -> Vec<u8> {
    let mut bytes = signature.to_vec();
    bytes.resize(16, 0);
    bytes.extend(label);
    bytes.push(0);
    bytes
}

#[test]
fn cvns_menu_scan_reads_file_slot_plus_one_and_accepts_only_csp() {
    let mut saves = MemorySaves::default();
    saves
        .files
        .insert(1, header(b"CSP1", b"2009-02-01 10:00:00 a"));
    saves
        .files
        .insert(2, header(b"CSV2", b"2009-02-01 11:00:00 b"));
    saves
        .files
        .insert(3, header(b"CSP9", b"2009-02-01 12:00:00 c"));
    let menu = SaveMenu::scan_cvns(&mut saves, 3).unwrap();
    assert_eq!(saves.calls, [1, 2, 3]);
    assert!(matches!(menu.slots[0], SlotStatus::Recognized(_)));
    assert!(matches!(menu.slots[1], SlotStatus::Invalid(_)));
    assert!(matches!(menu.slots[2], SlotStatus::Recognized(_)));
    assert_eq!(menu.latest, Some(2));
    // Unsigned count: a negative word is clamped to 256, not replaced by 10.
    let mut empty = MemorySaves::default();
    assert_eq!(
        SaveMenu::scan_cvns(&mut empty, u32::MAX)
            .unwrap()
            .slots
            .len(),
        256
    );
    assert_eq!(empty.calls.first(), Some(&1));
}

#[test]
fn string_bank_size_bounds_writes() {
    let mut state = State::default();
    assert_eq!(state.string_slots(), 128);
    assert!(state.set_string(200, b"x").is_err());
    state.set_string_slots(256);
    state.set_string(200, b"x").unwrap();
    assert!(state.fill_strings(250, 7, b"").is_err());
    state.fill_strings(250, 6, b"").unwrap();
}

fn word(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
}
fn script(code: &[u8], text: &[u8]) -> Vec<u8> {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    payload.extend(text);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    raw
}

#[derive(Default)]
struct MemorySource(BTreeMap<Vec<u8>, Vec<u8>>);
impl ScriptSource for MemorySource {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError> {
        Ok(Script::decode(self.0.get(name).ok_or("missing script")?)?)
    }
}

#[test]
fn translated_common_entry_pops_one_word_and_returns_through_0414() {
    // push "c.pss"; 0x2012; 0x2000
    let mut root = vec![];
    word(&mut root, 0x201);
    word(&mut root, 0x120);
    root.extend(0u32.to_le_bytes());
    word(&mut root, 0x20f);
    word(&mut root, 0x430);
    word(&mut root, 0x2012);
    let resume = root.len();
    word(&mut root, 0x2000);
    let root = Script::decode(&script(&root, b"c.pss\0")).unwrap();
    let mut common = vec![];
    word(&mut common, 0x414);
    word(&mut common, 0);
    let mut source = MemorySource::default();
    source.0.insert(b"c.pss".to_vec(), script(&common, &[]));

    let mut vm = Machine::new(&root).unwrap();
    vm.set_command_map(Some(CommandMap::new([(0x12, 0x412)])));
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    assert_eq!(vm.pending_system().map(|(_, c)| c), Some(0x412));
    assert_eq!(vm.pending_source(), Some(0x12));
    let mut host = ScriptControl::new(0);
    assert_eq!(
        host.shared_control(&mut vm, &mut source).unwrap(),
        Some(ControlEvent::CommonEntered { slot: 1 })
    );
    // The name word is gone; repeat count, return PC and caller slot remain.
    assert_eq!(vm.stack().len_bytes(), 12);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 0);
    assert_eq!(vm.stack().peek_u32(8).unwrap(), resume as u32);
    vm.step().unwrap();
    assert_eq!((vm.current_slot(), vm.pc()), (0, resume));
    assert_eq!(vm.stack().len_bytes(), 0);
}

fn push_const(code: &mut Vec<u8>, kind: u16, tag: u16, raw: u32) {
    word(code, kind);
    word(code, tag);
    code.extend(raw.to_le_bytes());
    word(code, 0x20f);
    word(code, 0x430);
}

fn cvns_movie_fixture(tag: &str, audio_flag: u32) -> (crate::support::temp_dir::TempDir, Vec<u8>) {
    let root = crate::support::temp_dir::TempDir::new(&format!("cvns-movie-{tag}"));
    std::fs::create_dir_all(root.join("data/pack")).unwrap();
    std::fs::create_dir_all(root.join("data/video")).unwrap();
    std::fs::create_dir_all(root.join("save")).unwrap();
    let archive: Vec<u8> = include_str!("../fixtures/cpz6-script.hex")
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    std::fs::write(root.join("data/pack/script.cpz"), archive).unwrap();
    std::fs::write(
        root.join("cvns.cfg"),
        b"[SOUND_INFO]\nMASTER_VOLUME=80\nVIDEO_VOLUME=50\n",
    )
    .unwrap();

    // Minimal 3-frame CMV3 at 24 fps with optional trailing Ogg entry.
    let frames = 3u32;
    let has_audio = audio_flag != 0;
    let count = frames + u32::from(has_audio);
    let resource_offset = 44 + count * 20;
    let payload_size = frames * 4 + if has_audio { 4 } else { 0 };
    let mut cmv = b"CMV3".to_vec();
    for w in [
        resource_offset,
        resource_offset + payload_size,
        0,
        frames,
        24,
        1,
        800,
        600,
        24,
        audio_flag,
    ] {
        cmv.extend(w.to_le_bytes());
    }
    for i in 0..frames {
        for w in [i, 4, 800 * 600 * 3, 2, i * 4] {
            cmv.extend(w.to_le_bytes());
        }
    }
    if has_audio {
        for w in [frames, 4, 4, 0, frames * 4] {
            cmv.extend(w.to_le_bytes());
        }
    }
    for _ in 0..frames {
        cmv.extend(b"JBP1");
    }
    if has_audio {
        cmv.extend(b"OggS");
    }
    std::fs::write(root.join("data/video/op.cmv"), cmv).unwrap();

    // text: 0 = "op.cmv\0", 7 = "data/video/\0"
    // Program: 0x2025("data/video/", 0); 0x20c0("op.cmv"); 0x2040(peek=1); 0x2000
    let mut code = vec![];
    push_const(&mut code, 0x201, 0x120, 7);
    push_const(&mut code, 0x200, 0x100, 0);
    word(&mut code, 0x2025);
    push_const(&mut code, 0x201, 0x120, 0);
    word(&mut code, 0x20c0);
    push_const(&mut code, 0x200, 0x100, 1);
    word(&mut code, 0x2040);
    word(&mut code, 0x2000);
    let text = b"op.cmv\0data/video/\0";
    let start_raw = script(&code, text);
    std::fs::write(root.join("data/pack/start.pss"), &start_raw).unwrap();
    (root, start_raw)
}

#[derive(Default)]
struct SharedAudioLog(std::sync::Arc<std::sync::Mutex<cmvs_core::host::audio::RecordingAudio>>);
impl cmvs_core::host::audio::AudioBackend for SharedAudioLog {
    fn play(
        &mut self,
        request: &cmvs_core::host::audio::AudioRequest,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.0.lock().unwrap().play(request)
    }
    fn configure_paths(
        &mut self,
        paths: &BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.lock().unwrap().configure_paths(paths)
    }
    fn configure_archive_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), cmvs_core::format::cpz::Cpz7Parameters>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0
            .lock()
            .unwrap()
            .configure_archive_parameters(parameters)
    }
    fn configure_voice_ducking(
        &mut self,
        _settings: cmvs_core::host::audio::VoiceDuckingSettings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    fn set_output_enabled(
        &mut self,
        _enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    fn play_movie_audio(
        &mut self,
        request: &cmvs_core::host::audio::AudioRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.lock().unwrap().play_movie_audio(request)
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.lock().unwrap().stop_movie_audio()
    }
    fn movie_clock(
        &self,
    ) -> Result<
        Option<cmvs_core::host::audio::MovieAudioClock>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        self.0.lock().unwrap().movie_clock()
    }
}

#[test]
fn cvns_runtime_0x20c0_blocks_vm_scales_volume_and_consumes_skip_input() {
    use cmvs_core::{
        host::audio::AudioRequest, host::input::CodeInput, host::input::KeyBindings,
        host::platform::RecordingBackend, profile::Registry, runtime::Runtime,
        runtime::RuntimeEvent, runtime::load_start,
    };

    let tick_until_yield = |run: &mut Runtime, platform: &mut RecordingBackend| loop {
        match run.tick(platform).unwrap() {
            RuntimeEvent::Progress => {}
            RuntimeEvent::Yield => break,
            other => panic!("unexpected runtime event: {other:?}"),
        }
    };

    // 1. Natural playback end: holds VM while playing, uses 100-based CVNS movie volume, then resumes.
    let (root, _) = cvns_movie_fixture("natural", 2);
    let game = Registry::builtin()
        .identify_game(&root, Some("cvns-20090203"))
        .unwrap();
    let start = load_start(&root, &game.files).unwrap();
    let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
    let audio_log = std::sync::Arc::new(std::sync::Mutex::new(
        cmvs_core::host::audio::RecordingAudio::default(),
    ));
    run.set_audio_backend(Box::new(SharedAudioLog(audio_log.clone())))
        .unwrap();
    let mut platform = RecordingBackend::default();

    tick_until_yield(&mut run, &mut platform);
    assert!(run.movies().has_cvns_modal_player());
    assert_eq!(run.machine().state().system_integer(0), Some(1));
    // MASTER=80 * VIDEO=50 / 100 = 40% => (40 * 256 / 100) = 102.
    assert_eq!(
        audio_log.lock().unwrap().requests(),
        &[AudioRequest::PlayMovieAudio {
            path: b"data/video/op.cmv".to_vec(),
            offset: 44 + 4 * 20 + 12,
            size: 4,
            volume: 102,
        }]
    );

    let steps_at_open = run.steps();
    run.advance_playback(40).unwrap();
    assert!(run.movies().has_cvns_modal_player());
    assert_eq!(run.tick(&mut platform).unwrap(), RuntimeEvent::Yield);
    assert_eq!(run.steps(), steps_at_open);

    run.advance_playback(100).unwrap();
    assert!(!run.movies().has_cvns_modal_player());
    tick_until_yield(&mut run, &mut platform);
    assert!(run.steps() > steps_at_open);
    let _ = std::fs::remove_dir_all(&root);

    // 2. Skip input via left click (code 1) and Return (code 22) stops movie audio and consumes edge.
    for (tag, skip_code, expected_mask) in [("click", 1usize, 1 << 24), ("enter", 22usize, 0)] {
        let (root, _) = cvns_movie_fixture(tag, 2);
        let game = Registry::builtin()
            .identify_game(&root, Some("cvns-20090203"))
            .unwrap();
        let start = load_start(&root, &game.files).unwrap();
        let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
        let audio_log = std::sync::Arc::new(std::sync::Mutex::new(
            cmvs_core::host::audio::RecordingAudio::default(),
        ));
        run.set_audio_backend(Box::new(SharedAudioLog(audio_log.clone())))
            .unwrap();
        let mut platform = RecordingBackend::default();
        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();

        tick_until_yield(&mut run, &mut platform);
        assert!(run.movies().has_cvns_modal_player());
        let steps_at_open = run.steps();

        let mut now = [false; 128];
        now[skip_code] = true;
        codes.update(&bindings, &now, run.input_mut());
        // Advances past 0x2040(peek=1) into 0x2000; 0x2040 must NOT see the consumed skip press.
        tick_until_yield(&mut run, &mut platform);
        assert!(!run.movies().has_cvns_modal_player());
        assert!(run.steps() > steps_at_open);
        assert_eq!(run.machine().state().system_integer(0), Some(expected_mask));
        assert_eq!(
            audio_log.lock().unwrap().requests().last(),
            Some(&AudioRequest::StopMovieAudio)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn cvns_system_save_reads_registered_template_until_a_user_save_exists() {
    use cmvs_core::{
        host::platform::RecordingBackend, profile::Registry, runtime::Runtime,
        runtime::RuntimeEvent,
    };

    let (root, _) = cvns_movie_fixture("registered-css0", 0);
    let user = crate::support::temp_dir::TempDir::new("cvns-css0-user");
    let (mut payload, starts) = css0_payload();
    std::fs::create_dir_all(root.join("data/save")).unwrap();
    std::fs::write(
        root.join("data/save/save000.dat"),
        css0(&payload, [1, 0], [1, 2]),
    )
    .unwrap();
    let mut code = Vec::new();
    push_const(&mut code, 0x201, 0x120, 0);
    push_const(&mut code, 0x200, 0x100, 0);
    word(&mut code, 0x2026);
    for (op, start) in [
        (0x210b, starts.bit),
        (0x210c, starts.integer),
        (0x210d, starts.string),
    ] {
        push_const(&mut code, 0x200, 0x100, start);
        word(&mut code, op);
    }
    word(&mut code, 0x21c0);
    word(&mut code, 0x2000);
    let start = Script::decode(&script(&code, b"data/save/\0")).unwrap();
    let game = Registry::builtin()
        .identify_game(&root, Some("cvns-20090203"))
        .unwrap();
    for expected in [-5i32, 42] {
        if expected == 42 {
            payload[1..5].copy_from_slice(&expected.to_le_bytes());
            std::fs::write(user.join("save000.dat"), css0(&payload, [1, 0], [1, 2])).unwrap();
        }
        let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
        run.set_user_data_directory(&user).unwrap();
        let mut platform = RecordingBackend::default();
        while run.tick(&mut platform).unwrap() != RuntimeEvent::Yield {}
        assert_eq!(
            run.machine().state().integer(starts.integer),
            Some(expected)
        );
    }
}

#[test]
fn cvns_preferences_round_trip_menu_settings_and_volume_percentages() {
    use cmvs_core::{
        host::platform::RecordingBackend, host::settings::CvnsSettings, profile::Registry,
        runtime::Runtime, runtime::RuntimeEvent,
    };

    let (root, _) = cvns_movie_fixture("preferences", 0);
    let user = crate::support::temp_dir::TempDir::new("cvns-preferences");
    let game = Registry::builtin()
        .identify_game(&root, Some("cvns-20090203"))
        .unwrap();
    let mut code = Vec::new();
    let volumes = [50, 100, 25, 75, 60];
    for (id, value) in
        (10..=14)
            .zip(volumes)
            .chain([(2, 1), (16, 1), (22, 1), (27, 1), (28, 2), (30, 1), (31, 2)])
    {
        for arg in [0, value, 0, id] {
            push_const(&mut code, 0x200, 0x100, arg);
        }
        word(&mut code, 0x21ed);
    }
    word(&mut code, 0x2000);
    let start = Script::decode(&script(&code, &[])).unwrap();
    let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
    run.set_user_data_directory(&user).unwrap();
    let mut platform = RecordingBackend::default();
    loop {
        match run.tick(&mut platform).unwrap() {
            RuntimeEvent::Progress => {}
            RuntimeEvent::Yield => break,
            other => panic!("unexpected runtime event: {other:?}"),
        }
    }
    assert_eq!(run.audio().settings().master, 128);
    assert!(run.settings().cvns().text_shadow);
    run.save_preferences(Default::default()).unwrap();
    drop(run);

    let mut resumed = Runtime::new(&start, &root, &game, 0).unwrap();
    resumed.set_user_data_directory(&user).unwrap();
    let audio = resumed.audio().settings();
    assert_eq!(
        [audio.master, audio.bgm, audio.se, audio.voice, audio.movie],
        volumes
    );
    assert_eq!(
        resumed.settings().cvns(),
        &CvnsSettings {
            wait_timing: true,
            text_shadow: true,
            auto_skip: true,
            mouse_right_mode: true,
            wheel_mode: 2,
            auto_move: true,
            frame_ctrl: 2,
            ..Default::default()
        }
    );
}

#[test]
fn cvns_0x21ab_save_comment_updates_csp_slot_and_skips_slot_zero() {
    use cmvs_core::{
        host::platform::RecordingBackend, profile::Registry, runtime::Runtime,
        runtime::RuntimeEvent, runtime::load_start,
    };

    let (root, _) = cvns_movie_fixture("save-comment", 0);
    // 0x2026(0, "data/save/"); 0x21ab(0, text[11]); 0x21ab(1, text[11]); 0x2000
    // text: 0 = "data/save/\0", 11 = "comment!\0"
    let mut code = vec![];
    push_const(&mut code, 0x201, 0x120, 0);
    push_const(&mut code, 0x200, 0x100, 0);
    word(&mut code, 0x2026);
    for slot in [0u32, 1] {
        push_const(&mut code, 0x201, 0x120, 11);
        push_const(&mut code, 0x200, 0x100, slot);
        word(&mut code, 0x21ab);
    }
    word(&mut code, 0x2000);
    std::fs::write(
        root.join("data/pack/start.pss"),
        script(&code, b"data/save/\0comment!\0"),
    )
    .unwrap();

    for profile in [
        "cvns-20040515",
        "cvns-20040730",
        "cvns-20050216",
        "cvns-20050727",
        "cvns-20090203",
    ] {
        let game = Registry::builtin()
            .identify_game(&root, Some(profile))
            .unwrap();
        let start = load_start(&root, &game.files).unwrap();
        let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
        let user = root.with_extension(format!("user-{profile}"));
        run.set_user_data_directory(&user).unwrap();
        let mut css0 = vec![0x5au8; 700];
        css0[..4].copy_from_slice(b"CSS0");
        let mut csp1 = vec![0x5au8; 900];
        csp1[..4].copy_from_slice(b"CSP1");
        std::fs::write(user.join("save000.dat"), &css0).unwrap();
        std::fs::write(user.join("save001.dat"), &csp1).unwrap();

        let mut platform = RecordingBackend::default();
        while run.tick(&mut platform).unwrap() != RuntimeEvent::Yield {}
        assert_eq!(run.machine().state().system_integer(0), Some(1));

        assert_eq!(std::fs::read(user.join("save000.dat")).unwrap(), css0);
        let mut expected = csp1;
        expected[272..281].copy_from_slice(b"comment!\0");
        assert_eq!(std::fs::read(user.join("save001.dat")).unwrap(), expected);
        drop(run);
        let _ = std::fs::remove_dir_all(&user);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cvns_color_dialog_preserves_rgb_and_commits_only_after_the_host_answers() {
    use cmvs_core::{
        host::platform::{PlatformBackend, PlatformRequest},
        profile::Registry,
        vm::dispatch::{self, Flow, Hosts},
    };

    struct Picker {
        answer: Option<u32>,
        fail: bool,
    }

    impl PlatformBackend for Picker {
        fn apply(
            &mut self,
            _: &PlatformRequest,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unreachable!()
        }

        fn choose_color(
            &mut self,
            swatches: [u32; 2],
        ) -> Result<Option<u32>, Box<dyn std::error::Error + Send + Sync>> {
            assert_eq!(swatches, [0x445566, 0x112233]);
            if self.fail {
                return Err("color picker failed".into());
            }
            Ok(self.answer)
        }
    }

    let mut code = Vec::new();
    push_const(&mut code, 0x200, 0x100, 0xff112233);
    push_const(&mut code, 0x200, 0x100, 0xff445566);
    word(&mut code, 0x21b7);
    word(&mut code, 0x2000);
    let script = Script::decode(&script(&code, &[])).unwrap();

    for id in [
        "cvns-20040515",
        "cvns-20040730",
        "cvns-20050216",
        "cvns-20050727",
        "cvns-20090203",
    ] {
        let profile = Registry::builtin().profile(id).unwrap();
        for (answer, result, color) in [(Some(0xdb8127), 1, 0xdb8127), (None, 0, 0x00aaaa)] {
            let mut vm = Machine::new(&script).unwrap();
            vm.set_command_map(profile.command_map());
            vm.state_mut().set_system_integer(0, 99).unwrap();
            vm.state_mut().set_system_integer(4, 0x00aaaa).unwrap();
            while !matches!(vm.step().unwrap(), Step::System { .. }) {}
            let location = vm.pending_system();
            let (_, cmd) = location.unwrap();
            assert_eq!(cmd, 0x5b7);
            let mut picker = Picker { answer, fail: true };
            let run = |vm: &mut Machine, picker: &mut Picker| {
                dispatch::run(
                    profile.semantics(cmd),
                    vm,
                    &mut Hosts {
                        device_frame: Some(picker),
                        ..Default::default()
                    },
                )
            };
            assert!(run(&mut vm, &mut picker).is_err());
            assert_eq!(vm.pending_system(), location);
            assert_eq!(vm.stack().len_bytes(), 8);
            assert_eq!(vm.state().system_integer(0), Some(99));
            assert_eq!(vm.state().system_integer(4), Some(0x00aaaa));

            picker.fail = false;
            assert_eq!(run(&mut vm, &mut picker).unwrap(), Some(Flow::Done));
            assert_eq!(vm.pending_system(), None);
            assert_eq!(vm.stack().len_bytes(), 0);
            assert_eq!(vm.state().system_integer(0), Some(result));
            assert_eq!(vm.state().system_integer(4), Some(color));
        }
    }
}

#[test]
fn cvns_in_game_color_picker_previews_edits_and_keeps_the_script_paused() {
    use cmvs_core::{
        host::input::{CodeInput, KeyBindings},
        host::platform::RecordingBackend,
        profile::Registry,
        runtime::{Runtime, RuntimeEvent},
    };

    let (root, _) = cvns_movie_fixture("color-picker", 0);
    let mut code = Vec::new();
    for _ in 0..2 {
        push_const(&mut code, 0x200, 0x100, 0xff112233);
        push_const(&mut code, 0x200, 0x100, 0xff445566);
        word(&mut code, 0x21b7);
    }
    word(&mut code, 0x2000);
    let start = Script::decode(&script(&code, &[])).unwrap();
    let game = Registry::builtin()
        .identify_game(&root, Some("cvns-20040730"))
        .unwrap();
    let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
    run.enable_in_game_color_picker();
    let mut platform = RecordingBackend::default();
    let mut codes = CodeInput::default();
    let bindings = KeyBindings::default();
    let [width, height] = run.screen_size().map(|v| v as i32);
    let [x, y] = [(width - 400) / 2, (height - 234) / 2];
    let pixel = |run: &Runtime, dx, dy| {
        let frame = run.render_scene_frame().unwrap();
        let offset = ((y + dy) * width + x + dx) as usize * 4;
        <[u8; 4]>::try_from(&frame.pixels()[offset..offset + 4]).unwrap()
    };
    let mut input_frame =
        |run: &mut Runtime, platform: &mut RecordingBackend, dx, dy, code: Option<usize>| {
            let mut keys = [false; 128];
            if let Some(code) = code {
                keys[code] = true;
            }
            let input = run.input_mut();
            input.set_pointer(x + dx, y + dy);
            codes.update(&bindings, &keys, input);
            assert_eq!(run.tick(platform).unwrap(), RuntimeEvent::Yield);
            run.advance_playback(16).unwrap();
        };
    let open = |run: &mut Runtime, platform: &mut RecordingBackend| {
        loop {
            match run.tick(platform).unwrap() {
                RuntimeEvent::Yield => break,
                RuntimeEvent::Progress => {}
                other => panic!("unexpected runtime event: {other:?}"),
            }
        }
        assert_eq!(run.machine().stack().len_bytes(), 8);
        assert_eq!(pixel(run, 260, 26), [0, 0, 0, 255]);
    };
    let clear = pixel(&run, 260, 26);
    let original = run.machine().state().system_integer(4);
    open(&mut run, &mut platform);
    let steps = run.steps();
    input_frame(&mut run, &mut platform, 36, 200, Some(1));
    input_frame(&mut run, &mut platform, 36, 200, None);
    assert_eq!(pixel(&run, 260, 26), [0x44, 0x55, 0x66, 255]);

    input_frame(&mut run, &mut platform, 80, 74, Some(1));
    input_frame(&mut run, &mut platform, 248, 74, Some(1));
    input_frame(&mut run, &mut platform, 248, 74, None);
    input_frame(&mut run, &mut platform, 248, 74, Some(36)); // Right adjusts red by one.
    assert_eq!(pixel(&run, 260, 26), [201, 0x55, 0x66, 255]);
    assert_eq!(run.steps(), steps);
    assert_eq!(run.machine().stack().len_bytes(), 8);
    assert_eq!(run.machine().state().system_integer(4), original);

    input_frame(&mut run, &mut platform, 248, 74, Some(22)); // Return confirms.
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.machine().state().system_integer(0), Some(1));
    assert_eq!(run.machine().state().system_integer(4), Some(0xc95566));
    assert_eq!(pixel(&run, 260, 26), clear);

    open(&mut run, &mut platform);
    input_frame(&mut run, &mut platform, 83, 200, Some(1));
    input_frame(&mut run, &mut platform, 83, 200, None);
    assert_eq!(pixel(&run, 260, 26), [0x11, 0x22, 0x33, 255]);
    input_frame(&mut run, &mut platform, 83, 200, Some(25)); // Escape cancels.
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    assert_eq!(run.machine().state().system_integer(4), Some(0xc95566));
    assert_eq!(pixel(&run, 260, 26), clear);
}

#[test]
fn cvns_in_game_message_dialogs_wait_for_the_player_and_return_the_answer() {
    use cmvs_core::{
        host::input::{CodeInput, KeyBindings},
        host::platform::RecordingBackend,
        profile::Registry,
        runtime::{Runtime, RuntimeEvent},
    };

    let (root, _) = cvns_movie_fixture("message-dialog", 0);
    let mut code = Vec::new();
    word(&mut code, 0x21b0);
    for mode in [0, 1] {
        push_const(&mut code, 0x201, 0x120, 6);
        push_const(&mut code, 0x201, 0x120, 0);
        push_const(&mut code, 0x200, 0x100, mode);
        word(&mut code, 0x21b1);
    }
    word(&mut code, 0x2000);
    let start = Script::decode(&script(&code, b"TITLE\0BODY\0")).unwrap();
    let game = Registry::builtin()
        .identify_game(&root, Some("cvns-20040730"))
        .unwrap();
    let mut run = Runtime::new(&start, &root, &game, 0).unwrap();
    run.enable_in_game_dialogs(None);
    let mut platform = RecordingBackend::default();
    let mut codes = CodeInput::default();
    let bindings = KeyBindings::default();
    let [width, height] = run.screen_size().map(|v| v as i32);
    let [x, y] = [(width - 400) / 2, (height - 234) / 2];

    let mut frame = |run: &mut Runtime, point: [i32; 2], code: Option<usize>| {
        let mut keys = [false; 128];
        if let Some(code) = code {
            keys[code] = true;
        }
        codes.update(&bindings, &keys, run.input_mut());
        run.input_mut().set_pointer(x + point[0], y + point[1]);
        loop {
            match run.tick(&mut platform).unwrap() {
                RuntimeEvent::Yield => break,
                RuntimeEvent::Progress => {}
                event => panic!("unexpected runtime event: {event:?}"),
            }
        }
        run.advance_playback(16).unwrap();
    };
    let clear = run.render_scene_frame().unwrap();
    frame(&mut run, [0, 0], Some(22)); // Opening Return is consumed.
    let exit_call = run.machine().pending_system();
    assert_eq!(exit_call.map(|(_, command)| command), Some(0x5b0));
    let decode = |code: u16| char::from_u32(u32::from(code)).unwrap();
    let page = run.dialog_page(decode).unwrap();
    assert_eq!(page.text[1].1, "QUIT THE GAME?");
    assert_eq!(
        page.buttons
            .iter()
            .map(|b| b.label.as_str())
            .collect::<Vec<_>>(),
        ["Cancel", "Yes"]
    );
    let steps = run.steps();
    assert_ne!(run.render_scene_frame().unwrap(), clear);
    frame(&mut run, [0, 0], Some(22));
    assert_eq!(run.steps(), steps);
    assert_eq!(run.machine().pending_system(), exit_call);
    frame(&mut run, [0, 0], None);
    frame(&mut run, [0, 0], Some(22)); // Default button is Cancel.
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    assert_eq!(run.render_scene_frame().unwrap(), clear);

    frame(&mut run, [0, 0], Some(3)); // An opening right click must not dismiss the alert.
    assert_eq!(
        run.machine().pending_system().map(|(_, cmd)| cmd),
        Some(0x5b1)
    );
    frame(&mut run, [0, 0], None);
    assert_eq!(run.machine().stack().len_bytes(), 12);
    let alert = run.render_scene_frame().unwrap();
    // The first letter of the script's title and body are both drawn.
    for (dx, dy) in [(18, 18), (18, 106)] {
        let i = (4 * ((y + dy) * width + x + dx)) as usize;
        assert_eq!(alert.pixels()[i..i + 3], [238, 242, 249]);
    }
    let page = run.dialog_page(decode).unwrap();
    assert_eq!(page.text[0].1, "TITLE");
    assert_eq!(page.text[1].1, "BODY");
    assert_eq!(page.buttons.len(), 1);
    assert_eq!(page.buttons[0].label, "OK");
    let [left, top, w, h] = page.buttons[0].rect;
    let ok = [left - x + w / 2, top - y + h / 2];
    frame(&mut run, ok, Some(1));
    assert_eq!(run.machine().stack().len_bytes(), 12);
    frame(&mut run, ok, None);
    assert!(run.dialog_page(decode).is_none());
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.machine().state().system_integer(0), Some(1));

    frame(&mut run, [0, 0], None);
    assert_eq!(
        run.machine().pending_system().map(|(_, cmd)| cmd),
        Some(0x5b1)
    );
    frame(&mut run, [0, 0], Some(25)); // Escape cancels a two-button prompt.
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert_eq!(run.machine().state().system_integer(0), Some(0));
    assert_eq!(run.render_scene_frame().unwrap(), clear);
    assert!(platform.message_dialogs().is_empty());
}

#[test]
fn comment_editor_waits_for_input_and_commits_only_on_panel_confirmation() {
    use cmvs_core::{
        host::{
            input::{CodeInput, KeyBindings},
            platform::{PlatformBackend, PlatformRequest, TextDialogRequest, TextDialogResult},
        },
        profile::Registry,
        runtime::{CommentEdit, Runtime, RuntimeEvent},
    };
    use std::error::Error;

    struct Keyboard {
        inline: bool,
        reply: Option<Vec<u8>>,
    }

    impl PlatformBackend for Keyboard {
        fn apply(&mut self, _: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
            unreachable!()
        }

        fn inline_comment_input(&self) -> bool {
            self.inline
        }

        fn poll_text_dialog(
            &mut self,
            request: &TextDialogRequest,
        ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
            assert!(!self.inline);
            assert_eq!(request.initial, b"\x82\xa0");
            assert!(request.prompt.ends_with(b"LIMIT ON THIRD LINE"));
            assert_eq!(request.max_bytes, 4);
            Ok(self
                .reply
                .take()
                .map_or(TextDialogResult::Pending, TextDialogResult::Accepted))
        }
    }

    let (root, _) = cvns_movie_fixture("comment-editor", 0);
    for (profile, opcode, command_id, inline) in [
        ("cmvs-20071117", 0x213c, 0x13c, true),
        ("cvns-20040730", 0x21b2, 0x5b2, false),
    ] {
        let game = Registry::builtin()
            .identify_game(&root, Some(profile))
            .unwrap();
        let mut code = Vec::new();
        let title = b"EDIT COMMENT\0";
        let prompt = b"ENTER A COMMENT\nSECOND LINE\nLIMIT ON THIRD LINE\0";
        let initial = (title.len() + prompt.len()) as u32;
        for _ in 0..3 {
            for argument in [0, 0x8000_0003, 4, initial, title.len() as u32, 0] {
                push_const(&mut code, 0x200, 0x100, argument);
            }
            word(&mut code, opcode);
        }
        word(&mut code, 0x2000);
        let text = [title.as_slice(), prompt, b"\x82\xa0\0"].concat();
        let start = Script::decode(&script(&code, &text)).unwrap();
        let mut runtime = Runtime::new(&start, &root, &game, 0).unwrap();
        runtime.enable_in_game_dialogs(None);
        let mut keyboard = Keyboard {
            inline,
            reply: None,
        };
        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();
        let [width, height] = runtime.screen_size().map(|size| size as i32);
        let [x, y] = [(width - 400) / 2, (height - 234) / 2];
        let ok = [x + 337, y + 200];
        runtime.input_mut().set_pointer(ok[0], ok[1]);
        let mut frame = |runtime: &mut Runtime, keyboard: &mut Keyboard, key: Option<usize>| {
            let mut keys = [false; 128];
            if let Some(key) = key {
                keys[key] = true;
            }
            codes.update(&bindings, &keys, runtime.input_mut());
            while runtime.tick(keyboard).unwrap() == RuntimeEvent::Progress {}
            runtime.advance_playback(16).unwrap();
        };
        let clear = runtime.render_scene_frame().unwrap();
        frame(&mut runtime, &mut keyboard, None);
        let command = runtime.machine().pending_system();
        assert_eq!(command.map(|(_, opcode)| opcode), Some(command_id));
        assert_eq!(runtime.machine().stack().len_bytes(), 24);
        assert_eq!(runtime.inline_comment_rect().is_some(), inline);
        let panel = runtime.render_scene_frame().unwrap();
        assert!(panel != clear);
        let accent = (y * width + x) as usize * 4;
        assert_eq!(&panel.pixels()[accent..accent + 4], &[127, 187, 255, 255]);

        let accepted = if inline {
            b"\x82\xa2A".as_slice()
        } else {
            b"A\x82\xa2"
        };
        if inline {
            assert!(runtime.edit_comment(CommentEdit::Tab { backwards: true }));
            frame(&mut runtime, &mut keyboard, Some(22));
            let scrolled = runtime.render_scene_frame().unwrap();
            let row = ((y + 84) * width + x + 18) as usize * 4;
            assert_ne!(
                &scrolled.pixels()[row..row + 320],
                &panel.pixels()[row..row + 320]
            );
            assert!(runtime.edit_comment(CommentEdit::Tab { backwards: false }));
            assert!(runtime.edit_comment(CommentEdit::SelectAll));
            assert!(runtime.edit_comment(CommentEdit::Insert(b"longer than four bytes".to_vec())));
            assert!(runtime.edit_comment(CommentEdit::Preedit(b"\x82\xa2".to_vec())));
            frame(&mut runtime, &mut keyboard, Some(1));
            assert!(runtime.inline_comment_rect().is_some());
            // A click started during preedit cannot confirm even if the IME commits by mouseup.
            assert!(runtime.edit_comment(CommentEdit::Insert(b"\x82\xa2".to_vec())));
            frame(&mut runtime, &mut keyboard, None);
            assert_eq!(runtime.machine().pending_system(), command);

            assert!(runtime.edit_comment(CommentEdit::SelectAll));
            // Click the right half of the wide glyph to insert after it.
            runtime.input_mut().set_pointer(x + 28 + 12, y + 131);
            frame(&mut runtime, &mut keyboard, Some(1));
            frame(&mut runtime, &mut keyboard, None);
            assert!(runtime.edit_comment(CommentEdit::Insert(b"A".to_vec())));
            runtime.input_mut().set_pointer(ok[0], ok[1]);
        } else {
            frame(&mut runtime, &mut keyboard, Some(22));
            frame(&mut runtime, &mut keyboard, None);
            assert!(runtime.keyboard_comment_rect().is_some());
            keyboard.reply = Some(b"longer than four bytes".to_vec());
            frame(&mut runtime, &mut keyboard, None);
            frame(&mut runtime, &mut keyboard, Some(22));
            keyboard.reply = Some(accepted.to_vec());
        }
        frame(&mut runtime, &mut keyboard, None);
        assert_eq!(runtime.machine().pending_system(), command);
        assert_eq!(runtime.machine().stack().len_bytes(), 24);
        assert!(runtime.render_scene_frame().unwrap() != panel);

        frame(&mut runtime, &mut keyboard, Some(1));
        frame(&mut runtime, &mut keyboard, None);
        assert_eq!(runtime.machine().state().system_integer(0), Some(1));
        assert_eq!(runtime.machine().state().string(3), Some(accepted));
        assert_eq!(runtime.machine().stack().len_bytes(), 0);
        assert!(runtime.render_scene_frame().unwrap() == clear);

        frame(&mut runtime, &mut keyboard, None);
        // Four bytes fit, but the legacy field accepts only two characters.
        let too_many = b"A\x82\xa2B".to_vec();
        if inline {
            assert!(runtime.edit_comment(CommentEdit::Insert(too_many)));
        } else {
            frame(&mut runtime, &mut keyboard, Some(22));
            frame(&mut runtime, &mut keyboard, None);
            keyboard.reply = Some(too_many);
        }
        frame(&mut runtime, &mut keyboard, None);
        frame(&mut runtime, &mut keyboard, Some(1));
        frame(&mut runtime, &mut keyboard, None);
        if !inline {
            assert_eq!(runtime.machine().stack().len_bytes(), 24);
            frame(&mut runtime, &mut keyboard, Some(25));
        }
        assert_eq!(
            runtime.machine().state().string(3),
            Some(b"\x82\xa0".as_slice())
        );

        frame(&mut runtime, &mut keyboard, None);
        frame(&mut runtime, &mut keyboard, Some(25));
        assert_eq!(runtime.machine().state().system_integer(0), Some(0));
        assert_eq!(
            runtime.machine().state().string(3),
            Some(b"\x82\xa0".as_slice())
        );
        assert!(runtime.render_scene_frame().unwrap() == clear);
    }
}

//! CVNS runtime pieces with synthetic data: CSS0, the save-menu scan, the 256-slot string
//! bank and the translated common-script entry.
use crate::support::shared_storage;
use cmvs_core::{
    cvns::CommandMap,
    evaluate::State,
    machine::{Machine, Step},
    ps3::Script,
    save_menu::{SaveMenu, SaveSource, SlotStatus},
    script_control::{ControlEvent, LoadError, ScriptControl, ScriptSource},
    system_save::{Css0Save, PersistentStarts},
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
    let archive: Vec<u8> = include_str!("../../../../tests/fixtures/cpz6-script.hex")
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
struct SharedAudioLog(std::rc::Rc<std::cell::RefCell<cmvs_core::audio::RecordingAudio>>);
impl cmvs_core::audio::AudioBackend for SharedAudioLog {
    fn play(
        &mut self,
        request: &cmvs_core::audio::AudioRequest,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.0.borrow_mut().play(request)
    }
    fn configure_paths(
        &mut self,
        paths: &BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.borrow_mut().configure_paths(paths)
    }
    fn configure_archive_parameters(
        &mut self,
        parameters: &BTreeMap<(u16, u32), cmvs_core::cpz::Cpz7Parameters>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.borrow_mut().configure_archive_parameters(parameters)
    }
    fn configure_voice_ducking(
        &mut self,
        _settings: cmvs_core::audio::VoiceDuckingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
    fn set_output_enabled(&mut self, _enabled: bool) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
    fn play_movie_audio(
        &mut self,
        request: &cmvs_core::audio::AudioRequest,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.borrow_mut().play_movie_audio(request)
    }
    fn stop_movie_audio(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.0.borrow_mut().stop_movie_audio()
    }
    fn movie_clock(
        &self,
    ) -> Result<Option<cmvs_core::audio::MovieAudioClock>, Box<dyn std::error::Error>> {
        self.0.borrow().movie_clock()
    }
}

#[test]
fn cvns_runtime_0x20c0_blocks_vm_scales_volume_and_consumes_skip_input() {
    use cmvs_core::{
        audio::AudioRequest,
        input::{CodeInput, KeyBindings},
        platform::RecordingBackend,
        profile::Registry,
        runtime::{Runtime, RuntimeEvent, load_start},
    };

    let tick_until_yield = |run: &mut Runtime<'_>, platform: &mut RecordingBackend| loop {
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
    let audio_log = std::rc::Rc::new(std::cell::RefCell::new(
        cmvs_core::audio::RecordingAudio::default(),
    ));
    run.set_audio_backend(Box::new(SharedAudioLog(audio_log.clone())))
        .unwrap();
    let mut platform = RecordingBackend::default();

    tick_until_yield(&mut run, &mut platform);
    assert!(run.movies().has_cvns_modal_player());
    assert_eq!(run.machine().state().system_integer(0), Some(1));
    // MASTER=80 * VIDEO=50 / 100 = 40% => (40 * 256 / 100) = 102.
    assert_eq!(
        audio_log.borrow().requests(),
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
        let audio_log = std::rc::Rc::new(std::cell::RefCell::new(
            cmvs_core::audio::RecordingAudio::default(),
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
            audio_log.borrow().requests().last(),
            Some(&AudioRequest::StopMovieAudio)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn cvns_0x21ab_save_comment_updates_csp_slot_and_skips_slot_zero() {
    use cmvs_core::{
        platform::RecordingBackend,
        profile::Registry,
        runtime::{Runtime, RuntimeEvent, load_start},
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
fn cvns_2004_color_dialog_pops_args_and_clears_accept_flag() {
    use cmvs_core::{
        dispatch::{self, Flow, Hosts},
        profile::Registry,
    };
    for id in ["cvns-20040515", "cvns-20040730"] {
        let profile = Registry::builtin().profile(id).unwrap();
        let mut code = Vec::new();
        push_const(&mut code, 0x200, 0x100, 0x112233);
        push_const(&mut code, 0x200, 0x100, 0x445566);
        word(&mut code, 0x21b7);
        word(&mut code, 0x2000);
        let sc = Script::decode(&script(&code, &[])).unwrap();
        let mut vm = Machine::new(&sc).unwrap();
        vm.set_command_map(profile.command_map());
        vm.state_mut().set_system_integer(0, 99).unwrap();

        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let (_, cmd) = vm.pending_system().unwrap();
        assert_eq!(
            dispatch::run(
                profile.semantics(cmd),
                &mut vm,
                &mut Hosts {
                    runtime: true,
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(0));
    }
}

use cmvs_rs::{
    audio::{AudioBackend, AudioRequest, AudioSettings, RecordingAudio},
    cmv::CmvIndex,
    machine::{Machine, Step},
    movie::{MovieHost, MovieSource, OpenedMovie},
    ps3::Script,
};
use std::{collections::BTreeMap, error::Error, io::Cursor};

fn script(code: &[u8]) -> Script {
    program(code, b"")
}
fn program(code: &[u8], text: &[u8]) -> Script {
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
    Script::decode(&raw).unwrap()
}
fn slot_command(op: u16, slots: &[u32]) -> Vec<u8> {
    let mut code = vec![];
    for &slot in slots {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(slot.to_le_bytes());
        code.extend(op.to_le_bytes());
    }
    code
}
fn destroy(slots: &[u32]) -> Vec<u8> {
    slot_command(0x230a, slots)
}
fn query(slots: &[u32]) -> Vec<u8> {
    slot_command(0x231a, slots)
}
fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}

#[test]
fn destroying_constructor_empty_slots_completes_without_a_result() {
    let script = script(&destroy(&[0, 1, 2]));
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let mut movies = MovieHost::default();
    for _ in 0..3 {
        pending(&mut vm);
        assert!(movies.try_handle(&mut vm).unwrap());
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert!((0..3).all(|slot| !movies.occupied(slot)));
}

#[test]
fn out_of_range_slots_and_short_stacks_keep_the_command_pending() {
    for op in [0x230au16, 0x231a] {
        for code in [
            slot_command(op, &[3]),
            slot_command(op, &[u32::MAX]),
            op.to_le_bytes().to_vec(),
        ] {
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            vm.state_mut().set_system_integer(0, 9).unwrap();
            pending(&mut vm);
            let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
            assert!(MovieHost::default().try_handle(&mut vm).is_err());
            assert_eq!(vm.pending_system(), Some((pc, op - 0x2000)));
            assert_eq!(vm.stack().len_bytes(), bytes);
            assert_eq!(vm.state().system_integer(0), Some(9));
        }
    }
}

#[test]
fn scene_creation_requires_the_resource_aware_entrypoint() {
    let mut code = vec![];
    code.extend(0x2308u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    assert!(!MovieHost::default().try_handle(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x308));
}

/// A CMV6 file with `frames` 4-byte JBPX entries and, optionally, a 3-byte Ogg entry.
fn cmv(frames: u32, rate: u32, rate_time: u32, audio: bool) -> Vec<u8> {
    let count = frames + u32::from(audio);
    let resource = 44 + count * 20;
    let data = frames * 4 + if audio { 3 } else { 0 };
    let mut bytes = b"CMV6".to_vec();
    // max_sequence is the frame limit; the index holds max_sequence + 1 entries.
    let max_sequence = count - 1;
    for word in [
        resource,
        resource + data,
        0,
        max_sequence,
        rate,
        rate_time,
        1280,
        720,
        24,
        2,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for i in 0..frames {
        for word in [i, 4, 16, 2, i * 4] {
            bytes.extend(word.to_le_bytes());
        }
    }
    if audio {
        for word in [frames, 3, 3, 0, frames * 4] {
            bytes.extend(word.to_le_bytes());
        }
    }
    bytes.resize(bytes.len() + data as usize, 0);
    bytes
}

#[derive(Default)]
struct Files(BTreeMap<Vec<u8>, Vec<u8>>);
impl MovieSource for Files {
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>> {
        let Some(bytes) = self.0.get(name) else {
            return Ok(None);
        };
        let mut path = b"data/video/".to_vec();
        path.extend_from_slice(name);
        Ok(Some((path, CmvIndex::read(&mut Cursor::new(bytes))?)))
    }
}
struct Broken;
impl AudioBackend for Broken {
    fn play(&mut self, _: &AudioRequest) -> Result<bool, Box<dyn Error>> {
        Err("synthetic device failure".into())
    }
}

/// 0x20c0 with the name at text offset 0, then `queries` times 0x20c2, then 0x20c1.
fn player_code(queries: usize) -> Vec<u8> {
    let mut code = vec![];
    code.extend(0x201u16.to_le_bytes());
    code.extend(0x120u16.to_le_bytes());
    code.extend(0u32.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    code.extend(0x430u16.to_le_bytes());
    code.extend(0x20c0u16.to_le_bytes());
    for _ in 0..queries {
        code.extend(0x20c2u16.to_le_bytes());
    }
    code.extend(0x20c1u16.to_le_bytes());
    code
}
fn step(
    vm: &mut Machine<'_>,
    movies: &mut MovieHost,
    files: &mut Files,
    backend: &mut dyn AudioBackend,
) -> Result<bool, Box<dyn Error>> {
    pending(vm);
    movies.try_handle_player(vm, files, AudioSettings::default(), backend)
}

#[test]
fn frames_follow_elapsed_time_and_the_player_ends_at_max_sequence() {
    let script = program(&player_code(3), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut files = Files::default();
    files.0.insert(b"a.cmv".to_vec(), cmv(3, 24, 1, true));
    let (mut movies, mut audio) = (MovieHost::default(), RecordingAudio::default());
    assert!(step(&mut vm, &mut movies, &mut files, &mut audio).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(
        audio.requests(),
        &[AudioRequest::PlayMovieAudio {
            path: b"data/video/a.cmv".to_vec(),
            offset: 44 + 4 * 20 + 12,
            size: 3,
            volume: 256
        }]
    );
    // 41 ms is frame 0 at 24 fps; 42 ms reaches frame 1.
    movies.advance(41);
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    movies.advance(1);
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(1));
    // 125 ms is frame 3, which max_sequence 3 no longer decodes: the player is gone.
    movies.advance(83);
    assert!(movies.player().is_none());
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    // Stopping without a player sends nothing.
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert_eq!(audio.requests().len(), 1);
}

#[test]
fn high_rates_use_the_floating_point_path() {
    let script = program(&player_code(1), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut files = Files::default();
    files
        .0
        .insert(b"a.cmv".to_vec(), cmv(100, 30000, 1001, true));
    let (mut movies, mut audio) = (MovieHost::default(), RecordingAudio::default());
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    // 30000 / 1000 / 1001 * 1000 ms = 29.97 frames, truncated.
    movies.advance(1000);
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(29));
    // 0x20c1 destroys the running player and stops channel 2.
    step(&mut vm, &mut movies, &mut files, &mut audio).unwrap();
    assert!(movies.player().is_none());
    assert_eq!(audio.requests().last(), Some(&AudioRequest::StopMovieAudio));
}

#[test]
fn missing_files_fail_quietly_and_bad_movies_keep_the_command_pending() {
    let script = program(&player_code(0), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    let (mut movies, mut audio) = (MovieHost::default(), RecordingAudio::default());
    step(&mut vm, &mut movies, &mut Files::default(), &mut audio).unwrap();
    assert_eq!(vm.state().system_integer(0), Some(0));
    assert!(movies.player().is_none());
    // The failed player's destructor stops channel 2.
    assert_eq!(audio.requests(), &[AudioRequest::StopMovieAudio]);

    let mut silent = Files::default();
    silent.0.insert(b"a.cmv".to_vec(), cmv(3, 24, 1, false));
    let mut broken = Files::default();
    broken.0.insert(b"a.cmv".to_vec(), b"CMV5".to_vec());
    let mut fine = Files::default();
    fine.0.insert(b"a.cmv".to_vec(), cmv(3, 24, 1, true));
    for (mut files, backend_fails) in [(silent, false), (broken, false), (fine, true)] {
        let mut vm = Machine::new(&script).unwrap();
        let mut movies = MovieHost::default();
        let outcome = if backend_fails {
            step(&mut vm, &mut movies, &mut files, &mut Broken)
        } else {
            step(
                &mut vm,
                &mut movies,
                &mut files,
                &mut RecordingAudio::default(),
            )
        };
        assert!(outcome.is_err());
        assert_eq!(vm.pending_system().map(|p| p.1), Some(0xc0));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert!(movies.player().is_none());
    }
}

struct Frames {
    files: Files,
    reads: Vec<u32>,
    fail: bool,
}
impl MovieSource for Frames {
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>> {
        self.files.open(name)
    }
    fn frame(
        &mut self,
        _: &[u8],
        _: &CmvIndex,
        frame: u32,
    ) -> Result<cmvs_rs::raster::RgbaImage, Box<dyn Error>> {
        self.reads.push(frame);
        if self.fail {
            return Err("synthetic frame read failed".into());
        }
        Ok(cmvs_rs::raster::RgbaImage::new(
            1,
            1,
            vec![frame as u8, 0, 0, 255],
        )?)
    }
}
#[test]
fn decoded_frames_open_advance_skip_retry_and_disappear_at_end() {
    let script = program(&player_code(0), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut source = Frames {
        files: Files::default(),
        reads: vec![],
        fail: false,
    };
    source
        .files
        .0
        .insert(b"a.cmv".to_vec(), cmv(10, 24, 1, true));
    let mut movies = MovieHost::default();
    movies.set_decode_frames(true);
    let mut audio = RecordingAudio::default();
    movies
        .try_handle_player(&mut vm, &mut source, AudioSettings::default(), &mut audio)
        .unwrap();
    assert_eq!(source.reads, [0]);
    movies.advance_decoded(16, &mut source).unwrap();
    assert_eq!(source.reads, [0]);
    movies.advance_decoded(100, &mut source).unwrap();
    assert_eq!(source.reads, [0, 2]);
    assert_eq!(
        movies.player().unwrap().image.as_ref().unwrap().pixels(),
        [2, 0, 0, 255]
    );
    let before = movies.player().unwrap().clone();
    source.fail = true;
    assert!(movies.advance_decoded(100, &mut source).is_err());
    assert_eq!(movies.player(), Some(&before));
    source.fail = false;
    movies.advance_decoded(100, &mut source).unwrap();
    assert_eq!(movies.player().unwrap().frame, 5);
    movies.advance_decoded(300, &mut source).unwrap();
    assert!(movies.player().is_none());
}
#[test]
fn initial_frame_failure_preserves_vm_and_sends_no_audio() {
    let script = program(&player_code(0), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut source = Frames {
        files: Files::default(),
        reads: vec![],
        fail: true,
    };
    source
        .files
        .0
        .insert(b"a.cmv".to_vec(), cmv(3, 24, 1, true));
    let mut movies = MovieHost::default();
    movies.set_decode_frames(true);
    let mut audio = RecordingAudio::default();
    let pc = vm.pc();
    assert!(
        movies
            .try_handle_player(&mut vm, &mut source, AudioSettings::default(), &mut audio)
            .is_err()
    );
    assert_eq!(vm.pending_system(), Some((pc, 0xc0)));
    assert_eq!(vm.stack().len_bytes(), 4);
    assert!(movies.player().is_none());
    assert!(audio.requests().is_empty());
}

#[test]
fn audio_clock_is_absolute_and_does_not_accumulate_wall_time() {
    use cmvs_rs::audio::MovieAudioClock;
    let script = program(&player_code(0), b"a.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let mut source = Frames {
        files: Files::default(),
        reads: vec![],
        fail: false,
    };
    source
        .files
        .0
        .insert(b"a.cmv".to_vec(), cmv(100, 24, 1, true));
    let mut movies = MovieHost::default();
    movies.set_decode_frames(true);
    movies
        .try_handle_player(
            &mut vm,
            &mut source,
            AudioSettings::default(),
            &mut RecordingAudio::default(),
        )
        .unwrap();
    for position_ms in [0, 0, 100, 100, 80] {
        movies
            .advance_with_clock(
                1000,
                Some(MovieAudioClock::Playing { position_ms }),
                &mut source,
            )
            .unwrap();
    }
    assert_eq!(movies.player().unwrap().elapsed, 100);
    assert_eq!(movies.player().unwrap().frame, 2);
    assert_eq!(source.reads, [0, 2]);
    let before = movies.player().unwrap().clone();
    source.fail = true;
    assert!(
        movies
            .advance_with_clock(
                0,
                Some(MovieAudioClock::Playing { position_ms: 300 }),
                &mut source
            )
            .is_err()
    );
    assert_eq!(movies.player(), Some(&before));
    movies
        .advance_with_clock(0, Some(MovieAudioClock::Ended), &mut source)
        .unwrap();
    assert!(movies.player().is_none());
}

#[path = "support/cmv9.rs"]
mod slot_fixture;
struct SceneFiles(Option<Vec<u8>>);
impl cmvs_rs::resources::ResourceSource for SceneFiles {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Ok(self.0.clone())
    }
}
fn create_code(slot: u32, object: u32, frame: u32) -> Vec<u8> {
    let mut code = vec![];
    for arg in [frame, 0xdeadbeef, 0, object, slot] {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(0x2308u16.to_le_bytes());
    code
}
#[test]
fn scene_movie_creation_replaces_the_object_without_changing_results_or_starting_playback() {
    let mut code = create_code(0, 28, 0);
    code.extend(destroy(&[0]));
    let program = program(&code, b"synthetic.cmv\0");
    let mut vm = Machine::new(&program).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    let mut files = SceneFiles(Some(slot_fixture::movie(false)));
    vm.state_mut().set_system_integer(0, 123).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    assert!(
        movies
            .try_create_scene(&mut vm, &mut scene, &mut files)
            .unwrap()
    );
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
    assert_eq!(vm.state().system_integer(0), Some(123));
    assert_eq!(movies.scene_player(0).unwrap().object_slot, 28);
    assert_eq!(movies.scene_player(0).unwrap().frame, 0);
    let old = scene.objects().clone();
    movies.advance(1000);
    assert_eq!(scene.objects(), &old);
    pending(&mut vm);
    movies.try_handle(&mut vm).unwrap();
    assert!(!movies.occupied(0));
    assert_eq!(scene.objects(), &old);
    assert!(movies.player().is_none());
}
#[test]
fn failed_scene_movie_creation_preserves_command_stack_results_and_scene() {
    for (slot, object, frame, data) in [
        (3, 28, 0, Some(slot_fixture::movie(false))),
        (0, 256, 0, Some(slot_fixture::movie(false))),
        (0, 28, 1, Some(slot_fixture::movie(false))),
        (0, 28, 0, None),
        (0, 28, 0, Some(vec![0; 44])),
    ] {
        let code = create_code(slot, object, frame);
        let program = program(&code, b"synthetic.cmv\0");
        let mut vm = Machine::new(&program).unwrap();
        let mut movies = MovieHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 456).unwrap();
        let pc = vm.pc();
        let stack = vm.stack().len_bytes();
        assert!(
            movies
                .try_create_scene(&mut vm, &mut scene, &mut SceneFiles(data))
                .is_err()
        );
        assert_eq!(vm.pending_system(), Some((pc, 0x308)));
        assert_eq!(vm.stack().len_bytes(), stack);
        assert_eq!(vm.state().system_integer(0), Some(456));
        assert!(scene.objects().is_empty());
        assert!(!movies.occupied(0));
    }
}

#[test]
fn replacing_an_existing_scene_player_is_atomic_on_resource_failure() {
    let mut code = create_code(0, 28, 0);
    code.extend(create_code(0, 29, 0));
    let script = program(&code, b"synthetic.cmv\0");
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    let mut files = SceneFiles(Some(slot_fixture::movie(false)));
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    let old = scene.objects().clone();
    let encoded = movies.scene_player(0).unwrap().encoded().to_vec();
    pending(&mut vm);
    let pc = vm.pc();
    files.0 = None;
    assert!(
        movies
            .try_create_scene(&mut vm, &mut scene, &mut files)
            .is_err()
    );
    assert_eq!(vm.pending_system(), Some((pc, 0x308)));
    assert_eq!(vm.stack().len_bytes(), 20);
    assert_eq!(scene.objects(), &old);
    assert_eq!(movies.scene_player(0).unwrap().encoded(), encoded);
    files.0 = Some(slot_fixture::movie(true));
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    assert_eq!(movies.scene_player(0).unwrap().object_slot, 29);
    // Changing the binding leaves the old object in the scene, as the player owns no object.
    assert!(scene.object(28).is_some());
    assert!(scene.object(29).is_some());
}

fn scene_start(code: &mut Vec<u8>, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(0x2309u16.to_le_bytes());
}
fn scene_play_fixture(extra: &[u8]) -> (Script, SceneFiles) {
    let mut code = create_code(0, 28, 0);
    code.extend(extra);
    (
        program(&code, b"synthetic.cmv\0"),
        SceneFiles(Some(slot_fixture::movie_frames(4, 15, 1, true))),
    )
}
#[test]
fn scene_playback_uses_fractional_frame_time_and_keeps_the_final_image() {
    let mut code = vec![];
    scene_start(&mut code, &[0, 0, 0, u32::MAX, 0]);
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    pending(&mut vm);
    vm.state_mut().set_system_integer(0, 789).unwrap();
    movies.try_start_scene(&mut vm).unwrap();
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(789));
    assert_eq!(movies.scene_player(0).unwrap().playback.segments[0].end, 3);
    movies.advance_scene(66, &mut scene).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().frame, 0);
    movies.advance_scene(1, &mut scene).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().frame, 1);
    assert_eq!(
        &scene
            .object(28)
            .unwrap()
            .image
            .as_ref()
            .unwrap()
            .pixels
            .pixels()[64..68],
        &[152, 152, 152, 255]
    );
    movies.advance_scene(133, &mut scene).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().frame, 3);
    assert!(movies.scene_player(0).unwrap().playback.active);
    movies.advance_scene(67, &mut scene).unwrap();
    let player = movies.scene_player(0).unwrap();
    assert!(!player.playback.active);
    assert!(player.playback.paused);
    assert_eq!(player.frame, 3);
    let old = scene.objects().clone();
    movies.advance_scene(1000, &mut scene).unwrap();
    assert_eq!(scene.objects(), &old);
}
#[test]
fn scene_playback_loops_and_defers_a_new_segment_until_the_boundary() {
    let mut code = vec![];
    scene_start(&mut code, &[0, 0, 0, 1, 1]);
    scene_start(&mut code, &[0, 1, 2, 3, 0]);
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    movies.advance_scene(200, &mut scene).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().frame, 1);
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().playback.pending, Some(1));
    movies.advance_scene(67, &mut scene).unwrap();
    let player = movies.scene_player(0).unwrap();
    assert_eq!(
        (
            player.frame,
            player.playback.current,
            player.playback.pending
        ),
        (2, 1, None)
    );
    movies.advance_scene(133, &mut scene).unwrap();
    assert!(!movies.scene_player(0).unwrap().playback.active);
}
#[test]
fn scene_start_does_not_seek_and_preserves_a_paused_completion_state() {
    let mut code = vec![];
    scene_start(&mut code, &[0, 0, 2, 0, 0]);
    scene_start(&mut code, &[0, 0, 0, 3, 0]);
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    assert_eq!(movies.scene_player(0).unwrap().frame, 0);
    movies.advance_scene(67, &mut scene).unwrap();
    assert!(movies.scene_player(0).unwrap().playback.paused);
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    movies.advance_scene(1000, &mut scene).unwrap();
    let p = &movies.scene_player(0).unwrap().playback;
    assert!(p.active && p.paused);
    assert_eq!(movies.scene_player(0).unwrap().frame, 0);
}
fn query_result(vm: &mut Machine<'_>, movies: &mut MovieHost) -> Option<u32> {
    pending(vm);
    vm.state_mut().set_system_integer(0, 9).unwrap();
    let pc = vm.pc();
    assert!(movies.try_handle(vm).unwrap());
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (pc + 2, 0));
    vm.state().system_integer(0)
}
#[test]
fn scene_activity_query_follows_start_and_completion_and_reports_empty_slots_as_zero() {
    let mut code = query(&[0, 2]);
    scene_start(&mut code, &[0, 0, 0, u32::MAX, 0]);
    code.extend(query(&[0, 0]));
    code.extend(destroy(&[0]));
    code.extend(query(&[0]));
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    // A created player is stopped; an empty slot also reads 0.
    assert_eq!(query_result(&mut vm, &mut movies), Some(0));
    assert_eq!(query_result(&mut vm, &mut movies), Some(0));
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    let playback = movies.scene_player(0).unwrap().playback.clone();
    let objects = scene.objects().clone();
    assert_eq!(query_result(&mut vm, &mut movies), Some(1));
    assert_eq!(movies.scene_player(0).unwrap().playback, playback);
    assert_eq!(scene.objects(), &objects);
    movies.advance_scene(267, &mut scene).unwrap();
    let player = movies.scene_player(0).unwrap();
    assert!(!player.playback.active && player.playback.paused);
    assert_eq!(player.frame, 3);
    // Completion keeps the player and its last image but reports it inactive.
    assert_eq!(query_result(&mut vm, &mut movies), Some(0));
    assert!(movies.occupied(0));
    pending(&mut vm);
    movies.try_handle(&mut vm).unwrap();
    assert_eq!(query_result(&mut vm, &mut movies), Some(0));
    assert!(!movies.occupied(0));
}
#[test]
fn scene_playback_rejects_destroyed_bindings_without_committing_its_frame_or_time() {
    let mut code = vec![];
    scene_start(&mut code, &[0, 0, 0, 3, 1]);
    code.extend(destroy(&[]));
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(28u32.to_le_bytes());
    code.extend(0x2021u16.to_le_bytes());
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    let old = movies.scene_player(0).unwrap().playback.clone();
    assert!(movies.advance_scene(67, &mut scene).is_err());
    assert_eq!(movies.scene_player(0).unwrap().playback, old);
    assert_eq!(movies.scene_player(0).unwrap().frame, 0);
}
#[test]
fn scene_start_checks_segments_and_arguments_but_an_empty_slot_is_a_noop() {
    for args in [vec![0, 16, 0, 3, 0], vec![3, 0, 0, 3, 0], vec![0, 0, 0, 3]] {
        let mut code = vec![];
        scene_start(&mut code, &args);
        let (script, mut files) = scene_play_fixture(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut movies = MovieHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        pending(&mut vm);
        movies
            .try_create_scene(&mut vm, &mut scene, &mut files)
            .unwrap();
        pending(&mut vm);
        let old = movies.scene_player(0).unwrap().playback.clone();
        assert!(movies.try_start_scene(&mut vm).is_err());
        assert_eq!(movies.scene_player(0).unwrap().playback, old);
        assert_eq!(vm.stack().len_bytes(), args.len() * 4);
    }
    let mut code = vec![];
    scene_start(&mut code, &[2, u32::MAX, 0, u32::MAX, 0]);
    let script = program(&code, b"");
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    assert!(MovieHost::default().try_start_scene(&mut vm).unwrap());
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn scene_frame_decode_failure_keeps_the_previous_pixels_frame_and_playback_for_retry() {
    let mut code = vec![];
    scene_start(&mut code, &[0, 0, 0, 4, 0]);
    let (script, mut files) = scene_play_fixture(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut movies = MovieHost::default();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    pending(&mut vm);
    movies
        .try_create_scene(&mut vm, &mut scene, &mut files)
        .unwrap();
    pending(&mut vm);
    movies.try_start_scene(&mut vm).unwrap();
    movies.advance_scene(200, &mut scene).unwrap();
    let old = scene.objects().clone();
    let playback = movies.scene_player(0).unwrap().playback.clone();
    assert_eq!(movies.scene_player(0).unwrap().frame, 3);
    assert!(movies.advance_scene(67, &mut scene).is_err());
    assert_eq!(movies.scene_player(0).unwrap().playback, playback);
    assert_eq!(scene.objects(), &old);
    assert_eq!(movies.scene_player(0).unwrap().frame, 3);
    assert!(movies.advance_scene(0, &mut scene).is_err());
    assert_eq!(movies.scene_player(0).unwrap().playback, playback);
}

#[test]
fn cmv6_scene_creation_and_segment_playback_share_the_existing_ownership_rules() {
    for bitmap_header in [false, true] {
        let mut code = create_code(2, 28, 1);
        scene_start(&mut code, &[2, 0, 0, u32::MAX, 1]);
        code.extend(destroy(&[2]));
        let script = program(&code, b"synthetic.cmv\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut movies = MovieHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        let mut files = SceneFiles(Some(slot_fixture::cmv6(4, bitmap_header)));
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 123).unwrap();
        movies
            .try_create_scene(&mut vm, &mut scene, &mut files)
            .unwrap();
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
        let p = movies.scene_player(2).unwrap();
        assert!(matches!(p.index, cmvs_rs::movie::SceneMovieIndex::Cmv6(_)));
        assert!(!p.playback.active);
        assert_eq!(p.frame, 1);
        assert!(
            scene
                .object(28)
                .unwrap()
                .image
                .as_ref()
                .unwrap()
                .pixels
                .pixels()
                .chunks_exact(4)
                .all(|p| p == [136, 136, 136, 255])
        );
        pending(&mut vm);
        movies.try_start_scene(&mut vm).unwrap();
        for target in [2, 3, 0, 1] {
            movies.advance_scene(67, &mut scene).unwrap();
            assert_eq!(movies.scene_player(2).unwrap().frame, target);
            let c = 128 + target as u8 * 8;
            assert!(
                scene
                    .object(28)
                    .unwrap()
                    .image
                    .as_ref()
                    .unwrap()
                    .pixels
                    .pixels()
                    .chunks_exact(4)
                    .all(|p| p == [c, c, c, 255])
            );
        }
        let old = scene.objects().clone();
        pending(&mut vm);
        movies.try_handle(&mut vm).unwrap();
        assert!(!movies.occupied(2));
        assert_eq!(scene.objects(), &old);
    }
}

#[test]
fn cmv6_scene_bad_frames_metadata_and_real_soundtracks_preserve_the_old_player() {
    let good = slot_fixture::cmv6(4, true);
    let mut bad_size = good.clone();
    bad_size[52..56].copy_from_slice(&769u32.to_le_bytes());
    let mut bad_dimensions = good.clone();
    bad_dimensions[28..32].copy_from_slice(&32u32.to_le_bytes());
    let mut audio = good.clone();
    audio.extend(b"Ogg");
    let len = audio.len() as u32;
    audio[8..12].copy_from_slice(&len.to_le_bytes());
    audio[128..132].copy_from_slice(&3u32.to_le_bytes());
    for (data, frame) in [(bad_size, 0), (bad_dimensions, 0), (audio, 0), (good, 4)] {
        let mut code = create_code(0, 28, 0);
        code.extend(create_code(0, 29, frame));
        let script = program(&code, b"synthetic.cmv\0");
        let mut vm = Machine::new(&script).unwrap();
        let mut movies = MovieHost::default();
        let mut scene = cmvs_rs::scene::SceneHost::default();
        pending(&mut vm);
        movies
            .try_create_scene(
                &mut vm,
                &mut scene,
                &mut SceneFiles(Some(slot_fixture::movie(false))),
            )
            .unwrap();
        let old = scene.objects().clone();
        let encoded = movies.scene_player(0).unwrap().encoded().to_vec();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 456).unwrap();
        let pc = vm.pc();
        let sp = vm.stack().len_bytes();
        assert!(
            movies
                .try_create_scene(&mut vm, &mut scene, &mut SceneFiles(Some(data)))
                .is_err()
        );
        assert_eq!(vm.pending_system(), Some((pc, 0x308)));
        assert_eq!(vm.stack().len_bytes(), sp);
        assert_eq!(vm.state().system_integer(0), Some(456));
        assert_eq!(scene.objects(), &old);
        assert_eq!(movies.scene_player(0).unwrap().encoded(), encoded);
    }
}

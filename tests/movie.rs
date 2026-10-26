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
fn destroy(slots: &[u32]) -> Vec<u8> {
    let mut code = vec![];
    for &slot in slots {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(slot.to_le_bytes());
        code.extend(0x230au16.to_le_bytes());
    }
    code
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
    for code in [
        destroy(&[3]),
        destroy(&[u32::MAX]),
        0x230au16.to_le_bytes().to_vec(),
    ] {
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let (pc, bytes) = (vm.pc(), vm.stack().len_bytes());
        assert!(MovieHost::default().try_handle(&mut vm).is_err());
        assert_eq!(vm.pending_system(), Some((pc, 0x30a)));
        assert_eq!(vm.stack().len_bytes(), bytes);
    }
}

#[test]
fn creation_commands_are_left_pending() {
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

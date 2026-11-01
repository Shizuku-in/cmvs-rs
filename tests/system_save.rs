#[path = "support/shared_storage.rs"]
mod shared_storage;
use cmvs_rs::{
    MAX_PAYLOAD,
    evaluate::State,
    machine::{Machine, Step},
    ps3::Script,
    system_save::{
        Css1Layout, Css1Parameters, Css1Save, FileSystemSave, NewSystemSave, SystemSaveEvent,
        SystemSaveHost, SystemSaveSink, SystemSaveSource,
    },
};
use shared_storage::SharedSystem;
use std::{fs, io, process::Command};

#[test]
fn encoder_preserves_persistent_words_lists_and_all_rotation_modes() {
    for mixed in [false, true] {
        let mut payload = body();
        let mut seed = 19u32;
        for (i, byte) in payload[..44800].iter_mut().enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = if mixed && i % 1100 < 750 {
                (seed >> 24) as u8
            } else {
                0
            };
        }
        let template = Css1Save::decode(&fixture(&payload, [0, 0])).unwrap();
        let state = template
            .applied(&State::default(), Some(0x12345678))
            .unwrap();
        let lists = [vec![b"a".to_vec(), vec![]], vec![vec![0x81, 0xff]]];
        for rotation in 2..=5 {
            let parameters = Css1Parameters {
                byte_add: 7,
                key_add: 161,
                rotation_word: ((rotation - 2) << 3) as u16,
            };
            let raw = template
                .encode_state(&state, 0x12345678, parameters, [&lists[0], &lists[1]])
                .unwrap();
            let decoded = Css1Save::decode(&raw).unwrap();
            let mut expected = payload.clone();
            expected.extend(b"a\0\0\x81\xff\0");
            assert_eq!(decoded.payload(), expected);
            assert_eq!(decoded.lists(), &lists);
            assert_eq!(decoded.layout().list_counts, [2, 1]);
            assert!(decoded.layout().packed_size < payload.len());
        }
    }
}

#[test]
fn encoder_never_fills_unknown_persistent_state_or_accepts_embedded_list_nuls() {
    let template = Css1Save::decode(&fixture(&body(), [0, 0])).unwrap();
    let p = Css1Parameters {
        byte_add: 0,
        key_add: 128,
        rotation_word: 0,
    };
    let mut state = State::default();
    assert!(template.encode_state(&state, 1, p, [&[], &[]]).is_err());
    state.fill_bits(2048, 260096, false).unwrap();
    assert!(template.encode_state(&state, 1, p, [&[], &[]]).is_err());
    state.fill_integers(2048, 2048, 0).unwrap();
    assert!(template.encode_state(&state, 1, p, [&[], &[]]).is_err());
    state.fill_floats(1024, 1024, 0).unwrap();
    assert!(template.encode_state(&state, 1, p, [&[], &[]]).is_err());
    state.fill_strings(64, 64, b"").unwrap();
    assert!(template.encode_state(&state, 1, p, [&[], &[]]).is_ok());
    assert!(
        template
            .encode_state(&state, 1, p, [&[b"a\0b".to_vec()], &[]])
            .is_err()
    );
}

#[test]
fn writer_failure_keeps_rng_vm_and_template_then_commits_exactly_four_draws() {
    use cmvs_rs::random::CrtRandom;
    #[derive(Default)]
    struct Sink {
        fail: bool,
        raw: Vec<u8>,
    }
    impl SystemSaveSink for Sink {
        fn write(&mut self, raw: &[u8]) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("synthetic sink failure"));
            }
            self.raw = raw.to_vec();
            Ok(())
        }
    }
    let template = Css1Save::decode(&fixture(&body(), [0, 0])).unwrap();
    let code = 0x2129u16.to_le_bytes();
    let mut raw = b"PS2A".to_vec();
    for v in [48u32, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0] {
        raw.extend(v.to_le_bytes());
    }
    raw.extend(code.map(|b| b.rotate_left(1).wrapping_add(124)));
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    *vm.state_mut() = template.applied(vm.state(), Some(0x12345678)).unwrap();
    vm.state_mut().set_system_integer(0, 77).unwrap();
    vm.step().unwrap();
    let pending = vm.pending_system();
    let mut host = SystemSaveHost::default();
    let mut sink = Sink {
        fail: true,
        ..Sink::default()
    };
    let mut random = CrtRandom::new(19);
    let before = random.clone();
    assert!(
        host.shared_write(&mut vm, &mut sink, &mut random, Some(0x12345678))
            .is_err()
    );
    host.loaded = Some(template);
    assert!(
        host.shared_write(&mut vm, &mut sink, &mut random, Some(1))
            .is_err()
    );
    sink.fail = true;
    assert!(
        host.shared_write(&mut vm, &mut sink, &mut random, Some(0x12345678))
            .is_err()
    );
    assert_eq!(random, before);
    assert_eq!(vm.pending_system(), pending);
    assert!(sink.raw.is_empty());
    sink.fail = false;
    host.shared_write(&mut vm, &mut sink, &mut random, Some(0x12345678))
        .unwrap();
    let mut expected = before;
    for _ in 0..4 {
        expected.next_value();
    }
    assert_eq!(random, expected);
    // Independent seed-19 CRT vector: 100, 15331, 209, 888.
    assert_eq!(&sink.raw[78..84], &[100, 0, 227, 0, 75, 108]);
    assert_eq!(vm.pending_system(), None);
    assert_eq!(vm.pc(), 2);
    assert_eq!(vm.state().system_integer(0), Some(77));
    assert_eq!(Css1Save::decode(&sink.raw).unwrap().payload(), body());
    // A first save has no loaded template. Failed output still consumes no RNG;
    // a successful retry writes the complete persistent state with our annotation.
    let mut fresh = SystemSaveHost::default();
    let before = random.clone();
    sink.fail = true;
    assert!(
        fresh
            .write_now(vm.state(), &mut sink, &mut random, Some(0x12345678))
            .is_err()
    );
    assert_eq!(random, before);
    sink.fail = false;
    fresh
        .write_now(vm.state(), &mut sink, &mut random, Some(0x12345678))
        .unwrap();
    assert!(fresh.loaded.is_none());
    assert_eq!(&sink.raw[8..15], b"cmvs-rs");
    assert_eq!(Css1Save::decode(&sink.raw).unwrap().payload(), body());
}

#[test]
fn new_output_rejects_game_paths_existing_files_and_creation_races() {
    let root = std::env::temp_dir().join(format!("cmvs-css1-output-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let game = root.join("game");
    fs::create_dir(&game).unwrap();
    assert!(NewSystemSave::new(&game.join("system.dat"), &game).is_err());
    let path = root.join("system.dat");
    let mut output = NewSystemSave::new(&path, &game).unwrap();
    output.write(b"first").unwrap();
    assert!(output.write(b"second").is_err());
    assert!(NewSystemSave::new(&path, &game).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"first");
    let race = root.join("race.dat");
    let mut output = NewSystemSave::new(&race, &game).unwrap();
    fs::write(&race, b"other writer").unwrap();
    assert!(output.write(b"replacement").is_err());
    assert_eq!(fs::read(&race).unwrap(), b"other writer");
    fs::remove_file(path).unwrap();
    fs::remove_file(race).unwrap();
    fs::remove_dir(game).unwrap();
    fs::remove_dir(root).unwrap();
}

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn body() -> Vec<u8> {
    vec![0; 44864]
}
fn literals(b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for chunk in b.chunks(8) {
        out.push(((1u16 << chunk.len()) - 1) as u8);
        out.extend(chunk);
    }
    out
}
fn wrap(payload: &[u8], packed: &[u8], rotation: u32, counts: [u32; 2]) -> Vec<u8> {
    let mut raw = vec![0; 360];
    raw[..4].copy_from_slice(b"CSS1");
    put(&mut raw, 4, 104);
    put(&mut raw, 68, 0x12345678);
    raw[76] = 1;
    raw[78] = 7;
    raw[80] = 161;
    raw[82..84].copy_from_slice(&(((rotation - 2) << 3) as u16).to_le_bytes());
    put(&mut raw, 84, counts[1]);
    put(&mut raw, 88, counts[0]);
    put(&mut raw, 92, payload.len() as u32);
    put(&mut raw, 96, packed.len() as u32);
    put(&mut raw, 100, 360 + packed.len() as u32);
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, b) in packed.iter().enumerate() {
        raw.push(
            ((b.wrapping_add(64) ^ 161u8.wrapping_add(key[i % key.len()])).wrapping_add(7))
                .rotate_left(rotation),
        );
    }
    let sum = raw[360..]
        .iter()
        .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
    raw.extend(sum.to_le_bytes());
    raw
}
fn fixture(payload: &[u8], counts: [u32; 2]) -> Vec<u8> {
    wrap(payload, &literals(payload), 4, counts)
}

#[test]
fn all_rotation_modes_decode_and_raw_bytes_are_preserved() {
    let mut p = vec![0; 44800];
    p.extend(b"\x81\xff");
    p.extend([0; 64]);
    for rotation in 2..=5 {
        let raw = wrap(&p, &literals(&p), rotation, [0, 0]);
        let save = Css1Save::decode(&raw).unwrap();
        assert_eq!(save.payload(), p);
        assert_eq!(save.strings()[0], [0x81, 0xff]);
        assert!(save.lists().iter().all(Vec::is_empty));
        assert_eq!(
            Css1Layout::parse(&raw[..360], raw.len() as u64).unwrap(),
            *save.layout()
        );
    }
}

#[test]
fn overlapping_references_maximum_length_and_ring_wrap_decode() {
    // One A literal followed by references to offset 976, including self overlap.
    let mut tokens = vec![(true, vec![b'A'])];
    let mut count = 44799;
    while count > 0 {
        let n = count.min(65);
        if n == 1 {
            tokens.push((true, vec![b'A']));
        } else {
            tokens.push((false, vec![0xd0, 0xc0 | (n - 2) as u8]));
        }
        count -= n;
    }
    for _ in 0..64 {
        tokens.push((true, vec![0]));
    }
    let mut packed = vec![];
    for group in tokens.chunks(8) {
        packed.push(
            group
                .iter()
                .enumerate()
                .fold(0u8, |flags, (i, (literal, _))| {
                    flags | (u8::from(*literal) << i)
                }),
        );
        for (_, bytes) in group {
            packed.extend(bytes);
        }
    }
    let mut p = vec![b'A'; 44800];
    p.extend([0; 64]);
    let save = Css1Save::decode(&wrap(&p, &packed, 3, [0, 0])).unwrap();
    assert_eq!(save.payload(), p);
}

#[test]
fn every_truncated_prefix_and_trailing_file_byte_is_rejected() {
    let raw = fixture(&body(), [0, 0]);
    for end in 0..raw.len() {
        assert!(Css1Save::decode(&raw[..end]).is_err(), "end={end}");
    }
    let mut extra = raw;
    extra.push(0);
    assert!(Css1Save::decode(&extra).is_err());
}

#[test]
fn malformed_headers_lengths_counts_and_checksum_are_rejected() {
    let raw = fixture(&body(), [0, 0]);
    for (at, value) in [
        (0, u32::from_le_bytes(*b"CSS2")),
        (4, 360),
        (76, 0),
        (84, 4097),
        (92, 44800),
        (92, u32::MAX),
        (96, u32::MAX),
        (100, u32::MAX),
    ] {
        let mut bad = raw.clone();
        put(&mut bad, at, value);
        assert!(Css1Save::decode(&bad).is_err(), "offset={at}");
    }
    assert!(Css1Layout::parse(&raw[..360], MAX_PAYLOAD as u64 + 1).is_err());
    let mut bad = raw;
    bad[361] ^= 1;
    assert_eq!(
        Css1Save::decode(&bad).unwrap_err().0,
        "CSS1 payload checksum mismatch"
    );
}

#[test]
fn truncated_lzss_and_wrong_declared_size_fail_after_valid_checksum() {
    for packed in [&[1][..], &[0, 0][..]] {
        assert!(Css1Save::decode(&wrap(&body(), packed, 4, [0, 0])).is_err());
    }
    let mut raw = fixture(&body(), [0, 0]);
    put(&mut raw, 92, 44865);
    assert_eq!(
        Css1Save::decode(&raw).unwrap_err().0,
        "CSS1 LZSS decoded length mismatch"
    );
    let mut p = body();
    p.push(0);
    let mut raw = fixture(&p, [0, 0]);
    put(&mut raw, 92, 44864);
    assert_eq!(
        Css1Save::decode(&raw).unwrap_err().0,
        "CSS1 LZSS output exceeds declared size"
    );
}

#[test]
fn strings_lists_and_trailing_payload_have_explicit_bounds() {
    let mut p = vec![0; 44800];
    p.extend(vec![b'x'; 1024]);
    p.extend([0; 64]);
    assert!(Css1Save::decode(&fixture(&p, [0, 0])).is_err());
    let mut p = body();
    p[44863] = b'x';
    assert!(Css1Save::decode(&fixture(&p, [0, 0])).is_err());
    let mut p = body();
    p.extend(b"first\0\x81second\0third\0");
    let save = Css1Save::decode(&fixture(&p, [2, 1])).unwrap();
    assert_eq!(save.lists()[0], [b"first".to_vec(), b"\x81second".to_vec()]);
    assert_eq!(save.lists()[1], [b"third".to_vec()]);
    assert!(Css1Save::decode(&fixture(&p, [1, 1])).is_err());
    assert!(Css1Save::decode(&fixture(&p, [3, 1])).is_err());
}

#[test]
fn application_restores_exact_ranges_preserves_registers_and_checks_game_id() {
    let mut p = vec![0; 44800];
    p[0] = 0x81;
    p[32511] = 0x80;
    put(&mut p, 32512, u32::MAX);
    put(&mut p, 40700, 42);
    put(&mut p, 40704, 0x7fc01234);
    put(&mut p, 44796, 0x80000000);
    p.extend(b"\x81text");
    p.extend([0; 64]);
    let save = Css1Save::decode(&fixture(&p, [0, 0])).unwrap();
    let mut state = State::default();
    state.set_bit(2047, true).unwrap();
    state.set_integer(2047, 12).unwrap();
    state.fill_floats(1023, 1, 11).unwrap();
    state.fill_strings(63, 1, b"before").unwrap();
    state.set_integer(4096, 8).unwrap();
    state.set_system_integer(0, 4).unwrap();
    state.result = Some(8);
    state.condition = Some(true);
    assert!(save.applied(&state, Some(1)).is_err());
    assert!(save.applied(&state, None).is_err());
    assert_eq!(state.integer(2048), None);
    let new = save.applied(&state, Some(0x12345678)).unwrap();
    assert_eq!(new.bit(2047), Some(true));
    assert_eq!(new.bit(2048), Some(true));
    assert_eq!(new.bit(2049), Some(false));
    assert_eq!(new.bit(2055), Some(true));
    assert_eq!(new.bit(262143), Some(true));
    assert_eq!(new.integer(2047), Some(12));
    assert_eq!(new.integer(2048), Some(-1));
    assert_eq!(new.integer(4095), Some(42));
    assert_eq!(new.integer(4096), Some(8));
    assert_eq!(new.float_bits(1023), Some(11));
    assert_eq!(new.float_bits(1024), Some(0x7fc01234));
    assert_eq!(new.float_bits(2047), Some(0x80000000));
    assert_eq!(new.string(63), Some(b"before".as_slice()));
    assert_eq!(new.string(64), Some(b"\x81text".as_slice()));
    assert_eq!(new.system_integer(0), Some(4));
    assert_eq!(new.result, Some(8));
    assert_eq!(new.condition, Some(true));
    assert!(
        save.applied(&State::new(8192, 65536), Some(0x12345678))
            .is_err()
    );
    let mut raw = fixture(&p, [0, 0]);
    put(&mut raw, 68, 0);
    assert!(
        Css1Save::decode(&raw)
            .unwrap()
            .applied(&state, None)
            .is_ok()
    );
}

#[derive(Default)]
struct Source {
    raw: Option<Vec<u8>>,
    fail: bool,
    calls: usize,
}
impl SystemSaveSource for Source {
    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        self.calls += 1;
        if self.fail {
            return Err(io::Error::other("synthetic I/O failure"));
        }
        Ok(self.raw.clone())
    }
}
fn script() -> Script {
    let code = [0x28, 0x21, 0x28, 0x21, 0x99, 0x20];
    let mut raw = b"PS2A".to_vec();
    for v in [48u32, 0, 0, 0, 6, 0, 0, 0, 0, 6, 0] {
        raw.extend(v.to_le_bytes());
    }
    raw.extend(code.map(|b: u8| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}

#[test]
fn host_io_format_and_identifier_errors_leave_vm_and_lists_for_retry() {
    let script = script();
    let mut vm = Machine::new(&script).unwrap();
    let mut source = Source::default();
    let mut host = SystemSaveHost::default();
    assert_eq!(host.shared_read(&mut vm, &mut source, None).unwrap(), None);
    assert_eq!(source.calls, 0);
    assert!(matches!(
        vm.step().unwrap(),
        Step::System { command: 0x128, .. }
    ));
    source.fail = true;
    assert!(host.shared_read(&mut vm, &mut source, None).is_err());
    source.fail = false;
    source.raw = Some(vec![0; 360]);
    assert!(host.shared_read(&mut vm, &mut source, None).is_err());
    source.raw = Some(fixture(&body(), [0, 0]));
    assert!(host.shared_read(&mut vm, &mut source, Some(1)).is_err());
    assert_eq!(vm.pc(), 0);
    assert_eq!(vm.state().integer(2048), None);
    assert!(host.loaded.is_none());
    assert_eq!(
        host.shared_read(&mut vm, &mut source, Some(0x12345678))
            .unwrap(),
        Some(SystemSaveEvent::Applied)
    );
    assert_eq!(vm.pc(), 2);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().integer(2048), Some(0));
}

#[test]
fn missing_source_keeps_existing_state_and_zero_counts_keep_existing_lists() {
    let script = script();
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_integer(2048, 123).unwrap();
    let mut host = SystemSaveHost::default();
    let mut source = Source::default();
    vm.step().unwrap();
    assert_eq!(
        host.shared_read(&mut vm, &mut source, None).unwrap(),
        Some(SystemSaveEvent::Missing)
    );
    assert_eq!(vm.state().integer(2048), Some(123));
    host.lists[0] = Some(vec![b"old".to_vec()]);
    source.raw = Some(fixture(&body(), [0, 0]));
    vm.step().unwrap();
    host.shared_read(&mut vm, &mut source, Some(0x12345678))
        .unwrap();
    assert_eq!(host.lists[0], Some(vec![b"old".to_vec()]));
    assert_eq!(host.lists[1], None);
    vm.step().unwrap();
    let calls = source.calls;
    assert_eq!(host.shared_read(&mut vm, &mut source, None).unwrap(), None);
    assert_eq!(source.calls, calls);
}

#[test]
fn nonempty_list_replaces_only_its_corresponding_host_list() {
    let script = script();
    let mut vm = Machine::new(&script).unwrap();
    vm.step().unwrap();
    let mut p = body();
    p.extend(b"new\0");
    let mut source = Source {
        raw: Some(fixture(&p, [0, 1])),
        ..Source::default()
    };
    let mut host = SystemSaveHost {
        lists: [Some(vec![b"kept".to_vec()]), Some(vec![b"old".to_vec()])],
        ..SystemSaveHost::default()
    };
    host.shared_read(&mut vm, &mut source, Some(0x12345678))
        .unwrap();
    assert_eq!(
        host.lists,
        [Some(vec![b"kept".to_vec()]), Some(vec![b"new".to_vec()])]
    );
}

#[test]
fn file_source_and_cli_remain_read_only_and_never_overwrite_output() {
    let root = std::env::temp_dir().join(format!("cmvs-css1-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("synthetic.dat");
    let output = root.join("decoded.bin");
    let raw = fixture(&body(), [0, 0]);
    fs::write(&path, &raw).unwrap();
    assert_eq!(
        FileSystemSave::new(&path).read().unwrap(),
        Some(raw.clone())
    );
    assert_eq!(
        FileSystemSave::new(root.join("absent")).read().unwrap(),
        None
    );
    let exe = env!("CARGO_BIN_EXE_cmvs-system-save");
    let run = |target: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--payload")
            .arg(target)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(fs::read(&output).unwrap(), body());
    assert!(!run(&output).status.success());
    fs::remove_file(&output).unwrap();
    let mut bad = raw;
    bad[361] ^= 1;
    fs::write(&path, bad).unwrap();
    assert!(!run(&output).status.success());
    assert!(!output.exists());
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}

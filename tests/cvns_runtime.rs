//! CVNS runtime pieces with synthetic data: CSS0, the save-menu scan, the 256-slot string
//! bank and the translated common-script entry.
#[path = "support/shared_storage.rs"]
mod shared_storage;
use cmvs_rs::{
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

#[path = "support/shared_storage.rs"]
mod shared_storage;
use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
    save_menu::{DirectorySaves, SaveHeader, SaveMenu, SaveMenuHost, SaveSource, SlotStatus},
};
use shared_storage::SharedSaveMenu;
use std::{collections::BTreeMap, io};

fn header(version: u8, label: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 16];
    bytes[..3].copy_from_slice(b"CSV");
    bytes[3] = version;
    bytes.extend(label);
    bytes.push(0);
    bytes
}

#[derive(Default)]
struct MemorySaves {
    files: BTreeMap<u16, Vec<u8>>,
    calls: Vec<u16>,
    fail_at: Option<u16>,
}
impl SaveSource for MemorySaves {
    fn read_header(&mut self, slot: u16) -> io::Result<Option<Vec<u8>>> {
        self.calls.push(slot);
        if self.fail_at == Some(slot) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic failure",
            ));
        }
        Ok(self.files.get(&slot).cloned())
    }
}

#[test]
fn accepts_menu_versions_and_preserves_non_utf8_label() {
    for version in b'2'..=b'9' {
        let label = b"2026-10-26 09:08:07 \x81\xff";
        let parsed = SaveHeader::parse(&header(version, label)).unwrap();
        assert_eq!(parsed.version, version);
        assert_eq!(parsed.label, label);
        assert_eq!(parsed.ordering_key, 0x355a4903);
    }
    for bytes in [
        b"CSS1".to_vec(),
        header(b'1', b"2026-10-26 09:08:07"),
        header(b':', b"2026-10-26 09:08:07"),
    ] {
        assert!(SaveHeader::parse(&bytes).is_err());
    }
}

#[test]
fn malformed_labels_and_read_boundary_are_rejected() {
    let valid = header(b'2', b"2026-10-26 09:08:07 title");
    for end in 0..valid.len() {
        assert!(SaveHeader::parse(&valid[..end]).is_err());
    }
    for label in [b"2026/10/26 09:08:07".as_slice(), b"2026-10-26 0x:08:07"] {
        assert!(SaveHeader::parse(&header(b'2', label)).is_err());
    }
    let mut oversized = header(b'2', b"2026-10-26 09:08:07");
    oversized.pop();
    oversized.resize(512, b'x');
    oversized.push(0); // A terminator outside the original 512-byte read is unavailable.
    assert!(SaveHeader::parse(&oversized).is_err());
    oversized[511] = 0;
    assert!(SaveHeader::parse(&oversized).is_ok());
}

#[test]
fn latest_uses_embedded_key_keeps_first_tie_and_wraps_century() {
    let mut source = MemorySaves::default();
    for (slot, date) in [
        (0, b"2026-10-26 09:08:06".as_slice()),
        (1, b"2026-10-26 09:08:07"), // same key, two-second precision
        (2, b"2026-10-26 09:08:08"),
        (3, b"2126-10-26 09:08:09"), // century ignored
    ] {
        source.files.insert(slot, header(b'2', date));
    }
    source.files.insert(4, b"bad".to_vec());
    let menu = SaveMenu::scan(&mut source, 6).unwrap();
    assert_eq!(menu.latest, Some(2));
    assert!(matches!(menu.slots[4], SlotStatus::Invalid(_)));
    assert_eq!(menu.slots[5], SlotStatus::Missing);
}

#[test]
fn numeric_prefix_is_not_calendar_validation_and_zero_key_is_not_latest() {
    let mut source = MemorySaves::default();
    source.files.insert(0, header(b'2', b"2000-00-00 00:00:00"));
    let menu = SaveMenu::scan(&mut source, 1).unwrap();
    assert!(matches!(menu.slots[0], SlotStatus::Recognized(_)));
    assert_eq!(menu.latest, None);
}

#[test]
fn slot_count_clamps_and_empty_scan_does_not_read() {
    for (request, count) in [
        (i32::MIN, 10),
        (-1, 10),
        (0, 0),
        (1, 1),
        (256, 256),
        (i32::MAX, 256),
    ] {
        let mut source = MemorySaves::default();
        let menu = SaveMenu::scan(&mut source, request).unwrap();
        assert_eq!(menu.slots.len(), count);
        assert_eq!(source.calls.len(), count);
        assert_eq!(menu.latest, None);
    }
}

fn script(commands: &[(u16, u32)]) -> Script {
    let mut code = Vec::new();
    for (command, value) in commands {
        code.extend(0x0442u16.to_le_bytes()); // immediate push, four-byte stride
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
        code.extend(command.to_le_bytes());
    }
    code.extend(0x201eu16.to_le_bytes()); // valid command outside the save-menu host
    let mut raw = b"PS2A".to_vec();
    for v in [
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
        raw.extend(v.to_le_bytes());
    }
    raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}

fn pending(vm: &mut Machine<'_>) {
    loop {
        if matches!(vm.step().unwrap(), Step::System { .. }) {
            break;
        }
    }
}

#[test]
fn commands_consume_parameters_toggle_and_preserve_unknown_commands() {
    let script = script(&[(0x22b3, 2), (0x22b4, 0), (0x22b4, 9)]);
    let mut vm = Machine::new(&script).unwrap();
    let mut source = MemorySaves::default();
    let mut host = SaveMenuHost::default();
    assert!(!host.shared_menu(&mut vm, &mut source).unwrap());
    for enabled in [None, Some(false), Some(true)] {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(host.shared_menu(&mut vm, &mut source).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(host.second_group_enabled(), enabled);
    }
    assert_eq!(host.menu().unwrap().slots.len(), 2);
    assert_eq!(source.calls, [0, 1]); // toggles perform no I/O
    pending(&mut vm);
    let before = vm.pending_system();
    assert!(!host.shared_menu(&mut vm, &mut source).unwrap());
    assert_eq!(vm.pending_system(), before);
}

#[test]
fn io_failure_keeps_previous_menu_and_pending_parameters_for_retry() {
    let script = script(&[(0x22b3, 1), (0x22b3, 2)]);
    let mut vm = Machine::new(&script).unwrap();
    let mut source = MemorySaves::default();
    let mut host = SaveMenuHost::default();
    pending(&mut vm);
    host.shared_menu(&mut vm, &mut source).unwrap();
    let old = host.menu().unwrap().clone();
    pending(&mut vm);
    let before = vm.pending_system();
    source.fail_at = Some(1);
    assert!(host.shared_menu(&mut vm, &mut source).is_err());
    assert_eq!(host.menu(), Some(&old));
    assert_eq!(vm.pending_system(), before);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 2);
    source.fail_at = None;
    assert!(host.shared_menu(&mut vm, &mut source).unwrap());
    assert_eq!(host.menu().unwrap().slots.len(), 2);
}

#[test]
fn missing_argument_does_not_read_source() {
    // Create the header with the helper, then reuse an isolated system-only entry.
    let script = script(&[(0x22b3, 1)]);
    let mut vm = Machine::at(&script, 8).unwrap();
    let mut source = MemorySaves::default();
    let mut host = SaveMenuHost::default();
    pending(&mut vm);
    assert!(host.shared_menu(&mut vm, &mut source).is_err());
    assert!(source.calls.is_empty());
    assert_eq!(vm.pc(), 8);
    assert!(host.menu().is_none());
}

#[test]
fn signature_query_accepts_versions_without_menu_labels_and_preserves_menu_state() {
    let mut source = MemorySaves::default();
    let mut inputs = vec![];
    for version in b'2'..=b'9' {
        inputs.push((vec![b'C', b'S', b'V', version], true));
    }
    for invalid in [
        b"".as_slice(),
        b"C",
        b"CS",
        b"CSV",
        b"CSV1",
        b"CSV:",
        b"CSS2",
        b"csv2",
    ] {
        inputs.push((invalid.to_vec(), false));
    }
    inputs.push((b"CSV2no valid label or payload".to_vec(), true));
    // A short read after a valid one cannot reuse the earlier signature bytes.
    inputs.push((b"CS".to_vec(), false));
    let mut commands = vec![(0x22b3, 1), (0x22b4, 1)];
    for (i, (bytes, _)) in inputs.iter().enumerate() {
        let slot = 870 + i as u16;
        source.files.insert(slot, bytes.clone());
        commands.push((0x22bb, u32::from(slot)));
    }
    commands.extend([(0x22bb, 999), (0x22bb, 1000), (0x22bb, u32::MAX)]);
    let program = script(&commands);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SaveMenuHost::default();
    for _ in 0..2 {
        pending(&mut vm);
        host.shared_menu(&mut vm, &mut source).unwrap();
    }
    let menu = host.menu().unwrap().clone();
    source.calls.clear();
    vm.state_mut().set_system_integer(4, 0xfeed).unwrap();
    for expected in inputs
        .iter()
        .map(|(_, ok)| *ok)
        .chain([false, false, false])
    {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(host.shared_menu(&mut vm, &mut source).unwrap());
        assert_eq!(vm.state().system_integer(0), Some(u32::from(expected)));
        assert_eq!(vm.state().system_integer(4), Some(0xfeed));
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(host.menu(), Some(&menu));
        assert_eq!(host.second_group_enabled(), Some(true));
        assert!(host.popup_enabled());
    }
    let expected_calls: Vec<_> = (870..870 + inputs.len() as u16).chain([999]).collect();
    assert_eq!(source.calls, expected_calls);
}

#[test]
fn signature_query_io_failure_and_missing_parameter_are_transactional() {
    let program = script(&[(0x22bb, 870)]);
    let mut vm = Machine::new(&program).unwrap();
    let mut source = MemorySaves {
        fail_at: Some(870),
        ..MemorySaves::default()
    };
    let mut host = SaveMenuHost::default();
    vm.state_mut().set_system_integer(0, 9).unwrap();
    pending(&mut vm);
    let before = vm.pending_system();
    assert!(host.shared_menu(&mut vm, &mut source).is_err());
    assert_eq!(vm.pending_system(), before);
    assert_eq!(vm.pc(), 8);
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 870);
    assert_eq!(vm.state().system_integer(0), Some(9));
    assert!(host.menu().is_none());
    source.fail_at = None;
    source.files.insert(870, b"CSV2".to_vec());
    assert!(host.shared_menu(&mut vm, &mut source).unwrap());
    assert_eq!(vm.state().system_integer(0), Some(1));
    assert_eq!(vm.stack().len_bytes(), 0);

    let mut vm = Machine::at(&program, 8).unwrap();
    pending(&mut vm);
    source.calls.clear();
    assert!(host.shared_menu(&mut vm, &mut source).is_err());
    assert_eq!(vm.pending_system(), Some((8, 0x2bb)));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert!(source.calls.is_empty());
}

#[test]
fn directory_reads_only_selected_slot_prefix_and_never_modifies_files() {
    let root = std::env::temp_dir().join(format!("cmvs-save-menu-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut data = header(b'2', b"2026-10-26 09:08:07 synthetic");
    data.resize(1024, 0xa5);
    let path = root.join("save000.dat");
    std::fs::write(&path, &data).unwrap();
    std::fs::write(root.join("system.dat"), b"CSS1 synthetic").unwrap();
    let mut source = DirectorySaves::new(&root).unwrap();
    assert_eq!(source.read_header(0).unwrap().unwrap(), data[..512]);
    assert_eq!(source.read_header(1).unwrap(), None);
    for slot in [256, 870, 999] {
        let extra = root.join(format!("save{slot:03}.dat"));
        std::fs::write(&extra, &data).unwrap();
        assert_eq!(source.read_header(slot).unwrap().unwrap(), data[..512]);
        assert_eq!(std::fs::read(&extra).unwrap(), data);
        std::fs::remove_file(extra).unwrap();
    }
    assert!(source.read_header(1000).is_err());
    assert!(source.read_header(u16::MAX).is_err());
    assert!(DirectorySaves::new(&path).is_err());
    let absent = root.join("absent");
    let mut empty = DirectorySaves::new(&absent).unwrap();
    let menu = SaveMenu::scan(&mut empty, 0).unwrap();
    assert!(menu.slots.iter().all(|slot| *slot == SlotStatus::Missing));
    assert_eq!(menu.latest, None);
    assert!(!absent.exists());
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(
        std::fs::read(root.join("system.dat")).unwrap(),
        b"CSS1 synthetic"
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(root.join("system.dat")).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn popup_flag_defaults_enabled_and_is_reported_without_cleanup() {
    let script = script(&[(0x22cd, 0), (0x22ce, 9), (0x22cd, 5), (0x22ce, 9)]);
    let mut vm = Machine::new(&script).unwrap();
    let mut host = SaveMenuHost::default();
    assert!(host.popup_enabled());
    assert!(!host.try_handle_popup(&mut vm).unwrap());
    let mut seen = vec![];
    for _ in 0..4 {
        pending(&mut vm);
        assert!(host.try_handle_popup(&mut vm).unwrap());
        seen.push((
            host.popup_enabled(),
            vm.state().system_integer(0),
            vm.stack().len_bytes(),
        ));
    }
    // 0x22ce returns 0x4000, so its pushed word stays on the stack.
    assert_eq!(
        seen,
        [
            (false, None, 0),
            (false, Some(0), 4),
            (true, Some(0), 4),
            (true, Some(1), 8)
        ]
    );
    pending(&mut vm);
    assert!(!host.try_handle_popup(&mut vm).unwrap());
}

#[test]
fn popup_flag_without_argument_stays_pending() {
    let mut raw = b"PS2A".to_vec();
    for v in [48u32, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0] {
        raw.extend(v.to_le_bytes());
    }
    raw.extend(
        0x22cdu16
            .to_le_bytes()
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    let mut host = SaveMenuHost::default();
    pending(&mut vm);
    assert!(host.try_handle_popup(&mut vm).is_err());
    assert_eq!(vm.pending_system(), Some((0, 0x2cd)));
    assert!(host.popup_enabled());
}

#[test]
fn confirmation_settings_are_set_and_queried_and_read_from_an_ini() {
    use cmvs_rs::save_menu::profile_int;
    let text =
        b"[WINDOW_STATUS]\r\nDLG_SAVE=0\r\n[ system_info ]\r\n dlg_load = -12x\r\nDLG_SAVE=1\r\n";
    assert_eq!(profile_int(text, "SYSTEM_INFO", "DLG_LOAD", 0), -12);
    assert_eq!(profile_int(text, "SYSTEM_INFO", "DLG_SAVE", 0), 1);
    assert_eq!(profile_int(text, "SYSTEM_INFO", "EFFECT_USE", 7), 7);

    // Pushes SP-8 then SP-4 for each command; 0x22ee takes none.
    let mut code = Vec::new();
    for (command, args) in [
        (0x22d4u16, &[0u32, 0][..]),
        (0x22d3, &[1, 0][..]),
        (0x22d3, &[0, 0][..]),
        (0x22ee, &[][..]),
    ] {
        for value in args.iter().rev() {
            code.extend(0x0440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(value.to_le_bytes());
        }
        code.extend(command.to_le_bytes());
    }
    let mut raw = b"PS2A".to_vec();
    for v in [
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
        raw.extend(v.to_le_bytes());
    }
    raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    let mut host = SaveMenuHost::default();
    host.set_confirmations(true, true);
    let step = |vm: &mut Machine<'_>, host: &mut SaveMenuHost| {
        pending(vm);
        assert!(host.try_handle_popup(vm).unwrap());
        vm.state().system_integer(0)
    };
    // 0x22d4 with a zero SP-4 queries DLG_SAVE.
    assert_eq!(step(&mut vm, &mut host), Some(1));
    // 0x22d3 with SP-4 = 1 sets DLG_LOAD to bool(SP-8) without touching the results.
    assert_eq!(step(&mut vm, &mut host), Some(1));
    assert_eq!(host.confirmations(), [false, true]);
    assert_eq!(step(&mut vm, &mut host), Some(0));
    // 0x22ee reports the second menu group flag, unknown (0) until 0x22b4.
    assert_eq!(step(&mut vm, &mut host), Some(0));
    assert_eq!(vm.stack().len_bytes(), 0);
}

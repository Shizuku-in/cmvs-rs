use cmvs_rs::{
    evaluate::State,
    ps3::Script,
    save::{Csv2Layout, Csv2Save},
    save_menu::SaveHeader,
    snapshot::{Framing, LocalVariables, VmSnapshot, framing},
};
use std::collections::BTreeMap;

fn known_state() -> State {
    let mut state = State::default();
    state.fill_integers(0, 2048, -7).unwrap();
    state.fill_floats(0, 1024, 0).unwrap();
    state.fill_bits(0, 2048, false).unwrap();
    state.fill_strings(0, 64, b"").unwrap();
    state.set_integer(2047, i32::MIN).unwrap();
    state.fill_floats(0, 1, 0x7fc0_1234).unwrap();
    state.fill_floats(1023, 1, 0x8000_0000).unwrap();
    for i in [0, 7, 8, 2047] {
        state.set_bit(i, true).unwrap();
    }
    state.set_string(0, b"raw\x82\xa0").unwrap();
    state.set_string(63, &vec![b'x'; 1023]).unwrap();
    state
}

fn record(out: &mut Vec<u8>, tag: u16, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    match framing(tag).unwrap() {
        Framing::Short => out.extend((body.len() as u16).to_le_bytes()),
        Framing::Long => out.extend((body.len() as u32).to_le_bytes()),
        Framing::Indexed => {
            out.extend(3u16.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
        }
        Framing::StringPair => panic!("fixture does not use string pairs"),
    }
    out.extend(body);
}

fn memory_image() -> Vec<u8> {
    let mut raw = b"PS2A".to_vec();
    // Intentionally retains nonzero disk key and compressed length in a decoded image.
    for n in [48u32, 1, 0x1234_5678, 1, 4, 4, 2, 0, 99, 14, 0] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(0u32.to_le_bytes());
    raw.extend([0, 0x20, 0, 0x20]);
    raw.extend([1, 2, 3, 4]);
    raw.extend(b"x\0");
    raw
}

fn fields() -> BTreeMap<u16, Vec<u8>> {
    let mut fields = BTreeMap::new();
    for i in 0..4u16 {
        fields.insert(
            0x102 + i,
            if i == 0 {
                b"main.ps3\0".to_vec()
            } else {
                vec![0]
            },
        );
        fields.insert(
            0x112 + i,
            if i == 0 {
                memory_image().len() as u32
            } else {
                0
            }
            .to_le_bytes()
            .to_vec(),
        );
    }
    for (tag, value) in [
        (0x106, 0u32),
        (0x107, 2),
        (0x108, 9),
        (0x240, 8),
        (0x241, 1),
    ] {
        fields.insert(tag, value.to_le_bytes().to_vec());
    }
    let mut frames = vec![0; 256];
    frames[4..8].copy_from_slice(&4u32.to_le_bytes());
    fields.insert(0x242, frames);
    fields.insert(0x243, vec![0xa5; 65536]);
    fields.insert(0x248, (0..11u32).flat_map(|n| n.to_le_bytes()).collect());
    fields.insert(0x900, vec![9, 8, 7, 6]);
    fields
}

fn save(fields: &BTreeMap<u16, Vec<u8>>) -> Csv2Save {
    let mut bytes = LocalVariables::from_state(&known_state())
        .unwrap()
        .encode_records();
    for (&tag, body) in fields {
        record(&mut bytes, tag, body);
    }
    record(&mut bytes, 0x200, &0u32.to_le_bytes());
    record(&mut bytes, 0x700, b"uninterpreted object");
    Csv2Save {
        layout: Csv2Layout {
            header: SaveHeader {
                version: b'2',
                label: b"synthetic".to_vec(),
                ordering_key: 0,
            },
            sections: [0..0, 0..0, 0..0],
            decoded_sizes: [bytes.len(), 0, memory_image().len()],
            checksum_offset: 0,
        },
        snapshot: bytes,
        auxiliary: vec![],
        script_image: memory_image(),
    }
}

#[test]
fn local_bank_round_trip_preserves_bits_strings_and_higher_state() {
    let source = known_state();
    let captured = LocalVariables::from_state(&source).unwrap();
    let encoded = captured.encode_records();
    let decoded = LocalVariables::decode(&encoded).unwrap();
    assert_eq!(captured, decoded);
    assert_eq!(decoded.float_bits()[0], 0x7fc0_1234);
    assert_eq!(decoded.float_bits()[1023], 0x8000_0000);
    assert_eq!(decoded.packed_bits()[0], 0x81);
    assert_eq!(decoded.packed_bits()[1], 1);
    let mut target = State::default();
    target.set_integer(2048, 123).unwrap();
    target.set_bit(2048, true).unwrap();
    target.fill_floats(1024, 1, 0xdead_beef).unwrap();
    target.set_string(64, b"persistent").unwrap();
    target.set_system_integer(0, 99).unwrap();
    target.result = Some(42);
    target.condition = Some(true);
    decoded.apply_to_state(&mut target).unwrap();
    assert_eq!(LocalVariables::from_state(&target).unwrap(), decoded);
    assert_eq!(target.integer(2048), Some(123));
    assert_eq!(target.bit(2048), Some(true));
    assert_eq!(target.float_bits(1024), Some(0xdead_beef));
    assert_eq!(target.string(64), Some(&b"persistent"[..]));
    assert_eq!(target.system_integer(0), Some(99));
    assert_eq!(target.result, Some(42));
    assert_eq!(target.condition, Some(true));
    assert_eq!(target.bit(0), Some(true));
    assert_eq!(source.bit(0), Some(true));
    assert_eq!(
        LocalVariables::from_state(&target)
            .unwrap()
            .encode_records(),
        encoded
    );
}

#[test]
fn unknown_capture_and_failed_install_do_not_change_live_state() {
    let mut unknown = State::default();
    unknown.set_bit(0, true).unwrap();
    assert!(LocalVariables::from_state(&unknown).is_err());
    assert_eq!(unknown.bit(0), Some(true));
    let vars = LocalVariables::from_state(&known_state()).unwrap();
    for (int_limit, bit_limit) in [(10, 4096), (4096, 10)] {
        let mut target = State::new(int_limit, bit_limit);
        target.set_integer(0, 123).unwrap();
        target.set_bit(0, false).unwrap();
        target.result = Some(99);
        assert!(vars.apply_to_state(&mut target).is_err());
        assert_eq!(target.integer(0), Some(123));
        assert_eq!(target.bit(0), Some(false));
        assert_eq!(target.float_bits(0), None);
        assert_eq!(target.result, Some(99));
    }
}

#[test]
fn malformed_local_records_fail_without_partial_values() {
    let valid = LocalVariables::from_state(&known_state())
        .unwrap()
        .encode_records();
    let list = cmvs_rs::snapshot::parse_records(&valid).unwrap();
    let bodies: BTreeMap<_, _> = list
        .iter()
        .map(|r| (r.tag, valid[r.body.clone()].to_vec()))
        .collect();
    for tag in 0x280..=0x283 {
        let mut missing = Vec::new();
        let mut duplicate = valid.clone();
        record(&mut duplicate, tag, &bodies[&tag]);
        assert!(LocalVariables::decode(&duplicate).is_err());
        for (&t, b) in &bodies {
            if t != tag {
                record(&mut missing, t, b);
            }
        }
        assert!(LocalVariables::decode(&missing).is_err());
    }
    for bad in [vec![0; 63], vec![0; 65], vec![b'x'; 1024], {
        let mut v = vec![b'x'; 1024];
        v.extend([0; 64]);
        v
    }] {
        let mut stream = Vec::new();
        for (&tag, body) in &bodies {
            record(&mut stream, tag, if tag == 0x283 { &bad } else { body });
        }
        assert!(LocalVariables::decode(&stream).is_err());
    }
    for tag in [0x280, 0x281, 0x282] {
        let mut stream = Vec::new();
        for (&t, body) in &bodies {
            record(
                &mut stream,
                t,
                if t == tag {
                    &body[..body.len() - 1]
                } else {
                    body
                },
            );
        }
        assert!(LocalVariables::decode(&stream).is_err());
    }
    let mut unknown = valid;
    unknown.extend(0xffffu16.to_le_bytes());
    assert!(LocalVariables::decode(&unknown).is_err());
}

#[test]
fn decoded_memory_image_is_not_decrypted_again_and_requires_exact_layout() {
    let raw = memory_image();
    let script = Script::from_memory_image(&raw).unwrap();
    assert_eq!(script.code(), [0, 0x20, 0, 0x20]);
    assert_eq!(script.unknown(), [1, 2, 3, 4]);
    assert_eq!(script.text(), b"x\0");
    assert_eq!(script.header.compressed_size, 99);
    assert_eq!(script.header.key, 0x1234_5678);
    assert!(Script::decode(&raw).is_err());
    for bad in [raw[..raw.len() - 1].to_vec(), {
        let mut b = raw.clone();
        b.push(0);
        b
    }] {
        assert!(Script::from_memory_image(&bad).is_err());
    }
    let mut bad = raw.clone();
    bad[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Script::from_memory_image(&bad).is_err());
    let mut bad = raw.clone();
    bad[32..36].copy_from_slice(&4u32.to_le_bytes());
    assert!(Script::from_memory_image(&bad).is_err());
    let mut bad = raw;
    bad[48..52].copy_from_slice(&4u32.to_le_bytes());
    assert!(Script::from_memory_image(&bad).is_err());
}

#[test]
fn vm_fields_validate_control_and_overlay_saved_mutable_data() {
    let save = save(&fields());
    let vm = VmSnapshot::decode(&save).unwrap();
    assert_eq!(vm.script_names[0], b"main.ps3");
    assert_eq!(vm.active_slot, 0);
    assert_eq!(vm.pc, 2);
    assert!(vm.active_pc_verified);
    assert_eq!(vm.repeat_count, 9);
    assert_eq!(vm.frame_depth, 1);
    assert_eq!(vm.stack_pointer, 8);
    assert_eq!(vm.frames[1], 4);
    assert_eq!(vm.stack.len(), 65536);
    assert_eq!(vm.system_results, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(vm.main_script.unknown(), [9, 8, 7, 6]);
    assert_eq!(
        vm.uninterpreted
            .iter()
            .map(|r| (r.tag, r.index))
            .collect::<Vec<_>>(),
        vec![(0x200, None), (0x700, Some(3))]
    );
}

#[test]
fn bad_vm_state_is_rejected_and_common_pc_is_never_claimed_verified() {
    for (tag, body) in [
        (0x107, 1u32.to_le_bytes().to_vec()),
        (0x106, 4u32.to_le_bytes().to_vec()),
        (0x240, 65537u32.to_le_bytes().to_vec()),
        (0x241, 64u32.to_le_bytes().to_vec()),
        (0x900, vec![0; 3]),
        (0x248, vec![0; 28]),
        (0x102, b"main.ps3".to_vec()),
        (0x112, 1u32.to_le_bytes().to_vec()),
    ] {
        let mut fields = fields();
        fields.insert(tag, body);
        assert!(VmSnapshot::decode(&save(&fields)).is_err(), "tag={tag:x}");
    }
    let mut bad = fields();
    bad.get_mut(&0x242).unwrap()[4..8].copy_from_slice(&9u32.to_le_bytes());
    assert!(VmSnapshot::decode(&save(&bad)).is_err());
    let mut missing = fields();
    missing.remove(&0x248);
    assert!(VmSnapshot::decode(&save(&missing)).is_err());
    let mut duplicate = save(&fields());
    record(&mut duplicate.snapshot, 0x107, &2u32.to_le_bytes());
    assert!(VmSnapshot::decode(&duplicate).is_err());
    let mut common = fields();
    common.insert(0x103, b"common.ps3\0".to_vec());
    common.insert(0x113, 512u32.to_le_bytes().to_vec());
    common.insert(0x106, 1u32.to_le_bytes().to_vec());
    common.insert(0x107, 123u32.to_le_bytes().to_vec());
    let save = save(&common);
    let vm = VmSnapshot::decode(&save).unwrap();
    assert!(!vm.active_pc_verified);
    assert_eq!(vm.pc, 123);
}

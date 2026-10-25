use cmvs_rs::{
    machine::Machine,
    ps3::Script,
    snapshot::{Framing, MAX_SNAPSHOT, SaveImage, SaveTitle, framing, parse_records},
};

fn short(tag: u16, body: &[u8]) -> Vec<u8> {
    let mut b = tag.to_le_bytes().to_vec();
    b.extend((body.len() as u16).to_le_bytes());
    b.extend(body);
    b
}
fn long(tag: u16, index: Option<u16>, body: &[u8]) -> Vec<u8> {
    let mut b = tag.to_le_bytes().to_vec();
    if let Some(index) = index {
        b.extend(index.to_le_bytes());
    }
    b.extend((body.len() as u32).to_le_bytes());
    b.extend(body);
    b
}

#[test]
fn every_writer_framing_is_walked_in_order() {
    let mut stream = short(0x102, b"main.ps3\0");
    stream.extend(long(0x282, None, &[1, 0, 0, 0]));
    stream.extend(long(0x700, Some(29), b"abc"));
    stream.extend(short(0x304, b"x\0"));
    stream.extend(1u16.to_le_bytes());
    stream.push(0);
    stream.extend(long(0x900, None, &[]));
    let records = parse_records(&stream).unwrap();
    let view: Vec<_> = records
        .iter()
        .map(|r| (r.offset, r.tag, r.index, &stream[r.body.clone()]))
        .collect();
    assert_eq!(
        view,
        [
            (0, 0x102, None, &b"main.ps3\0"[..]),
            (13, 0x282, None, &[1, 0, 0, 0][..]),
            (23, 0x700, Some(29), &b"abc"[..]),
            (34, 0x304, None, &[2, 0, b'x', 0, 1, 0, 0][..]),
            (43, 0x900, None, &[][..]),
        ]
    );
    assert!(parse_records(&[]).unwrap().is_empty());
}

#[test]
fn framing_table_matches_writer_groups() {
    assert_eq!(framing(0x10b), Some(Framing::Short));
    assert_eq!(framing(0x248), Some(Framing::Short));
    assert_eq!(framing(0x242), Some(Framing::Long));
    assert_eq!(framing(0x318), Some(Framing::Long));
    assert_eq!(framing(0x8e8), Some(Framing::Indexed));
    assert_eq!(framing(0x304), Some(Framing::StringPair));
    for unknown in [0, 0x10c, 0x116, 0x244, 0x6a1, 0xffff] {
        assert_eq!(framing(unknown), None);
    }
}

#[test]
fn unknown_tags_and_truncation_fail_without_partial_records() {
    let valid = short(0x100, b"t\0");
    let mut unknown = valid.clone();
    unknown.extend(short(0x116, b""));
    assert_eq!(
        parse_records(&unknown).unwrap_err().0,
        "unknown snapshot record tag"
    );
    let pair = {
        let mut b = short(0x304, b"x\0");
        b.extend(5u16.to_le_bytes());
        b.push(0);
        b
    };
    let indexed = long(0x380, Some(7), b"body");
    for bad in [
        &valid[..valid.len() - 1],
        &valid[..1],
        &valid[..3],
        &indexed[..4],
        &indexed[..indexed.len() - 1],
        &pair[..],
    ] {
        assert!(parse_records(bad).is_err(), "{bad:?}");
    }
    let mut huge = 0x243u16.to_le_bytes().to_vec();
    huge.extend(u32::MAX.to_le_bytes());
    assert!(parse_records(&huge).is_err());
}

#[test]
fn stream_larger_than_original_buffer_is_rejected() {
    let mut stream = long(0x243, None, &vec![0; MAX_SNAPSHOT - 6]);
    assert_eq!(stream.len(), MAX_SNAPSHOT);
    assert_eq!(parse_records(&stream).unwrap().len(), 1);
    stream.push(0);
    assert_eq!(
        parse_records(&stream).unwrap_err().0,
        "snapshot exceeds original 3 MiB buffer"
    );
}

#[test]
fn unavailable_capture_clears_bit_zero_and_records_origin() {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(0x430u16.to_le_bytes());
    let mut raw = b"PS2A".to_vec();
    for n in [48u32, 0, 0, 1, 2, 0, 0, 0, 0, 6, 0] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    let script = Script::decode(&raw).unwrap();
    let mut vm = Machine::new(&script).unwrap();
    vm.set_current_name(b"main.ps3".to_vec());
    vm.state_mut().set_bit(0, true).unwrap();
    vm.state_mut().set_bit(1, true).unwrap();
    let mut image = SaveImage::default();
    assert_eq!(image, SaveImage::Absent);
    image.capture_unavailable(&mut vm).unwrap();
    assert_eq!(vm.state().bit(0), Some(false));
    assert_eq!(vm.state().bit(1), Some(true));
    assert_eq!(
        image,
        SaveImage::Unavailable {
            script: b"main.ps3".to_vec(),
            pc: 0
        }
    );
}

fn title_script(text: &[u8], offsets: &[u32]) -> Script {
    let mut code = vec![];
    for offset in offsets {
        code.extend(0x201u16.to_le_bytes());
        code.extend(0x120u16.to_le_bytes());
        code.extend(offset.to_le_bytes());
        code.extend(0x20fu16.to_le_bytes());
        code.extend(0x430u16.to_le_bytes());
        code.extend(0x2162u16.to_le_bytes());
    }
    code.extend(0x2161u16.to_le_bytes());
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(&code);
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
fn to_system(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), cmvs_rs::machine::Step::System { .. }) {}
}

#[test]
fn save_title_copies_short_strings_and_rejects_the_unmodeled_flag_path() {
    let mut text = b"- main -\0".to_vec();
    text.extend([b'x'; 128]);
    text.push(0);
    let script = title_script(&text, &[0, 9]);
    let mut vm = Machine::new(&script).unwrap();
    let mut title = SaveTitle::default();
    assert_eq!(title.bytes(), b"");
    assert!(!title.try_handle(&mut vm).unwrap());
    to_system(&mut vm);
    assert!(title.try_handle(&mut vm).unwrap());
    assert_eq!(title.bytes(), b"- main -");
    assert_eq!(vm.stack().len_bytes(), 0);
    to_system(&mut vm);
    let before = vm.pending_system();
    assert!(title.try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system(), before);
    assert_eq!(title.bytes(), b"- main -");
    vm.complete_system(cmvs_rs::vm::SystemResult(0x4004))
        .unwrap();
    to_system(&mut vm);
    assert!(!title.try_handle(&mut vm).unwrap());
}

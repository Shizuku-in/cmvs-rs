use cmvs_rs::cpz::{ArchiveIndex, ArchiveLayout, ArchiveVersion, Cpz7Index, Cpz7Layout};
use std::io::Cursor;

#[test]
fn cpz7_uses_per_archive_keys_and_countdown_index_tails() {
    use cmvs_rs::cpz::Cpz7Parameters;
    let bytes = hex(include_str!("fixtures/cpz7-keyed.hex"));
    let original_keys = [0x10203040, 0x89abcdef, 0xfedcba98, 0x76543210];
    let parameters = Cpz7Parameters::from(original_keys);
    let mut reader = Cursor::new(&bytes);
    let index = ArchiveIndex::read_with_cpz7_parameters(&mut reader, parameters).unwrap();
    for j in 0..2 {
        let expected = [
            b"synthetic CPZ7 payload ".as_slice(),
            &[b'0' + j as u8],
            b"!!!",
        ]
        .concat();
        assert_eq!(index.read_entry_verified(&mut reader, j).unwrap(), expected);
    }
    assert!(ArchiveIndex::read(&mut Cursor::new(&bytes)).is_err());
    let cpz6 = hex(include_str!("fixtures/cpz6-0.hex"));
    assert!(
        ArchiveIndex::read_with_cpz7_parameters(&mut Cursor::new(cpz6), parameters)
            .unwrap_err()
            .to_string()
            .contains("cannot be used with CPZ3/CPZ5/CPZ6")
    );
    for n in 0..4 {
        let mut keys = original_keys;
        keys[n] ^= 1;
        match ArchiveIndex::read_with_cpz7_parameters(
            &mut Cursor::new(&bytes),
            Cpz7Parameters::from(keys),
        ) {
            Err(_) => assert!(n < 2),
            Ok(index) => {
                let decoded = index
                    .read_entry_verified(&mut Cursor::new(&bytes), 0)
                    .unwrap();
                assert_ne!(decoded, b"synthetic CPZ7 payload 0!!!");
            }
        }
    }
}

fn memory_script(code: Vec<u8>, text: &[u8]) -> cmvs_rs::ps3::Script {
    let mut bytes = b"PS2A".to_vec();
    for field in [
        48,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        bytes.extend(field.to_le_bytes());
    }
    bytes.extend(code);
    bytes.extend(text);
    cmvs_rs::ps3::Script::from_memory_image(&bytes).unwrap()
}

fn push(code: &mut Vec<u8>, string: bool, value: u32) {
    code.extend(if string { 0x201u16 } else { 0x200 }.to_le_bytes());
    code.extend(if string { 0x120u16 } else { 0x100 }.to_le_bytes());
    code.extend(value.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    code.extend(0x430u16.to_le_bytes());
}

fn registration(code: &mut Vec<u8>, keys: [u32; 4]) {
    for key in keys.into_iter().rev() {
        push(code, false, key);
    }
    push(code, true, 1);
    push(code, false, 0);
    code.extend(0x2268u16.to_le_bytes());
}

#[test]
fn cli_discovers_startup_beside_cpz7_and_respects_explicit_overrides() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cpz7-discovery-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("script.cpz");
    let start = root.join("start.ps3");
    let explicit_start = root.join("custom.ps3");
    let legacy = root.join("legacy.cpz");
    fs::write(&archive, hex(include_str!("fixtures/cpz7-keyed.hex"))).unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let mut code = Vec::new();
    push(&mut code, false, 13);
    code.extend(0x201fu16.to_le_bytes());
    registration(&mut code, [0x10203040, 0x89abcdef, 0xfedcba98, 0x76543210]);
    let mut encoded = memory_script(code, b"\0data\\pack\\script.cpz\0")
        .memory_image(&[])
        .unwrap();
    for byte in &mut encoded[48..] {
        *byte = byte.rotate_left(1).wrapping_add(124);
    }
    fs::write(&start, &encoded).unwrap();
    let result = run(&["--list".as_ref(), archive.as_os_str()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("2 files"));
    fs::write(&explicit_start, encoded).unwrap();
    fs::write(&start, b"invalid startup").unwrap();
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    assert!(
        run(&[
            "--start-script".as_ref(),
            explicit_start.as_os_str(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
    assert!(
        run(&[
            "--keys".as_ref(),
            "0x10203040,0x89abcdef,0xfedcba98,0x76543210".as_ref(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
    fs::write(&legacy, hex(include_str!("fixtures/cpz6-0.hex"))).unwrap();
    assert!(
        run(&["--list".as_ref(), legacy.as_os_str()])
            .status
            .success()
    );
    for path in [&archive, &start, &explicit_start, &legacy] {
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir(root).unwrap();
}

#[test]
fn startup_inspection_preserves_key_order_and_rejects_dynamic_or_ambiguous_keys() {
    use cmvs_rs::{cpz::Cpz7Parameters, script_control::cpz7_parameters_from_start};
    let keys = [0x10203040, 0x89abcdef, 0xfedcba98, 0x76543210];
    let text = b"\0data\\pack\\script.cpz\0";
    let mut code = Vec::new();
    push(&mut code, false, 13);
    code.extend(0x201fu16.to_le_bytes());
    registration(&mut code, keys);
    let script = memory_script(code.clone(), text);
    assert_eq!(
        cpz7_parameters_from_start(&script, b"SCRIPT.CPZ").unwrap(),
        Some(Cpz7Parameters::from(keys))
    );
    assert_eq!(
        cpz7_parameters_from_start(&script, b"other.cpz").unwrap(),
        None
    );
    // Variable reference in place of the first literal key must not be treated as a key.
    let mut dynamic = code.clone();
    dynamic[16..18].copy_from_slice(&0x101u16.to_le_bytes());
    assert!(cpz7_parameters_from_start(&memory_script(dynamic, text), b"script.cpz").is_err());
    registration(&mut code, [1, 2, 3, 4]);
    assert!(cpz7_parameters_from_start(&memory_script(code, text), b"script.cpz").is_err());

    let mut code = Vec::new();
    push(&mut code, false, 13);
    code.extend(0x201fu16.to_le_bytes());
    let jump_at = code.len();
    code.extend(0x400u16.to_le_bytes());
    code.extend(0u32.to_le_bytes());
    registration(&mut code, keys);
    let end = code.len() as u32;
    code[jump_at + 2..jump_at + 6].copy_from_slice(&end.to_le_bytes());
    code.extend(0x2000u16.to_le_bytes());
    assert_eq!(
        cpz7_parameters_from_start(&memory_script(code, text), b"script.cpz").unwrap(),
        None
    );

    let mut code = Vec::new();
    push(&mut code, false, 12);
    code.extend(0x201fu16.to_le_bytes());
    assert_eq!(
        cpz7_parameters_from_start(&memory_script(code, text), b"script.cpz").unwrap(),
        Some(Cpz7Parameters::default())
    );
    assert!(
        cpz7_parameters_from_start(
            &memory_script(0x2000u16.to_le_bytes().to_vec(), text),
            b"script.cpz"
        )
        .is_err()
    );
}

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn fixture(i: usize) -> Vec<u8> {
    hex([
        include_str!("fixtures/cpz7-0.hex"),
        include_str!("fixtures/cpz7-1.hex"),
        include_str!("fixtures/cpz7-2.hex"),
        include_str!("fixtures/cpz7-3.hex"),
        include_str!("fixtures/cpz7-4.hex"),
        include_str!("fixtures/cpz7-5.hex"),
        include_str!("fixtures/cpz7-6.hex"),
        include_str!("fixtures/cpz7-7.hex"),
    ][i])
}

#[test]
fn independent_vectors_cover_directories_keys_and_unaligned_resources() {
    for i in 0..8 {
        let mut reader = Cursor::new(fixture(i));
        reader.set_position(9);
        let index = ArchiveIndex::read(&mut reader).unwrap();
        assert_eq!(index.version(), ArchiveVersion::Cpz7);
        assert_eq!(index.entries().len(), 2);
        let name = [b"fixture.bin".as_slice(), &vec![b'y'; i / 4]].concat();
        assert!(
            index
                .find(&name)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert_eq!(index.lookup(&name.to_ascii_uppercase()).unwrap(), Some(0));
        assert_eq!(index.lookup(b"absent.bin").unwrap(), None);
        assert!(index.lookup(b"nul\0name").is_err());
        let ArchiveLayout::Cpz7(layout) = ArchiveLayout::read(&mut reader).unwrap() else {
            panic!("expected CPZ7");
        };
        assert_eq!(layout.directory_count, 2);
        assert_eq!(layout.encryption_mode, 1);
        for j in 0..2 {
            assert_eq!(index.entries()[j].name, name);
            assert_eq!(
                index.entries()[j].directory.as_ref(),
                [b"root".as_slice(), &vec![b'x'; (i + j) % 4]].concat()
            );
            let expected = [
                b"synthetic CPZ7 payload ".as_slice(),
                &[b'0' + j as u8],
                &vec![b'!'; i % 4],
            ]
            .concat();
            assert_eq!(index.read_entry_verified(&mut reader, j).unwrap(), expected);
            assert_eq!(index.read_entry(&mut reader, j).unwrap(), expected);
        }
        assert!(index.read_entry(&mut reader, usize::MAX).is_err());
    }
}

#[test]
fn unencrypted_resources_still_verify_and_use_encrypted_indexes() {
    let mut reader = Cursor::new(hex(include_str!("fixtures/cpz7-plain.hex")));
    let index = Cpz7Index::read(&mut reader).unwrap();
    assert_eq!(index.layout.encryption_mode, 0);
    assert_eq!(
        index.read_entry_verified(&mut reader, 1).unwrap(),
        b"synthetic CPZ7 payload 1"
    );
}

#[test]
fn header_index_key_and_resource_corruption_are_rejected_at_their_boundaries() {
    let original = fixture(0);
    for at in [0, 4, 32, 64, 68] {
        let mut bytes = original.clone();
        bytes[at] ^= 1;
        assert!(ArchiveIndex::read(&mut Cursor::new(bytes)).is_err());
    }
    for at in [72, 120] {
        let mut bytes = original.clone();
        bytes[at] ^= 1;
        let error = ArchiveIndex::read(&mut Cursor::new(bytes)).unwrap_err();
        assert!(error.to_string().contains("index digest mismatch"));
    }
    let index = Cpz7Index::read(&mut Cursor::new(&original)).unwrap();
    let mut bytes = original;
    bytes[index.layout.data_offset as usize] ^= 1;
    let error = index
        .read_entry_verified(&mut Cursor::new(&bytes), 0)
        .unwrap_err();
    assert!(error.to_string().contains("resource checksum mismatch"));
    assert!(index.read_entry(&mut Cursor::new(&bytes), 0).is_ok());
    bytes.truncate(index.layout.data_offset as usize + 1);
    assert!(index.read_entry(&mut Cursor::new(bytes), 0).is_err());
}

#[test]
fn truncated_archives_and_authenticated_invalid_entry_ranges_fail() {
    let bytes = fixture(0);
    for length in 0..bytes.len() {
        assert!(
            ArchiveIndex::read(&mut Cursor::new(&bytes[..length])).is_err(),
            "prefix {length}"
        );
    }
    for vector in [
        include_str!("fixtures/cpz7-offset.hex"),
        include_str!("fixtures/cpz7-size.hex"),
    ] {
        assert!(Cpz7Index::read(&mut Cursor::new(hex(vector))).is_err());
    }
    let index = Cpz7Index::read(&mut Cursor::new(&bytes)).unwrap();
    assert!(Cpz7Layout::parse(&bytes[..72], index.layout.data_offset - 1).is_err());
}

/// Opt-in local evidence only; game archives and extracted assets are never fixtures.
#[test]
#[ignore = "requires CMVS_CPZ7_TEST_INPUTS pointing to read-only external archives"]
fn external_archive_resources() -> Result<(), Box<dyn std::error::Error>> {
    use cmvs_rs::digest::{Sha256, to_hex};
    use std::fs::File;

    let inputs = std::env::var_os("CMVS_CPZ7_TEST_INPUTS").ok_or("set CMVS_CPZ7_TEST_INPUTS")?;
    let sample = std::env::var_os("CMVS_CPZ7_SAMPLE_EDGES").is_some();
    let mut archives = 0;
    for path in std::env::split_paths(&inputs) {
        let mut file = File::open(&path)?;
        let index = if std::env::var_os("CMVS_CPZ7_STARTUP_KEYS").is_some() {
            let start_path = path
                .parent()
                .ok_or("archive has no parent directory")?
                .join("start.ps3");
            let start = cmvs_rs::ps3::Script::decode(&std::fs::read(start_path)?)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("invalid archive basename")?;
            let parameters =
                cmvs_rs::script_control::cpz7_parameters_from_start(&start, name.as_bytes())?
                    .ok_or("missing startup key registration")?;
            ArchiveIndex::read_with_cpz7_parameters(&mut file, parameters)?
        } else {
            ArchiveIndex::read(&mut file)?
        };
        assert_eq!(index.version(), ArchiveVersion::Cpz7);
        let mut bytes = 0u64;
        let mut checked = 0;
        let mut digest = Sha256::new();
        for (i, entry) in index.entries().iter().enumerate() {
            if sample && i != 0 && i + 1 != index.entries().len() {
                continue;
            }
            let data = index.read_entry_verified(&mut file, i)?;
            assert_eq!(data.len(), entry.size as usize);
            let lower = entry.name.to_ascii_lowercase();
            if lower.ends_with(b".ps3") {
                let script = cmvs_rs::ps3::Script::decode(&data)?;
                cmvs_rs::bytecode::decode_layout(
                    script.code(),
                    script.header.entry_pc,
                    &script.labels(),
                )?;
            }
            for (suffix, magic) in [
                (b".ps3".as_slice(), b"PS2A".as_slice()),
                (b".pb3", b"PB3B"),
                (b".ogg", b"OggS"),
            ] {
                if lower.ends_with(suffix) {
                    assert!(
                        data.starts_with(magic),
                        "{}: {}",
                        path.display(),
                        entry.name.escape_ascii()
                    );
                }
            }
            bytes += data.len() as u64;
            checked += 1;
            digest.update(&data);
        }
        eprintln!(
            "{}: entries={} checked={checked} bytes={bytes} plaintext-sha256={}",
            path.display(),
            index.entries().len(),
            to_hex(&digest.finish())
        );
        archives += 1;
    }
    assert!(archives > 0, "at least one archive is required");
    Ok(())
}

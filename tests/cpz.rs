use cmvs_rs::cpz::{CPZ6_HEADER_SIZE, Cpz6Layout};
use cmvs_rs::cpz::{Cpz6Index, data_checksum, lookup_key};
use std::io::Cursor;

fn header(dirs: u32, dir_bytes: u32, file_bytes: u32) -> [u8; CPZ6_HEADER_SIZE] {
    let mut header = [0; CPZ6_HEADER_SIZE];
    header[..4].copy_from_slice(b"CPZ6");
    for (offset, value) in [
        (4, dirs ^ 0xfe3a53da),
        (8, dir_bytes ^ 0x37f298e8),
        (12, file_bytes ^ 0x7a6f3a2d),
        (52, 1 ^ 0xfb73a956),
    ] {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    header
}

#[test]
fn data_offset_accounts_for_both_indexes_without_reading_archive_body() {
    let layout = Cpz6Layout::parse(&header(1, 24, 200), 1000).unwrap();
    assert_eq!(layout.directory_count, 1);
    assert_eq!(layout.data_offset, 288);
    assert_eq!(layout.data_bytes, 712);
    assert_eq!(layout.encryption_mode, 1);
}

#[test]
fn every_truncated_header_and_wrong_version_rejected() {
    let mut bytes = header(1, 24, 200);
    for length in 0..64 {
        assert!(Cpz6Layout::parse(&bytes[..length], 1000).is_err());
    }
    bytes[3] = b'7';
    assert!(Cpz6Layout::parse(&bytes, 1000).is_err());
}

#[test]
fn truncated_archive_and_impossible_directory_count_rejected() {
    assert!(Cpz6Layout::parse(&header(1, 24, 200), 287).is_err());
    assert!(Cpz6Layout::parse(&header(2, 24, 200), 1000).is_err());
    assert!(Cpz6Layout::parse(&header(u32::MAX, u32::MAX, 0), u64::MAX).is_err());
}

#[test]
fn index_arithmetic_uses_64_bits_and_does_not_allocate_from_header_fields() {
    let layout = Cpz6Layout::parse(&header(1, u32::MAX, u32::MAX), u64::MAX).unwrap();
    assert_eq!(layout.data_offset, 64 + 2 * u64::from(u32::MAX));
    assert!(Cpz6Layout::parse(&header(1, u32::MAX, u32::MAX), u32::MAX as u64).is_err());
}

fn vector(index: usize) -> Vec<u8> {
    let fixtures = [
        include_str!("fixtures/cpz6-0.hex"),
        include_str!("fixtures/cpz6-1.hex"),
        include_str!("fixtures/cpz6-2.hex"),
        include_str!("fixtures/cpz6-3.hex"),
        include_str!("fixtures/cpz6-4.hex"),
        include_str!("fixtures/cpz6-5.hex"),
        include_str!("fixtures/cpz6-6.hex"),
        include_str!("fixtures/cpz6-7.hex"),
    ];
    fixtures[index]
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn independent_encrypted_vectors_cover_index_and_resource_tail_bytes() {
    for i in 0..8 {
        let mut reader = Cursor::new(vector(i));
        let index = Cpz6Index::read(&mut reader).unwrap();
        assert_eq!(index.entries().len(), 1);
        let entry = &index.entries()[0];
        let mut dir = b"root".to_vec();
        if i < 4 {
            dir.extend(vec![b'x'; i]);
        }
        let mut name = b"fixture.bin".to_vec();
        if i >= 4 {
            name.extend(vec![b'y'; i - 4]);
        }
        assert_eq!(entry.directory.as_ref(), dir.as_slice());
        assert_eq!(entry.name, name);
        assert_eq!(index.find(&name).unwrap(), 0);
        let mut expected = b"synthetic CPZ6 payload".to_vec();
        expected.extend(vec![b'!'; i % 4]);
        assert_eq!(index.read_entry(&mut reader, 0).unwrap(), expected);
        assert!(index.find(b"absent").is_err());
        assert!(index.read_entry(&mut reader, 1).is_err());
    }
}

#[test]
fn every_truncated_archive_prefix_is_rejected() {
    let bytes = vector(0);
    for length in 0..bytes.len() {
        assert!(
            Cpz6Index::read(&mut Cursor::new(&bytes[..length])).is_err(),
            "prefix {length}"
        );
    }
}

#[test]
fn unknown_encryption_mode_and_oversized_indexes_fail_before_allocation() {
    let mut bytes = vector(0);
    bytes[52..56].copy_from_slice(&0xfb73a956u32.to_le_bytes());
    assert!(Cpz6Index::read(&mut Cursor::new(bytes)).is_err());
    let mut bytes = vector(0);
    bytes[8..12].copy_from_slice(&(u32::MAX ^ 0x37f298e8).to_le_bytes());
    assert!(Cpz6Index::read(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn a_truncated_resource_reader_fails_without_returning_partial_plaintext() {
    let bytes = vector(0);
    let mut reader = Cursor::new(&bytes);
    let index = Cpz6Index::read(&mut reader).unwrap();
    let short = &bytes[..bytes.len() - 1];
    assert!(index.read_entry(&mut Cursor::new(short), 0).is_err());
}

#[test]
fn extract_cli_never_overwrites_an_existing_file() {
    let root = std::env::temp_dir().join(format!("cmvs-cpz-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("synthetic.bin");
    let output = root.join("existing.bin");
    std::fs::write(&input, vector(0)).unwrap();
    std::fs::write(&output, b"keep me").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
        .arg("--extract")
        .arg(&input)
        .arg("fixture.bin")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&output).unwrap(), b"keep me");
    std::fs::remove_file(&input).unwrap();
    std::fs::remove_file(&output).unwrap();
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn trace_cli_loads_and_steps_synthetic_ps3_directly_from_archive() {
    let path = std::env::temp_dir().join(format!("cmvs-cpz-script-{}.bin", std::process::id()));
    let hex = include_str!("fixtures/cpz6-script.hex").trim();
    let bytes: Vec<u8> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    std::fs::write(&path, bytes).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cmvs-trace"))
        .arg(&path)
        .args(["--script", "fixture.ps3", "--profile", "cmvs-20160201"])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("pending system=0x201e"));
    assert!(stdout.contains("top_word=0x00000007"));
}

fn from_hex(hex: &str) -> Vec<u8> {
    hex.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn multiple_directories_have_separate_keys_and_ambiguous_basenames() {
    let mut reader = Cursor::new(from_hex(include_str!("fixtures/cpz6-multiple.hex")));
    let index = Cpz6Index::read(&mut reader).unwrap();
    assert_eq!(index.entries().len(), 2);
    assert_eq!(index.entries()[0].directory.as_ref(), b"first");
    assert_eq!(index.entries()[1].directory.as_ref(), b"second");
    assert!(index.find(b"fixture.bin").is_err());
    assert_eq!(index.read_entry(&mut reader, 0).unwrap(), b"first payload");
    assert_eq!(index.read_entry(&mut reader, 1).unwrap(), b"second payload");
}

#[test]
fn decrypted_resource_offset_overflow_is_rejected() {
    let bytes = from_hex(include_str!("fixtures/cpz6-invalid-offset.hex"));
    let error = Cpz6Index::read(&mut Cursor::new(bytes)).unwrap_err();
    assert!(error.to_string().contains("offset overflow"));
}

#[test]
fn lookup_key_folds_ascii_case_but_not_shift_jis_trail_bytes() {
    assert_eq!(lookup_key(b"LOGO.PB3").unwrap(), b"logo.pb3");
    assert_eq!(
        lookup_key(b"\x82A\x9fZ\xe0Q\xfcRX").unwrap(),
        b"\x82A\x9fZ\xe0Q\xfcRx"
    );
    // 0x80, 0xa0 and 0xfd are not lead bytes, so the following letter is folded.
    assert_eq!(lookup_key(b"\x80A\xa0B\xfdC").unwrap(), b"\x80a\xa0b\xfdc");
    // A trailing lead byte simply ends the string.
    assert_eq!(lookup_key(b"A\x81").unwrap(), b"a\x81");
    assert_eq!(lookup_key(&[b'a'; 63]).unwrap().len(), 63);
    assert!(lookup_key(&[b'a'; 64]).is_err());
    assert!(lookup_key(b"a\0b").is_err());
}

#[test]
fn lookup_takes_the_first_folded_basename_in_index_order() {
    let reader = Cursor::new(from_hex(include_str!("fixtures/cpz6-multiple.hex")));
    let index = Cpz6Index::read(&mut reader.clone()).unwrap();
    assert_eq!(index.lookup(b"FIXTURE.BIN").unwrap(), Some(0));
    assert_eq!(index.entries()[0].directory.as_ref(), b"first");
    assert_eq!(index.lookup(b"absent").unwrap(), None);
    assert!(index.lookup(&[b'a'; 64]).is_err());
}

#[test]
fn data_checksum_sums_words_then_tail_bytes_from_the_original_seed() {
    assert_eq!(data_checksum(&[]), 0x5a90_2b7c);
    assert_eq!(data_checksum(&[1, 0, 0, 0, 2, 3]), 0x5a90_2b7c + 1 + 2 + 3);
    assert_eq!(data_checksum(&[0, 0, 0, 1, 0xff]), 0x5b90_2c7b);
    assert_eq!(data_checksum(&[0xff; 4]), 0x5a90_2b7b);
}

#[test]
fn verified_reads_compare_the_stored_checksum_with_encrypted_bytes() {
    // The synthetic vectors store zero checksums, which their resources do not sum to.
    for i in 0..8 {
        let mut reader = Cursor::new(vector(i));
        let index = Cpz6Index::read(&mut reader).unwrap();
        let error = index.read_entry_verified(&mut reader, 0).unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"), "{i}");
        assert!(index.read_entry(&mut reader, 0).is_ok());
    }
    // Adjusting the first encrypted word makes the resource sum to the stored zero.
    let mut bytes = vector(3);
    let index = Cpz6Index::read(&mut Cursor::new(&bytes)).unwrap();
    let entry = &index.entries()[0];
    let range = entry.offset as usize..entry.offset as usize + entry.size as usize;
    let sum = data_checksum(&bytes[range.clone()]);
    let first = u32::from_le_bytes(bytes[range.start..range.start + 4].try_into().unwrap());
    bytes[range.start..range.start + 4].copy_from_slice(&first.wrapping_sub(sum).to_le_bytes());
    assert_eq!(data_checksum(&bytes[range]), 0);
    let mut reader = Cursor::new(bytes);
    assert_eq!(
        index.read_entry_verified(&mut reader, 0).unwrap(),
        index.read_entry(&mut reader, 0).unwrap()
    );
}

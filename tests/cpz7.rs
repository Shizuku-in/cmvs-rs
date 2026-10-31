use cmvs_rs::cpz::{ArchiveIndex, ArchiveLayout, ArchiveVersion, Cpz7Index, Cpz7Layout};
use std::io::Cursor;

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
    let mut archives = 0;
    for path in std::env::split_paths(&inputs) {
        let mut file = File::open(&path)?;
        let index = ArchiveIndex::read(&mut file)?;
        assert_eq!(index.version(), ArchiveVersion::Cpz7);
        let mut bytes = 0u64;
        let mut digest = Sha256::new();
        for (i, entry) in index.entries().iter().enumerate() {
            let data = index.read_entry_verified(&mut file, i)?;
            assert_eq!(data.len(), entry.size as usize);
            let lower = entry.name.to_ascii_lowercase();
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
            digest.update(&data);
        }
        eprintln!(
            "{}: entries={} bytes={bytes} plaintext-sha256={}",
            path.display(),
            index.entries().len(),
            to_hex(&digest.finish())
        );
        archives += 1;
    }
    assert!(archives > 0, "at least one archive is required");
    Ok(())
}

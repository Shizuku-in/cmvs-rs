//! CPZ1/CPZ2 vectors built by an encoder written independently of the reader.
use cmvs_rs::cpz::{
    ArchiveIndex, ArchiveLayout, ArchiveVersion, Cpz2Index, Cpz7Parameters, cpz3_checksum,
};
use std::collections::BTreeMap;
use std::io::Cursor;

const PHRASE_V2: &[u8; 64] = b"\x88\xc3\x8d\x86\x82\xcd\x82\xcb\x81\x41\x89\xf0\x90\xcd\x82\xb5\x82\xbf\x82\xe1\x83\x5f\x83\x81\x82\xc8\x82\xcc\x81\x63\x82\xd9\x82\xf1\x82\xc6\x82\xbe\x82\xe6\x81\xf4\x82\xa0\x82\xea\x81\x63\x92\x4e\x82\xc9\x8c\xbe\x82\xc1\x82\xc4\x82\xe9\x82\xcc\x81\x48";

fn encrypt_v1(bytes: &mut [u8]) {
    let mut phrase = *PHRASE_V2;
    phrase[47] = 0x41;
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = byte.wrapping_add(108) ^ phrase[i % 64].wrapping_add(10);
    }
}

fn encrypt_v2(bytes: &mut [u8], key: u32) {
    let mut mask = 55u8;
    let scrambled = PHRASE_V2.map(|b| {
        let out = b ^ mask;
        mask = mask.wrapping_sub(41);
        out
    });
    let table: Vec<u32> = scrambled
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()).wrapping_add(key))
        .collect();
    let mut nibbles = 5;
    for n in 0..8 {
        nibbles ^= key >> (4 * n);
    }
    let shift = (nibbles & 15) + 8;
    let words = bytes.len() / 4;
    for i in 0..words {
        let at = 4 * i;
        let plain = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let value = plain.rotate_left(shift).wrapping_add(0x15c3e7) ^ table[i % 16];
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for i in 0..bytes.len() % 4 {
        let at = 4 * words + i;
        bytes[at] = bytes[at].wrapping_sub(55) ^ (table[(words + i) % 16] >> (4 * i)) as u8;
    }
}

/// `padding` appends unused index bytes after the last record, as real archives do.
fn archive(version: u8, files: &[(&[u8], &[u8], u32)], index_key: u32, padding: usize) -> Vec<u8> {
    let mut index = Vec::new();
    let mut data = Vec::new();
    for &(name, payload, key) in files {
        let mut payload = payload.to_vec();
        if version == 1 {
            encrypt_v1(&mut payload);
        } else {
            encrypt_v2(&mut payload, key);
        }
        for value in [
            25 + name.len() as u32,
            payload.len() as u32,
            data.len() as u32,
            0,
            if version == 1 {
                0
            } else {
                cpz3_checksum(&payload)
            },
            key ^ 0x796c3afd,
        ] {
            index.extend(value.to_le_bytes());
        }
        index.extend(name);
        index.push(0);
        data.extend(payload);
    }
    index.resize(index.len() + padding, 0);
    let mut out = Vec::new();
    if version == 1 {
        encrypt_v1(&mut index);
        out.extend(b"CPZ1");
        out.extend((files.len() as u32).to_le_bytes());
        out.extend((index.len() as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
    } else {
        encrypt_v2(&mut index, index_key);
        out.extend(b"CPZ2");
        out.extend((files.len() as u32 ^ 0xe47c59f3).to_le_bytes());
        out.extend((index.len() as u32 ^ 0x3f71de2a).to_le_bytes());
        out.extend(cpz3_checksum(&index).to_le_bytes());
        out.extend((index_key ^ 0x37a9f45b ^ 0x77777777).to_le_bytes());
    }
    out.extend(index);
    out.extend(data);
    out
}

const FILES: [(&[u8], &[u8], u32); 4] = [
    (b"Start.pss", b"PSS0 synthetic payload", 0xaed6cd2a),
    (b"\x83\x65\x83\x58\x83\x67.pb2", b"tail!", 0),
    (b"empty.msk", b"", 0x12345678),
    (b"START.PSS", b"shadowed duplicate..", 0xffffffff),
];

#[test]
fn both_versions_decode_names_payloads_tails_and_padding() {
    for (version, expected) in [(1, ArchiveVersion::Cpz1), (2, ArchiveVersion::Cpz2)] {
        for padding in [0, 1, 2, 3, 40] {
            let bytes = archive(version, &FILES, 0x9d3f2a61, padding);
            let mut reader = Cursor::new(&bytes);
            let index = ArchiveIndex::read(&mut reader).unwrap();
            assert_eq!(index.version(), expected);
            let (file_count, data_end) = match ArchiveLayout::read(&mut reader).unwrap() {
                ArchiveLayout::Cpz1(l) if version == 1 => {
                    (l.file_count, l.data_offset + l.data_bytes)
                }
                ArchiveLayout::Cpz2(l) if version == 2 => {
                    (l.file_count, l.data_offset + l.data_bytes)
                }
                other => panic!("{other:?}"),
            };
            assert_eq!(file_count, 4);
            assert_eq!(data_end, bytes.len() as u64);
            for (i, (name, payload, _)) in FILES.iter().enumerate() {
                assert_eq!(index.entries()[i].name, *name);
                assert!(index.entries()[i].directory.is_empty());
                assert_eq!(index.read_entry(&mut reader, i).unwrap(), *payload);
                assert_eq!(index.read_entry_verified(&mut reader, i).unwrap(), *payload);
            }
            assert!(index.read_entry(&mut reader, 4).is_err());
        }
    }
}

#[test]
fn lookup_folds_ascii_on_both_sides_and_keeps_shift_jis_pairs() {
    for version in [1, 2] {
        let bytes = archive(version, &FILES, 7, 0);
        let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
        let prefix = |stem: &[u8]| match &index {
            ArchiveIndex::Cpz1(index) => index.lookup_prefix(stem).unwrap(),
            ArchiveIndex::Cpz2(index) => index.lookup_prefix(stem).unwrap(),
            _ => panic!(),
        };
        // First match wins; exact `find` sees the two spellings as distinct names.
        assert_eq!(index.lookup(b"start.PSS").unwrap(), Some(0));
        assert_eq!(index.find(b"START.PSS").unwrap(), 3);
        assert!(index.find(b"start.pss").is_err());
        assert_eq!(
            index.lookup(b"\x83\x65\x83\x58\x83\x67.PB2").unwrap(),
            Some(1)
        );
        // 0x65 is a trail byte here, not the letter `e`.
        assert_eq!(index.lookup(b"\x83\x45\x83\x58\x83\x67.pb2").unwrap(), None);
        assert_eq!(index.lookup(b"absent").unwrap(), None);
        assert!(index.lookup(b"a\0b").is_err());
        assert_eq!(prefix(b"EMPTY"), Some(2));
        assert_eq!(prefix(b"start."), Some(0));
        assert_eq!(prefix(b"nothing"), None);
    }
}

#[test]
fn cpz2_checksums_guard_index_and_verified_reads() {
    let bytes = archive(2, &FILES, 0x9d3f2a61, 8);
    let index = Cpz2Index::read(&mut Cursor::new(&bytes)).unwrap();
    let mut corrupted = bytes.clone();
    corrupted[24] ^= 1;
    assert!(Cpz2Index::read(&mut Cursor::new(&corrupted)).is_err());
    let mut corrupted = bytes.clone();
    let at = index.entries()[0].offset as usize;
    corrupted[at] ^= 1;
    let mut reader = Cursor::new(&corrupted);
    assert!(index.read_entry_verified(&mut reader, 0).is_err());
    assert!(index.read_entry(&mut reader, 0).is_ok());
}

#[test]
fn malformed_inputs_are_rejected() {
    for version in [1, 2] {
        let bytes = archive(version, &FILES, 3, 0);
        let header = if version == 1 { 16 } else { 20 };
        for length in 0..header + 60 {
            assert!(ArchiveIndex::read(&mut Cursor::new(&bytes[..length])).is_err());
        }
        // Resource past the end of the file.
        assert!(ArchiveIndex::read(&mut Cursor::new(&bytes[..bytes.len() - 1])).is_err());
        // Declared count that cannot fit in the index.
        let mut many = bytes.clone();
        let count: u32 = if version == 1 {
            1000
        } else {
            1000 ^ 0xe47c59f3
        };
        many[4..8].copy_from_slice(&count.to_le_bytes());
        assert!(ArchiveIndex::read(&mut Cursor::new(&many)).is_err());
        assert!(ArchiveLayout::read(&mut Cursor::new(&many)).is_err());
        assert!(
            ArchiveIndex::read_with_cpz7_parameters(
                &mut Cursor::new(&bytes),
                Cpz7Parameters {
                    seed_add: 1,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let empty_name = archive(1, &[(b"", b"x", 0)], 0, 0);
    assert!(ArchiveIndex::read(&mut Cursor::new(&empty_name)).is_err());
    assert!(
        ArchiveIndex::read(&mut Cursor::new(archive(2, &[], 0, 0)))
            .unwrap()
            .entries()
            .is_empty()
    );
}

#[test]
fn write_back_is_refused() {
    let bytes = archive(2, &FILES, 3, 0);
    let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut output = Cursor::new(Vec::new());
    assert!(
        index
            .write_replacements(&mut Cursor::new(&bytes), &mut output, &BTreeMap::new())
            .is_err()
    );
    assert!(output.into_inner().is_empty());
}

/// Every entry of every listed CVNS archive: verified read, magic by extension,
/// and a linear bytecode decode of each PSS0 script.
/// `CMVS_CVNS_TEST_INPUTS` is a path list of `.cpz` files.
#[test]
#[ignore = "requires local CVNS archives"]
fn external_archive_resources() -> Result<(), Box<dyn std::error::Error>> {
    use cmvs_rs::ps3::{Container, Script};
    let inputs = std::env::var_os("CMVS_CVNS_TEST_INPUTS").ok_or("set CMVS_CVNS_TEST_INPUTS")?;
    let sample = std::env::var_os("CMVS_CVNS_SAMPLE_EDGES").is_some();
    let mut archives = 0;
    for path in std::env::split_paths(&inputs) {
        let mut file = std::fs::File::open(&path)?;
        let index = ArchiveIndex::read(&mut file)?;
        assert!(matches!(
            index.version(),
            ArchiveVersion::Cpz1 | ArchiveVersion::Cpz2
        ));
        let (mut checked, mut scripts) = (0, 0);
        for (i, entry) in index.entries().iter().enumerate() {
            let name = entry.name.to_ascii_lowercase();
            let script = name.ends_with(b".pss");
            if sample && !script && i != 0 && i + 1 != index.entries().len() {
                continue;
            }
            let bytes = index.read_entry_verified(&mut file, i)?;
            assert_eq!(bytes.len(), entry.size as usize);
            if script {
                let script = Script::decode(&bytes)?;
                assert_eq!(script.header().container, Container::Pss0);
                cmvs_rs::bytecode::decode_layout(
                    script.code(),
                    script.header().entry_pc,
                    &script.labels(),
                )?;
                script.strings()?;
                scripts += 1;
            }
            for (extension, magic) in [
                (b".pb2".as_slice(), b"PB2A".as_slice()),
                (b".msk", b"MSK0"),
                (b".mv2", b"MV2X"),
                (b".ogg", b"OggS"),
            ] {
                if name.ends_with(extension) {
                    assert!(bytes.starts_with(magic), "{}", name.escape_ascii());
                }
            }
            checked += 1;
        }
        eprintln!(
            "{}: {:?} entries={} checked={checked} scripts={scripts}",
            path.display(),
            index.version(),
            index.entries().len()
        );
        archives += 1;
    }
    assert!(archives > 0);
    Ok(())
}

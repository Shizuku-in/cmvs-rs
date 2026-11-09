use cmvs_rs::cpz::{ArchiveIndex, ArchiveVersion, Cpz3Index, Cpz7Parameters, data_checksum};
use std::{collections::BTreeMap, io::Cursor};

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn fixtures() -> Vec<(Vec<u8>, Cpz7Parameters)> {
    let mut cases: Vec<_> = [
        include_str!("fixtures/cpz3-0.hex"),
        include_str!("fixtures/cpz3-1.hex"),
        include_str!("fixtures/cpz3-2.hex"),
        include_str!("fixtures/cpz3-3.hex"),
        include_str!("fixtures/cpz3-4.hex"),
        include_str!("fixtures/cpz3-padded.hex"),
        include_str!("fixtures/cpz5-5.hex"),
        include_str!("fixtures/cpz5-6.hex"),
        include_str!("fixtures/cpz5-7.hex"),
        include_str!("fixtures/cpz5-8.hex"),
        include_str!("fixtures/cpz5-9.hex"),
        include_str!("fixtures/cpz5-plain.hex"),
        include_str!("fixtures/cpz6-0.hex"),
        include_str!("fixtures/cpz6-1.hex"),
        include_str!("fixtures/cpz6-2.hex"),
        include_str!("fixtures/cpz6-3.hex"),
        include_str!("fixtures/cpz6-4.hex"),
        include_str!("fixtures/cpz6-5.hex"),
        include_str!("fixtures/cpz6-6.hex"),
        include_str!("fixtures/cpz6-7.hex"),
        include_str!("fixtures/cpz6-multiple.hex"),
        include_str!("fixtures/cpz7-0.hex"),
        include_str!("fixtures/cpz7-1.hex"),
        include_str!("fixtures/cpz7-2.hex"),
        include_str!("fixtures/cpz7-3.hex"),
        include_str!("fixtures/cpz7-4.hex"),
        include_str!("fixtures/cpz7-5.hex"),
        include_str!("fixtures/cpz7-6.hex"),
        include_str!("fixtures/cpz7-7.hex"),
        include_str!("fixtures/cpz7-plain.hex"),
    ]
    .into_iter()
    .map(|s| (hex(s), Cpz7Parameters::default()))
    .collect();
    cases.push((
        hex(include_str!("fixtures/cpz7-keyed.hex")),
        [0x10203040, 0x89abcdef, 0xfedcba98, 0x76543210].into(),
    ));
    // Legacy CPZ6 vectors deliberately store zero resource checksums. Adjust
    // ciphertext to meet those checksums before testing a verified rewrite.
    for (bytes, _) in &mut cases {
        if &bytes[..4] != b"CPZ6" {
            continue;
        }
        let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
        for entry in index.entries() {
            let at = entry.offset as usize;
            let sum = data_checksum(&bytes[at..at + entry.size as usize]);
            let first = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            bytes[at..at + 4].copy_from_slice(
                &first
                    .wrapping_add(entry.checksum_raw)
                    .wrapping_sub(sum)
                    .to_le_bytes(),
            );
        }
    }
    cases
}

#[test]
fn every_version_preserves_names_keys_modes_and_untouched_ciphertext() {
    for (bytes, parameters) in fixtures() {
        let index =
            ArchiveIndex::read_with_cpz7_parameters(&mut Cursor::new(&bytes), parameters).unwrap();
        let mut output = Cursor::new(Vec::new());
        index
            .write_replacements(&mut Cursor::new(&bytes), &mut output, &BTreeMap::new())
            .unwrap();
        let unchanged = output.into_inner();
        if index.version() == ArchiveVersion::Cpz6 {
            // CPZ6's old synthetic header omits digests; rewriting fills them.
            assert_eq!(&unchanged[64..], &bytes[64..]);
        } else {
            assert_eq!(unchanged, bytes);
        }
        for length in [0, 1, 2, 3, 7, 16, 63, 64, 65, 66, 67, 68] {
            let selected = length % index.entries().len();
            let replacement: Vec<_> = (0..length).map(|i| (i * 71) as u8).collect();
            let mut output = Cursor::new(Vec::new());
            index
                .write_replacements(
                    &mut Cursor::new(&bytes),
                    &mut output,
                    &BTreeMap::from([(selected, replacement.clone())]),
                )
                .unwrap();
            let result = output.into_inner();
            let rewritten =
                ArchiveIndex::read_with_cpz7_parameters(&mut Cursor::new(&result), parameters)
                    .unwrap();
            assert_eq!(rewritten.version(), index.version());
            assert_eq!(rewritten.entries().len(), index.entries().len());
            assert_eq!(&result[..12], &bytes[..12]);
            if index.version() != ArchiveVersion::Cpz3 {
                assert_eq!(&result[12..16], &bytes[12..16]);
                assert_eq!(&result[32..60], &bytes[32..60]);
            }
            for (i, (before, after)) in index.entries().iter().zip(rewritten.entries()).enumerate()
            {
                assert_eq!(before.directory, after.directory);
                assert_eq!(before.name, after.name);
                let decoded = rewritten
                    .read_entry_verified(&mut Cursor::new(&result), i)
                    .unwrap();
                if i == selected {
                    assert_eq!(decoded, replacement);
                } else {
                    assert_eq!(
                        decoded,
                        index
                            .read_entry_verified(&mut Cursor::new(&bytes), i)
                            .unwrap()
                    );
                    let a = before.offset as usize;
                    let b = after.offset as usize;
                    assert_eq!(
                        &bytes[a..a + before.size as usize],
                        &result[b..b + after.size as usize]
                    );
                }
            }
        }
    }
}

#[test]
fn rewriting_rejects_invalid_entries_existing_outputs_mismatched_sources_and_corruption() {
    for (bytes, parameters) in fixtures() {
        let index =
            ArchiveIndex::read_with_cpz7_parameters(&mut Cursor::new(&bytes), parameters).unwrap();
        let mut output = Cursor::new(Vec::new());
        assert!(
            index
                .write_replacements(
                    &mut Cursor::new(&bytes),
                    &mut output,
                    &BTreeMap::from([(index.entries().len(), vec![])])
                )
                .is_err()
        );
        assert!(output.into_inner().is_empty());
        let mut output = Cursor::new(b"keep".to_vec());
        assert!(
            index
                .write_replacements(&mut Cursor::new(&bytes), &mut output, &BTreeMap::new())
                .is_err()
        );
        assert_eq!(output.into_inner(), b"keep");
        let mut corrupt = bytes.clone();
        corrupt[index.entries()[0].offset as usize] ^= 1;
        for replacement in [BTreeMap::new(), BTreeMap::from([(0, vec![7])])] {
            assert!(
                index
                    .write_replacements(
                        &mut Cursor::new(&corrupt),
                        &mut Cursor::new(Vec::new()),
                        &replacement
                    )
                    .is_err()
            );
        }
        for missing in [&bytes[..3], &bytes[..bytes.len() - 1]] {
            assert!(
                index
                    .write_replacements(
                        &mut Cursor::new(missing),
                        &mut Cursor::new(Vec::new()),
                        &BTreeMap::new()
                    )
                    .is_err()
            );
        }
        let other = hex(include_str!("fixtures/cpz3-empty.hex"));
        let mut output = Cursor::new(Vec::new());
        assert!(
            index
                .write_replacements(&mut Cursor::new(other), &mut output, &BTreeMap::new())
                .is_err()
        );
        assert!(output.into_inner().is_empty());
    }
}

#[test]
fn cpz3_creation_preserves_raw_names_empty_entries_duplicates_and_order() {
    let entries = vec![
        (b"same.bin".to_vec(), vec![1, 2, 3]),
        (b"same.bin".to_vec(), vec![]),
        (b"\x82\xa0.bin".to_vec(), vec![7; 65]),
    ];
    let mut output = Cursor::new(Vec::new());
    Cpz3Index::write(&mut output, &entries).unwrap();
    let bytes = output.into_inner();
    let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
    assert!(index.find(b"same.bin").is_err());
    for (i, (name, expected)) in entries.iter().enumerate() {
        assert_eq!(&index.entries()[i].name, name);
        assert_eq!(
            index
                .read_entry_verified(&mut Cursor::new(&bytes), i)
                .unwrap(),
            *expected
        );
    }
    for name in [vec![], b"bad\0name".to_vec(), vec![b'a'; 4097]] {
        let mut output = Cursor::new(Vec::new());
        assert!(Cpz3Index::write(&mut output, &[(name, vec![])]).is_err());
        assert!(output.into_inner().is_empty());
    }
    let mut output = Cursor::new(b"keep".to_vec());
    assert!(Cpz3Index::write(&mut output, &entries).is_err());
    assert_eq!(output.into_inner(), b"keep");
    let mut output = Cursor::new(Vec::new());
    Cpz3Index::write(&mut output, &[]).unwrap();
    let empty = output.into_inner();
    assert!(
        ArchiveIndex::read(&mut Cursor::new(&empty))
            .unwrap()
            .entries()
            .is_empty()
    );
    let index = ArchiveIndex::read(&mut Cursor::new(&empty)).unwrap();
    let mut output = Cursor::new(Vec::new());
    index
        .write_replacements(&mut Cursor::new(&empty), &mut output, &BTreeMap::new())
        .unwrap();
    assert_eq!(output.into_inner(), empty);
}

#[test]
fn new_cpz5_modes_and_cpz6_roundtrip_directories_raw_names_and_cipher_boundaries() {
    use cmvs_rs::cpz::{ArchiveDirectory, ArchiveFile, Cpz5Index, Cpz5Mode, Cpz6Index};
    let directories = vec![
        ArchiveDirectory {
            name: b"empty".to_vec(),
            files: vec![],
        },
        ArchiveDirectory {
            name: vec![],
            files: (0..=68)
                .map(|size| ArchiveFile {
                    name: b"duplicate.bin".to_vec(),
                    data: (0..size).map(|i| (i * 71) as u8).collect(),
                })
                .collect(),
        },
        ArchiveDirectory {
            name: b"\x82\xa0".to_vec(),
            files: vec![ArchiveFile {
                name: b"\x82\xa2.bin".to_vec(),
                data: b"last file".to_vec(),
            }],
        },
    ];
    for mode in 5..=10 {
        let mut output = Cursor::new(Vec::new());
        if mode == 10 {
            Cpz6Index::write(&mut output, &directories).unwrap();
        } else {
            Cpz5Index::write(&mut output, Cpz5Mode::try_from(mode).unwrap(), &directories).unwrap();
        }
        let bytes = output.into_inner();
        let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
        match &index {
            ArchiveIndex::Cpz5(i) => {
                assert_eq!(i.mode as u32, mode);
                assert_eq!(i.layout.encryption_mode, 1);
            }
            ArchiveIndex::Cpz6(i) => {
                assert_eq!(mode, 10);
                assert_eq!(i.layout.encryption_mode, 1);
            }
            _ => panic!("unexpected new archive version"),
        }
        let mut i = 0;
        for directory in &directories {
            for file in &directory.files {
                assert_eq!(index.entries()[i].directory.as_ref(), directory.name);
                assert_eq!(index.entries()[i].name, file.name);
                assert_eq!(
                    index
                        .read_entry_verified(&mut Cursor::new(&bytes), i)
                        .unwrap(),
                    file.data
                );
                i += 1;
            }
        }
        assert_eq!(i, index.entries().len());
        assert!(index.find(b"duplicate.bin").is_err());
        let mut output = Cursor::new(Vec::new());
        index
            .write_replacements(&mut Cursor::new(&bytes), &mut output, &BTreeMap::new())
            .unwrap();
        assert_eq!(output.into_inner(), bytes);
        let mut output = Cursor::new(Vec::new());
        if mode == 10 {
            Cpz6Index::write(&mut output, &[]).unwrap();
        } else {
            Cpz5Index::write(&mut output, Cpz5Mode::try_from(mode).unwrap(), &[]).unwrap();
        }
        assert!(
            ArchiveIndex::read(&mut Cursor::new(output.into_inner()))
                .unwrap()
                .entries()
                .is_empty()
        );
    }
}

#[test]
fn directory_archive_creation_rejects_invalid_names_and_nonempty_output() {
    use cmvs_rs::cpz::{ArchiveDirectory, ArchiveFile, Cpz5Index, Cpz5Mode, Cpz6Index};
    let create = |output: &mut Cursor<Vec<u8>>, directories: &[ArchiveDirectory], cpz6| {
        if cpz6 {
            Cpz6Index::write(output, directories)
        } else {
            Cpz5Index::write(output, Cpz5Mode::Mode9, directories)
        }
    };
    for cpz6 in [false, true] {
        for name in [vec![], b"bad\0name".to_vec(), vec![b'a'; 4097]] {
            let mut output = Cursor::new(Vec::new());
            let directories = [ArchiveDirectory {
                name: vec![],
                files: vec![ArchiveFile { name, data: vec![] }],
            }];
            assert!(create(&mut output, &directories, cpz6).is_err());
            assert!(output.into_inner().is_empty());
        }
        for name in [b"bad\0directory".to_vec(), vec![b'a'; 4097]] {
            let mut output = Cursor::new(Vec::new());
            assert!(
                create(
                    &mut output,
                    &[ArchiveDirectory {
                        name,
                        files: vec![]
                    }],
                    cpz6
                )
                .is_err()
            );
            assert!(output.into_inner().is_empty());
        }
        let mut output = Cursor::new(b"keep".to_vec());
        assert!(create(&mut output, &[ArchiveDirectory::default()], cpz6).is_err());
        assert_eq!(output.into_inner(), b"keep");
    }
}

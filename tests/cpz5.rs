use cmvs_rs::{
    cpz::{ArchiveIndex, ArchiveLayout, ArchiveVersion, Cpz5Index, Cpz5Layout, Cpz5Mode},
    ps3::Script,
    script_control::packing_mode_from_start,
};
use std::{
    fs::File,
    io::{Cursor, Read},
};

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn fixture(mode: u32) -> Vec<u8> {
    hex(match mode {
        5 => include_str!("fixtures/cpz5-5.hex"),
        6 => include_str!("fixtures/cpz5-6.hex"),
        7 => include_str!("fixtures/cpz5-7.hex"),
        8 => include_str!("fixtures/cpz5-8.hex"),
        9 => include_str!("fixtures/cpz5-9.hex"),
        _ => unreachable!(),
    })
}

#[test]
fn all_five_modes_decode_independent_vectors_and_are_detected_from_index() {
    for mode in 5..=9 {
        let bytes = fixture(mode);
        let mut reader = Cursor::new(&bytes);
        let index = Cpz5Index::read(&mut reader).unwrap();
        assert_eq!(index.mode as u32, mode);
        let selected =
            ArchiveIndex::read_with_cpz5_mode(&mut reader, Cpz5Mode::try_from(mode).unwrap())
                .unwrap();
        assert_eq!(selected.version(), ArchiveVersion::Cpz5);
        assert_eq!(selected.entries().len(), 2);
        let name = [b"fixture.bin".as_slice(), &vec![b'y'; (mode - 5) as usize]].concat();
        assert!(selected.find(&name).is_err());
        assert_eq!(
            selected.lookup(&name.to_ascii_uppercase()).unwrap(),
            Some(0)
        );
        for j in 0..2 {
            let expected = [
                b"synthetic CPZ5 payload ".as_slice(),
                &[b'0' + j as u8],
                &vec![b'!'; ((mode - 5) % 4) as usize],
            ]
            .concat();
            assert_eq!(
                selected.read_entry_verified(&mut reader, j).unwrap(),
                expected
            );
            assert_eq!(index.read_entry(&mut reader, j).unwrap(), expected);
        }
        assert!(selected.lookup(&[b'a'; 64]).is_err());
        assert!(selected.read_entry(&mut reader, usize::MAX).is_err());
        let ArchiveLayout::Cpz5(layout) = ArchiveLayout::read(&mut reader).unwrap() else {
            panic!("expected CPZ5");
        };
        assert_eq!(layout, index.layout);
    }
    assert!(Cpz5Mode::try_from(4).is_err());
    assert!(Cpz5Mode::try_from(10).is_err());
}

#[test]
fn checksum_layers_truncated_inputs_and_invalid_ranges_are_rejected() {
    let bytes = fixture(9);
    for length in 0..bytes.len() {
        assert!(Cpz5Index::read(&mut Cursor::new(&bytes[..length])).is_err());
    }
    let mut header = bytes.clone();
    header[32] ^= 1;
    assert!(
        Cpz5Layout::parse(&header[..64], header.len() as u64)
            .unwrap_err()
            .to_string()
            .contains("header checksum")
    );
    let mut index_bytes = bytes.clone();
    index_bytes[64] ^= 1;
    assert!(
        Cpz5Index::read(&mut Cursor::new(index_bytes))
            .unwrap_err()
            .to_string()
            .contains("index digest")
    );
    assert!(
        Cpz5Index::read(&mut Cursor::new(hex(include_str!(
            "fixtures/cpz5-offset.hex"
        ))))
        .is_err()
    );
    let mut reader = Cursor::new(bytes.clone());
    let index = Cpz5Index::read(&mut reader).unwrap();
    assert!(Cpz5Layout::parse(&bytes[..64], index.layout.data_offset - 1).is_err());
    let mut corrupt = bytes.clone();
    corrupt[index.entries()[0].offset as usize] ^= 1;
    assert!(
        index
            .read_entry_verified(&mut Cursor::new(corrupt), 0)
            .unwrap_err()
            .to_string()
            .contains("resource checksum")
    );
    assert!(
        index
            .read_entry(&mut Cursor::new(&bytes[..bytes.len() - 1]), 1)
            .is_err()
    );
    for wrong_mode in [5, 6, 7, 8] {
        assert!(
            Cpz5Index::read_with_mode(
                &mut Cursor::new(&bytes),
                Cpz5Mode::try_from(wrong_mode).unwrap()
            )
            .is_err()
        );
    }
}

#[test]
fn impossible_header_counts_modes_and_large_indexes_fail_before_allocation() {
    use cmvs_rs::cpz::data_checksum;
    use std::io::{self, Seek, SeekFrom};
    fn checksum(bytes: &mut [u8]) {
        let sum = data_checksum(&bytes[..60])
            .wrapping_sub(0x5a902b7c)
            .wrapping_sub(0x6dc5a9b4);
        bytes[60..64].copy_from_slice(&sum.to_le_bytes());
    }
    let mut bytes = fixture(9);
    bytes[4..8].copy_from_slice(&(u32::MAX ^ 0xfe3a53d9).to_le_bytes());
    checksum(&mut bytes);
    assert!(Cpz5Layout::parse(&bytes[..64], bytes.len() as u64).is_err());
    let mut bytes = fixture(9);
    bytes[52..56].copy_from_slice(&(2u32 ^ 0xfb73a955).to_le_bytes());
    checksum(&mut bytes);
    assert!(
        Cpz5Index::read_with_mode(&mut Cursor::new(bytes), Cpz5Mode::Mode9)
            .unwrap_err()
            .to_string()
            .contains("encryption mode")
    );
    struct SizedReader(Cursor<Vec<u8>>, u64);
    impl Read for SizedReader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            assert!(out.len() <= 64);
            self.0.read(out)
        }
    }
    impl Seek for SizedReader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            if pos == SeekFrom::End(0) {
                Ok(self.1)
            } else {
                self.0.seek(pos)
            }
        }
    }
    let mut bytes = fixture(9);
    bytes[8..12].copy_from_slice(&((cmvs_rs::MAX_PAYLOAD as u32 + 1) ^ 0x37f298e7).to_le_bytes());
    checksum(&mut bytes);
    let mut reader = SizedReader(Cursor::new(bytes), cmvs_rs::MAX_PAYLOAD as u64 + 1_000_000);
    assert!(
        Cpz5Index::read_with_mode(&mut reader, Cpz5Mode::Mode9)
            .unwrap_err()
            .to_string()
            .contains("allocation limit")
    );
}

#[test]
fn plain_resources_and_startup_mode_selection_work_through_cli() {
    use std::{fs, process::Command};
    let mut reader = Cursor::new(hex(include_str!("fixtures/cpz5-plain.hex")));
    let index = Cpz5Index::read(&mut reader).unwrap();
    assert_eq!(index.layout.encryption_mode, 0);
    assert_eq!(
        index.read_entry_verified(&mut reader, 0).unwrap(),
        b"synthetic CPZ5 payload 0"
    );
    let root = std::env::temp_dir().join(format!("cmvs-cpz5-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("fixture.cpz");
    let start = root.join("start.ps2");
    fs::write(&archive, fixture(5)).unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let mut raw = b"PS2A".to_vec();
    for field in [48u32, 0, 0, 0, 14, 0, 0, 0, 0, 14, 0] {
        raw.extend(field.to_le_bytes());
    }
    let mut code = Vec::new();
    for word in [0x200u16, 0x100] {
        code.extend(word.to_le_bytes());
    }
    code.extend(5u32.to_le_bytes());
    for word in [0x20fu16, 0x430, 0x201f] {
        code.extend(word.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    fs::write(&start, raw).unwrap();
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    fs::write(&start, b"invalid").unwrap();
    assert!(
        !run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    assert!(
        run(&[
            "--packing-mode".as_ref(),
            "5".as_ref(),
            "--list".as_ref(),
            archive.as_os_str()
        ])
        .status
        .success()
    );
    for p in [&start, &archive] {
        fs::remove_file(p).unwrap();
    }
    fs::remove_dir(root).unwrap();
}

#[test]
#[ignore = "requires CMVS_CPZ5_TEST_INPUTS pointing to read-only external archives"]
fn external_archive_resources() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = std::env::var_os("CMVS_CPZ5_TEST_INPUTS").ok_or("set CMVS_CPZ5_TEST_INPUTS")?;
    let sample = std::env::var_os("CMVS_CPZ5_SAMPLE_EDGES").is_some();
    let mut archives = 0;
    for path in std::env::split_paths(&inputs) {
        let directory = path.parent().ok_or("archive has no parent directory")?;
        let start_path = if directory.join("start.ps3").is_file() {
            directory.join("start.ps3")
        } else {
            directory.join("start.ps2")
        };
        let mut raw = Vec::new();
        File::open(start_path)?
            .take(cmvs_rs::MAX_PAYLOAD as u64 + 49)
            .read_to_end(&mut raw)?;
        let mode = Cpz5Mode::try_from(packing_mode_from_start(&Script::decode(&raw)?)?)?;
        let mut file = File::open(&path)?;
        let index = ArchiveIndex::read(&mut file)?;
        assert_eq!(index.version(), ArchiveVersion::Cpz5);
        let ArchiveIndex::Cpz5(detected) = &index else {
            unreachable!()
        };
        assert_eq!(detected.mode, mode);
        let mut scripts = 0;
        let mut checked = 0;
        for (i, entry) in index.entries().iter().enumerate() {
            if sample && i != 0 && i + 1 != index.entries().len() {
                continue;
            }
            let bytes = index.read_entry_verified(&mut file, i)?;
            assert_eq!(bytes.len(), entry.size as usize);
            let name = entry.name.to_ascii_lowercase();
            if name.ends_with(b".ps2") || name.ends_with(b".ps3") {
                let script = Script::decode(&bytes)?;
                cmvs_rs::bytecode::decode_layout(
                    script.code(),
                    script.header.entry_pc,
                    &script.labels(),
                )?;
                scripts += 1;
            }
            if name.ends_with(b".pb3") {
                assert!(bytes.starts_with(b"PB3B"));
            }
            if name.ends_with(b".ogg") {
                assert!(bytes.starts_with(b"OggS"));
            }
            checked += 1;
        }
        eprintln!(
            "{}: mode={} entries={} checked={checked} scripts={scripts}",
            path.display(),
            mode as u32,
            index.entries().len()
        );
        archives += 1;
    }
    assert!(archives > 0);
    Ok(())
}

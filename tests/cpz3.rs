use cmvs_rs::cpz::{
    ArchiveIndex, ArchiveLayout, ArchiveVersion, Cpz3Index, Cpz3Layout, Cpz5Mode, Cpz7Parameters,
    cpz3_checksum,
};
use std::io::Cursor;

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

fn fixture(i: usize) -> Vec<u8> {
    hex([
        include_str!("fixtures/cpz3-0.hex"),
        include_str!("fixtures/cpz3-1.hex"),
        include_str!("fixtures/cpz3-2.hex"),
        include_str!("fixtures/cpz3-3.hex"),
        include_str!("fixtures/cpz3-4.hex"),
        include_str!("fixtures/cpz3-5.hex"),
        include_str!("fixtures/cpz3-6.hex"),
        include_str!("fixtures/cpz3-7.hex"),
    ][i])
}

#[test]
fn independent_vectors_cover_tail_lengths_keys_duplicates_and_dispatch() {
    for i in 0..8 {
        let bytes = fixture(i);
        let mut reader = Cursor::new(&bytes);
        reader.set_position(7);
        let index = ArchiveIndex::read(&mut reader).unwrap();
        let ArchiveLayout::Cpz3(layout) = ArchiveLayout::read(&mut reader).unwrap() else {
            panic!()
        };
        assert_eq!(index.version(), ArchiveVersion::Cpz3);
        assert_eq!(layout.file_count, if i < 4 { 1 } else { 2 });
        let name = [b"fixture.bin".as_slice(), &vec![b'y'; i % 4]].concat();
        assert_eq!(index.lookup(&name.to_ascii_uppercase()).unwrap(), Some(0));
        assert_eq!(index.lookup(b"absent").unwrap(), None);
        if i < 4 {
            assert_eq!(index.find(&name).unwrap(), 0);
        } else {
            assert!(index.find(&name).is_err());
        }
        assert!(index.entries()[0].directory.is_empty());
        let expected = [b"synthetic CPZ3 payload".as_slice(), &vec![b'!'; i % 4]].concat();
        assert_eq!(index.read_entry_verified(&mut reader, 0).unwrap(), expected);
        assert_eq!(index.read_entry(&mut reader, 0).unwrap(), expected);
        if i >= 4 {
            assert_eq!(
                index.read_entry_verified(&mut reader, 1).unwrap(),
                b"second payload"
            );
        }
        assert!(index.lookup(&[b'a'; 64]).is_err());
        assert!(index.lookup(b"a\0b").is_err());
        assert!(index.read_entry(&mut reader, usize::MAX).is_err());
        assert!(ArchiveIndex::read_with_cpz5_mode(&mut reader, Cpz5Mode::Mode5).is_err());
        assert!(
            ArchiveIndex::read_with_cpz7_parameters(
                &mut reader,
                Cpz7Parameters::from([1, 0, 0, 0])
            )
            .is_err()
        );
    }
    let empty = hex(include_str!("fixtures/cpz3-empty.hex"));
    assert!(
        Cpz3Index::read(&mut Cursor::new(empty))
            .unwrap()
            .entries()
            .is_empty()
    );
}

#[test]
fn checksums_bounds_and_every_truncated_prefix_are_checked() {
    let bytes = fixture(3);
    for length in 0..bytes.len() {
        assert!(ArchiveIndex::read(&mut Cursor::new(&bytes[..length])).is_err());
    }
    let mut corrupted = bytes.clone();
    corrupted[20] ^= 1;
    assert!(
        Cpz3Index::read(&mut Cursor::new(corrupted))
            .unwrap_err()
            .to_string()
            .contains("index checksum")
    );
    let mut reader = Cursor::new(bytes.clone());
    let index = Cpz3Index::read(&mut reader).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[index.entries()[0].offset as usize] ^= 1;
    assert!(
        index
            .read_entry_verified(&mut Cursor::new(&corrupt), 0)
            .unwrap_err()
            .to_string()
            .contains("resource checksum")
    );
    assert!(index.read_entry(&mut Cursor::new(corrupt), 0).is_ok());
    assert!(
        index
            .read_entry(&mut Cursor::new(&bytes[..bytes.len() - 1]), 0)
            .is_err()
    );
    assert!(
        Cpz3Index::read(&mut Cursor::new(hex(include_str!(
            "fixtures/cpz3-offset.hex"
        ))))
        .is_err()
    );
    let mut header = bytes[..20].to_vec();
    header[4..8].copy_from_slice(&(u32::MAX ^ 0x5e9c4f37).to_le_bytes());
    assert!(Cpz3Layout::parse(&header, bytes.len() as u64).is_err());
    header[..4].copy_from_slice(b"CPZ4");
    assert!(Cpz3Layout::parse(&header, bytes.len() as u64).is_err());
    assert_eq!(cpz3_checksum(&[]), 0x78341256);
    assert_eq!(cpz3_checksum(&[0xff; 7]), 0x78341552);
}

#[test]
fn large_indexes_fail_before_allocation_and_records_are_bounded() {
    use std::io::{self, Read, Seek, SeekFrom};
    struct HeaderOnly(Cursor<Vec<u8>>);
    impl Read for HeaderOnly {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            assert!(out.len() <= 20);
            self.0.read(out)
        }
    }
    impl Seek for HeaderOnly {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            if position == SeekFrom::End(0) {
                Ok(u64::MAX)
            } else {
                self.0.seek(position)
            }
        }
    }
    let mut header = fixture(0)[..20].to_vec();
    header[8..12].copy_from_slice(&((cmvs_rs::MAX_PAYLOAD as u32 + 1) ^ 0xf32aed17).to_le_bytes());
    assert!(
        Cpz3Index::read(&mut HeaderOnly(Cursor::new(header)))
            .unwrap_err()
            .to_string()
            .contains("allocation limit")
    );
    let bytes = hex(include_str!("fixtures/cpz3-padded.hex"));
    let mut reader = Cursor::new(bytes);
    let index = Cpz3Index::read(&mut reader).unwrap();
    assert_eq!(
        index.read_entry_verified(&mut reader, 0).unwrap(),
        b"synthetic CPZ3 payload"
    );
    for vector in [
        include_str!("fixtures/cpz3-short-record.hex"),
        include_str!("fixtures/cpz3-long-record.hex"),
        include_str!("fixtures/cpz3-no-nul.hex"),
        include_str!("fixtures/cpz3-empty-name.hex"),
    ] {
        assert!(Cpz3Index::read(&mut Cursor::new(hex(vector))).is_err());
    }
}

#[test]
fn cli_lists_extracts_and_rejects_inapplicable_settings() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cpz3-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("fixture.cpz");
    let output = root.join("payload.bin");
    fs::write(&archive, fixture(0)).unwrap();
    // CPZ3 must not try to load adjacent startup settings.
    fs::write(root.join("start.ps2"), b"invalid").unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(env!("CARGO_BIN_EXE_cmvs-cpz"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&[archive.as_os_str()]).status.success());
    assert!(
        run(&["--list".as_ref(), archive.as_os_str()])
            .status
            .success()
    );
    let args = [
        "--extract".as_ref(),
        archive.as_os_str(),
        "fixture.bin".as_ref(),
        output.as_os_str(),
    ];
    assert!(run(&args).status.success());
    assert_eq!(fs::read(&output).unwrap(), b"synthetic CPZ3 payload");
    assert!(!run(&args).status.success());
    for option in [["--keys", "0,0,0,0"], ["--packing-mode", "5"]] {
        assert!(
            !run(&[
                option[0].as_ref(),
                option[1].as_ref(),
                "--list".as_ref(),
                archive.as_os_str()
            ])
            .status
            .success()
        );
    }
    for path in [output, archive, root.join("start.ps2")] {
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir(root).unwrap();
}

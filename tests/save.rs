use cmvs_rs::{
    MAX_PAYLOAD,
    save::{Csv2Layout, Csv2Save, HEADER_SIZE},
};
use std::{fs, process::Command};

fn put(bytes: &mut [u8], at: usize, n: u32) {
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn literals(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in bytes.chunks(8) {
        out.push(((1u16 << chunk.len()) - 1) as u8);
        out.extend(chunk);
    }
    out
}
fn fixture(sections: [&[u8]; 3]) -> Vec<u8> {
    let mut packed = sections.map(literals);
    let key = b"DAME_SAVE_IMAGE";
    for (i, b) in packed[0].iter_mut().enumerate() {
        *b = b.wrapping_add(128) ^ key[i % key.len()];
    }
    let end = HEADER_SIZE + packed.iter().map(Vec::len).sum::<usize>();
    let mut raw = vec![0; HEADER_SIZE];
    raw[..4].copy_from_slice(b"CSV2");
    put(&mut raw, 4, HEADER_SIZE as u32);
    let label = b"2026-10-26 09:08:07 synthetic";
    raw[16..16 + label.len()].copy_from_slice(label);
    put(&mut raw, 540, end as u32);
    raw[546] = 1;
    for i in 0..3 {
        put(&mut raw, 560 + i * 4, packed[i].len() as u32);
        put(&mut raw, 580 + i * 4, sections[i].len() as u32);
    }
    let sum = packed[0]
        .iter()
        .fold(0u16, |sum, b| sum.wrapping_add(u16::from(*b)));
    for chunk in packed {
        raw.extend(chunk);
    }
    raw.extend(sum.to_le_bytes());
    raw
}

#[test]
fn three_sections_decode_and_metadata_can_be_read_from_header_only() {
    let parts: [&[u8]; 3] = [
        b"snapshot with enough bytes to wrap the key repeatedly",
        b"auxiliary",
        b"runtime script image",
    ];
    let raw = fixture(parts);
    let layout = Csv2Layout::parse(&raw[..600], raw.len() as u64).unwrap();
    let save = Csv2Save::decode(&raw).unwrap();
    assert_eq!(save.layout, layout);
    assert_eq!(
        [
            save.snapshot.as_slice(),
            save.auxiliary.as_slice(),
            save.script_image.as_slice()
        ],
        parts
    );
    assert_eq!(layout.checksum_offset + 2, raw.len());
}

#[test]
fn empty_optional_section_and_wrapping_checksum_are_supported() {
    let large = vec![0x7f; 4096];
    let raw = fixture([&large, b"", b"script"]);
    let save = Csv2Save::decode(&raw).unwrap();
    assert_eq!(save.snapshot, large);
    assert!(save.auxiliary.is_empty());
    let sum: u32 = raw[save.layout.sections[0].clone()]
        .iter()
        .map(|b| u32::from(*b))
        .sum();
    assert!(sum > 65535);
}

#[test]
fn every_truncated_prefix_and_trailing_bytes_are_rejected() {
    let raw = fixture([b"snapshot", b"aux", b"script"]);
    for end in 0..raw.len() {
        assert!(Csv2Save::decode(&raw[..end]).is_err(), "end={end}");
    }
    let mut trailing = raw;
    trailing.push(0);
    assert!(Csv2Save::decode(&trailing).is_err());
}

#[test]
fn unsupported_headers_and_modes_fail_without_decoding() {
    let raw = fixture([b"snapshot", b"aux", b"script"]);
    for (at, n) in [(0, u32::from_le_bytes(*b"CSV3")), (4, 512), (546, 0)] {
        let mut bad = raw.clone();
        put(&mut bad, at, n);
        assert!(Csv2Save::decode(&bad).is_err());
    }
    assert!(Csv2Layout::parse(&raw[..600], MAX_PAYLOAD as u64 + 1).is_err());
    assert!(Csv2Layout::parse(&raw[..600], u64::MAX).is_err());
}

#[test]
fn section_length_overflow_and_aggregate_allocation_limit_fail() {
    let raw = fixture([b"snapshot", b"aux", b"script"]);
    for (at, n) in [
        (540, u32::MAX),
        (560, u32::MAX),
        (564, u32::MAX),
        (568, 0),
        (580, u32::MAX),
        (584, 0),
        (588, u32::MAX),
    ] {
        let mut bad = raw.clone();
        put(&mut bad, at, n);
        assert!(Csv2Save::decode(&bad).is_err());
    }
    let mut bad = raw.clone();
    put(&mut bad, 580, MAX_PAYLOAD as u32); // each individually bounded, sum is not
    put(&mut bad, 584, 1);
    assert!(Csv2Layout::parse(&bad[..600], bad.len() as u64).is_err());
    let mut gap = raw;
    let n = u32::from_le_bytes(gap[568..572].try_into().unwrap());
    put(&mut gap, 568, n - 1);
    assert!(Csv2Save::decode(&gap).is_err());
}

#[test]
fn checksum_rejects_changed_snapshot_or_trailer() {
    let raw = fixture([b"snapshot", b"aux", b"script"]);
    for at in [601, raw.len() - 1] {
        let mut bad = raw.clone();
        bad[at] ^= 1;
        assert_eq!(
            Csv2Save::decode(&bad).unwrap_err().0,
            "CSV2 snapshot checksum mismatch"
        );
    }
}

#[test]
fn checksum_does_not_cover_auxiliary_section() {
    let mut raw = fixture([b"snapshot", b"aux", b"script"]);
    let layout = Csv2Layout::parse(&raw[..600], raw.len() as u64).unwrap();
    raw[layout.sections[1].start + 1] = b'b'; // literal value changes, LZSS stays valid
    let save = Csv2Save::decode(&raw).unwrap();
    assert_eq!(save.auxiliary, b"bux");
}

#[test]
fn lzss_length_mismatch_is_rejected_after_valid_checksum() {
    let mut raw = fixture([b"snapshot", b"aux", b"script"]);
    put(&mut raw, 580, 1);
    assert_eq!(
        Csv2Save::decode(&raw).unwrap_err().0,
        "LZSS output exceeds declared size"
    );
    let mut raw = fixture([b"snapshot", b"aux", b"script"]);
    put(&mut raw, 588, 100);
    assert_eq!(
        Csv2Save::decode(&raw).unwrap_err().0,
        "LZSS decoded length differs from header"
    );
}

#[test]
fn cli_export_never_overwrites_and_invalid_input_creates_no_output() {
    let root = std::env::temp_dir().join(format!("cmvs-csv2-cli-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let input = root.join("synthetic.dat");
    let output = root.join("decoded.bin");
    let raw = fixture([b"snapshot", b"aux", b"script"]);
    fs::write(&input, &raw).unwrap();
    let exe = env!("CARGO_BIN_EXE_cmvs-save");
    let export = |target: &std::path::Path| {
        Command::new(exe)
            .args(["--section", "2"])
            .arg(&input)
            .arg(target)
            .output()
            .unwrap()
    };
    assert!(!export(&input).status.success());
    assert_eq!(fs::read(&input).unwrap(), raw);
    assert!(export(&output).status.success());
    assert_eq!(fs::read(&output).unwrap(), b"script");
    assert!(!export(&output).status.success());
    fs::remove_file(&output).unwrap();
    let mut bad = raw;
    bad[601] ^= 1;
    fs::write(&input, bad).unwrap();
    assert!(!export(&output).status.success());
    assert!(!output.exists());
    fs::remove_file(input).unwrap();
    fs::remove_dir(root).unwrap();
}

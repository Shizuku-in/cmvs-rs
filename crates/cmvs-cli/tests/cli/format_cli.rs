use crate::support::TempDir;
use cmvs_core::{
    format::pb3, format::system_save::FileSystemSave, format::system_save::SystemSaveSource,
    host::platform::DirectoryAssets, host::platform::PlatformAssets,
};
use std::{fs, process::Command};

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn pb3_encrypt(b: &mut [u8]) {
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
}

fn pb3_mutate(mut b: Vec<u8>, at: usize, value: u32) -> Vec<u8> {
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = (b[8 + i] ^ key[i % 2]).wrapping_sub(seed[i]);
    }
    put(&mut b, at, value);
    pb3_encrypt(&mut b);
    b
}

fn pb3_simple() -> Vec<u8> {
    let planes: [&[u8]; 3] = [&[1, 2, 3, 4], &[11, 12, 13, 14], &[21, 22, 23, 24]];
    let mut flags = vec![];
    for p in planes {
        let mut b = vec![0; 12];
        put(&mut b, 0, 1);
        put(&mut b, 4, 0);
        put(&mut b, 8, p.len() as u32);
        b.push(0);
        b.push(0);
        flags.push(b);
    }
    let data_at = 52 + 4 * planes.len() + flags.iter().map(Vec::len).sum::<usize>();
    let mut b = vec![0; 52];
    b[..4].copy_from_slice(b"PB3B");
    put(&mut b, 8, 1);
    put(&mut b, 24, 2);
    b[28] = 1;
    b[30..32].copy_from_slice(&2u16.to_le_bytes());
    b[32..34].copy_from_slice(&2u16.to_le_bytes());
    b[34..36].copy_from_slice(&24u16.to_le_bytes());
    put(&mut b, 44, 52);
    put(&mut b, 48, data_at as u32);
    for f in &flags {
        b.extend((f.len() as u32).to_le_bytes());
    }
    for f in flags {
        b.extend(f);
    }
    for p in planes {
        b.extend((p.len() as u32).to_le_bytes());
    }
    for p in planes {
        b.extend(p);
    }
    let len = b.len() as u32;
    put(&mut b, 4, len);
    pb3_encrypt(&mut b);
    b
}

#[test]
fn cli_export_is_create_new_and_unsupported_codec_creates_no_file() {
    let root = TempDir::new("pb3");
    let path = root.join("synthetic.pb3");
    let output = root.join("pixels.bin");
    let raw = pb3_simple();
    fs::write(&path, &raw).unwrap();
    let exe = env!("CARGO_BIN_EXE_cmvs-pb3");
    let run = |p: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--rgba")
            .arg(p)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(
        fs::read(&output).unwrap(),
        pb3::decode(&raw).unwrap().pixels()
    );
    assert!(!run(&output).status.success());
    fs::remove_file(&output).unwrap();
    fs::write(&path, pb3_mutate(raw, 28, 3 | (2 << 16))).unwrap();
    assert!(!run(&output).status.success());
    assert!(!output.exists());
}

fn cur() -> Vec<u8> {
    let mut b = vec![0; 94];
    b[2] = 2;
    b[4] = 1;
    b[6] = 2;
    b[7] = 2;
    b[8] = 4;
    b[10] = 1;
    put(&mut b, 14, 72);
    put(&mut b, 18, 22);
    put(&mut b, 22, 40);
    put(&mut b, 26, 2);
    put(&mut b, 30, 4);
    b[34] = 1;
    b[36] = 8;
    put(&mut b, 54, 4);
    b[66..70].copy_from_slice(&[30, 20, 10, 0]);
    b[70..74].copy_from_slice(&[255, 255, 255, 0]);
    b[78..86].copy_from_slice(&[2, 3, 0, 0, 0, 1, 0, 0]);
    b[86] = 0xc0;
    b
}

#[test]
fn asset_paths_and_cursor_cli_are_bounded_and_read_only() {
    let root = TempDir::new("platform");
    fs::create_dir(root.join("icons")).unwrap();
    let path = root.join("icons/mouse.cur");
    let raw = cur();
    fs::write(&path, &raw).unwrap();
    let mut assets = DirectoryAssets::new(&root).unwrap();
    assert_eq!(assets.read(b"icons\\mouse.cur").unwrap(), raw);
    for name in [
        b"../escape".as_slice(),
        b"icons/../mouse.cur",
        b"/icons/mouse.cur",
        b"C:\\mouse.cur",
        b"\xff",
    ] {
        assert!(assets.read(name).is_err());
    }
    let output = root.join("planes.bin");
    let exe = env!("CARGO_BIN_EXE_cmvs-cursor");
    let run = |target: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--planes")
            .arg(target)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(fs::read(&output).unwrap().len(), 16);
    assert!(!run(&output).status.success());
}

fn literals(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in bytes.chunks(8) {
        out.push(((1u16 << chunk.len()) - 1) as u8);
        out.extend(chunk);
    }
    out
}

fn csv2_fixture(sections: [&[u8]; 3]) -> Vec<u8> {
    use cmvs_core::format::save::HEADER_SIZE;
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
fn cli_lists_snapshot_records_and_rejects_unknown_tags() {
    let root = TempDir::new("csv2-records");
    let input = root.join("synthetic.dat");
    let mut snapshot = vec![0x02, 0x01, 4, 0, b'a', b'.', b'p', 0];
    snapshot.extend([0x00, 0x07, 3, 0, 9, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    fs::write(&input, csv2_fixture([&snapshot, b"", b"script"])).unwrap();
    let exe = env!("CARGO_BIN_EXE_cmvs-save");
    let output = Command::new(exe)
        .arg("--records")
        .arg(&input)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("snapshot records=2"));
    assert!(stdout.contains("record offset=0 tag=0x102 index=None len=4"));
    assert!(stdout.contains("record offset=8 tag=0x700 index=Some(3) len=9"));
    snapshot[0] = 0x16;
    fs::write(&input, csv2_fixture([&snapshot, b"", b"script"])).unwrap();
    let output = Command::new(exe)
        .arg("--records")
        .arg(&input)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown snapshot record tag"));
}

#[test]
fn cli_export_never_overwrites_and_invalid_input_creates_no_output() {
    let root = TempDir::new("csv2-cli");
    let input = root.join("synthetic.dat");
    let output = root.join("decoded.bin");
    let raw = csv2_fixture([b"snapshot", b"aux", b"script"]);
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
}

fn css1_body() -> Vec<u8> {
    vec![0; 44864]
}

fn css1_fixture(payload: &[u8]) -> Vec<u8> {
    let packed = literals(payload);
    let mut raw = vec![0; 360];
    raw[..4].copy_from_slice(b"CSS1");
    put(&mut raw, 4, 104);
    put(&mut raw, 68, 0x12345678);
    raw[76] = 1;
    raw[78] = 7;
    raw[80] = 161;
    raw[82..84].copy_from_slice(&(((4u32 - 2) << 3) as u16).to_le_bytes());
    put(&mut raw, 92, payload.len() as u32);
    put(&mut raw, 96, packed.len() as u32);
    put(&mut raw, 100, 360 + packed.len() as u32);
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, b) in packed.iter().enumerate() {
        raw.push(
            ((b.wrapping_add(64) ^ 161u8.wrapping_add(key[i % key.len()])).wrapping_add(7))
                .rotate_left(4),
        );
    }
    let sum = raw[360..]
        .iter()
        .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
    raw.extend(sum.to_le_bytes());
    raw
}

#[test]
fn file_source_and_cli_remain_read_only_and_never_overwrite_output() {
    let root = TempDir::new("css1");
    let path = root.join("synthetic.dat");
    let output = root.join("decoded.bin");
    let body = css1_body();
    let raw = css1_fixture(&body);
    fs::write(&path, &raw).unwrap();
    assert_eq!(
        FileSystemSave::new(&path).read().unwrap(),
        Some(raw.clone())
    );
    assert_eq!(
        FileSystemSave::new(root.join("absent")).read().unwrap(),
        None
    );
    let exe = env!("CARGO_BIN_EXE_cmvs-system-save");
    let run = |target: &std::path::Path| {
        Command::new(exe)
            .arg(&path)
            .arg("--payload")
            .arg(target)
            .output()
            .unwrap()
    };
    assert!(!run(&path).status.success());
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert!(run(&output).status.success());
    assert_eq!(fs::read(&output).unwrap(), body);
    assert!(!run(&output).status.success());
    fs::remove_file(&output).unwrap();
    let mut bad = raw;
    bad[361] ^= 1;
    fs::write(&path, bad).unwrap();
    assert!(!run(&output).status.success());
    assert!(!output.exists());
}

use crate::support::{TempDir, bytecode::command};
use cmvs_core::format::cmv::{KIND_JBPX, KIND_OGG};
use std::{fs, path::Path, process::Command};

const OGG_FIXTURE: &[u8] = include_bytes!("../../../cmvs-audio/tests/fixtures/audio/mono440.ogg");

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn synthetic_ps2a(code: &[u8]) -> Vec<u8> {
    let mut raw = b"PS2A".to_vec();
    for field in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        0,
        0,
        0,
        code.len() as u32,
        0,
    ] {
        raw.extend_from_slice(&field.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    raw
}

fn synthetic_jbpd_16x16() -> Vec<u8> {
    // Six zero DC deltas encoded with 4-bit symbol 0 (0000).
    let dc_bits = vec![0u8; 24];
    // Six EOB markers encoded with 4-bit symbol 15 (1111).
    let ac_bits = vec![1u8; 24];
    let pack = |bits: &[u8]| {
        let mut bytes = vec![0u8; bits.len().div_ceil(32) * 4];
        for (i, &bit) in bits.iter().enumerate() {
            bytes[i / 8] |= bit << (i % 8);
        }
        bytes
    };
    let dc = pack(&dc_bits);
    let ac = pack(&ac_bits);
    let mut data = vec![0u8; 48];
    data[..4].copy_from_slice(b"JBPD");
    for (at, value) in [
        (4, 44u32),
        (8, 0x58c0_0002),
        (16, 16 | (16 << 16)),
        (20, 24),
        (28, dc.len() as u32),
        (32, ac.len() as u32),
    ] {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for _ in 0..32 {
        data.extend(1u32.to_le_bytes());
    }
    data.extend([0u8; 16]);
    data.extend([1u8; 128]);
    data.extend([1, 1, 1, 0, 1, 1, 0, 0]);
    data.extend(dc);
    data.extend(ac);
    data
}

fn synthetic_cmv6() -> Vec<u8> {
    let jbpd = synthetic_jbpd_16x16();
    let count = 2u32;
    let resource = 44 + count * 20;
    let total_data = jbpd.len() as u32 + OGG_FIXTURE.len() as u32;
    let mut bytes = b"CMV6".to_vec();
    for word in [resource, resource + total_data, 0, 1, 24, 1, 16, 16, 24, 1] {
        bytes.extend(word.to_le_bytes());
    }
    for word in [0, jbpd.len() as u32, 16 * 16 * 3, KIND_JBPX, 0] {
        bytes.extend(word.to_le_bytes());
    }
    for word in [
        1,
        OGG_FIXTURE.len() as u32,
        OGG_FIXTURE.len() as u32,
        KIND_OGG,
        jbpd.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(&jbpd);
    bytes.extend(OGG_FIXTURE);
    bytes
}

fn assert_help_ok(exe: &str, cwd: &Path, args: &[&str]) {
    let out = Command::new(exe)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{exe} {args:?} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.starts_with("Usage:"),
        "{exe} {args:?} did not print Usage:\n{stdout}"
    );
    assert!(
        stderr.is_empty(),
        "{exe} {args:?} wrote to stderr:\n{stderr}"
    );
}

fn assert_not_help(exe: &str, cwd: &Path, args: &[&str]) {
    let out = Command::new(exe)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "{exe} {args:?} unexpectedly succeeded:\n{stdout}"
    );
    assert!(
        !stdout.starts_with("Usage:"),
        "{exe} {args:?} treated argument after `--` as help:\n{stdout}"
    );
    assert!(
        !stderr
            .lines()
            .any(|line| line.ends_with("unknown option --")
                || line.ends_with("unknown subcommand --")),
        "{exe} {args:?} treated `--` itself as an unknown option or subcommand:\n{stderr}"
    );
}

#[test]
fn all_cli_binaries_honor_help_in_any_position_and_respect_double_dash() {
    let empty_dir = TempDir::new("help-positions");
    let binaries: &[(&str, &[&[&str]])] = &[
        (
            env!("CARGO_BIN_EXE_cmvs-inspect"),
            &[
                &["script.ps3", "--payload", "--help"],
                &["script.ps3", "--payload", "out.bin", "-h"],
            ],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-trace"),
            &[&["--steps", "--help"], &["script.ps3", "--steps", "-h"]],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-cpz"),
            &[
                &["--list", "--help"],
                &["--start-script", "missing.ps3", "-h"],
                &["archive.cpz", "--help"],
            ],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-save"),
            &[
                &["--records", "--help"],
                &["--section", "0", "-h"],
                &["save000.dat", "--help"],
            ],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-system-save"),
            &[
                &["system.dat", "--help"],
                &["system.dat", "--payload", "-h"],
            ],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-cursor"),
            &[&["01.cur", "--rgba", "--help"], &["--planes", "-h"]],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-pb3"),
            &[&["bg.pb3", "--rgba", "--help"], &["--rgba", "-h"]],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-cmv"),
            &[
                &["op.cmv", "--frame", "0", "--help"],
                &["--check-frames", "-h"],
            ],
        ),
        (
            env!("CARGO_BIN_EXE_cmvs-mgv"),
            &[
                &["op.mgv", "--frame", "0", "--help"],
                &["--check-frames", "-h"],
            ],
        ),
        #[cfg(feature = "audio-output")]
        (
            env!("CARGO_BIN_EXE_cmvs-audio"),
            &[&["voice.ogg", "--pcm", "--help"], &["--pcm", "-h"]],
        ),
        #[cfg(feature = "desktop")]
        (
            env!("CARGO_BIN_EXE_cmvs-desktop"),
            &[&[".", "--encoding", "--help"], &["--encoding", "-h"]],
        ),
    ];

    for &(exe, extra_cases) in binaries {
        for base in [
            &["-h"][..],
            &["--help"],
            &["missing-input", "-h"],
            &["missing-input", "--help"],
        ] {
            assert_help_ok(exe, &empty_dir, base);
        }
        for case in extra_cases {
            assert_help_ok(exe, &empty_dir, case);
        }
        assert_not_help(exe, &empty_dir, &["--"]);
        assert_not_help(exe, &empty_dir, &["--", "--help"]);
        assert_not_help(exe, &empty_dir, &["--", "-h"]);
    }

    // Verify `--` allows inspecting real files/directories literally named `--help`.
    let literal_dir = TempDir::new("literal-help-file");
    let mut code = Vec::new();
    command(&mut code, 0x2003, &[]);
    fs::write(literal_dir.join("--help"), synthetic_ps2a(&code)).unwrap();
    for (exe, args) in [
        (env!("CARGO_BIN_EXE_cmvs-inspect"), &["--", "--help"][..]),
        (
            env!("CARGO_BIN_EXE_cmvs-trace"),
            &["--profile", "cmvs-20160201", "--", "--help"][..],
        ),
    ] {
        let out = Command::new(exe)
            .current_dir(&literal_dir)
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{exe} {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!stdout.starts_with("Usage:"), "{stdout}");
    }
}

#[test]
fn mgv_cli_inspects_checks_frames_and_exports_without_desktop_feature() {
    let dir = TempDir::new("mgv-smoke");
    let movie = dir.join("synthetic.mgv");
    let rgba_out = dir.join("frame0.rgba");
    let png_out = dir.join("frame0.png");
    let raw = hex(include_str!("../../../cmvs-video/tests/fixtures/mgv1.hex"));
    fs::write(&movie, &raw).unwrap();

    let exe = env!("CARGO_BIN_EXE_cmvs-mgv");
    let inspect = Command::new(exe).arg(&movie).output().unwrap();
    let stdout = String::from_utf8(inspect.stdout).unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(
        stdout.contains("MGV1 32x32 frames=4 packets=3 rate=3/1"),
        "{stdout}"
    );

    let check = Command::new(exe)
        .arg(&movie)
        .arg("--check-frames")
        .output()
        .unwrap();
    let stdout = String::from_utf8(check.stdout).unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        stdout.contains("fe911eee671616f6b45be1128bd5a0ad643b00d746cd33f4e0da3d1fd9edec40"),
        "{stdout}"
    );
    assert!(
        stdout.contains("verified 4 nominal frames, 3 decoded packets"),
        "{stdout}"
    );

    assert!(
        Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&rgba_out)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&rgba_out).unwrap().len(), 32 * 32 * 4);
    // Refuses to overwrite existing output.
    assert!(
        !Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&rgba_out)
            .output()
            .unwrap()
            .status
            .success()
    );

    assert!(
        Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&png_out)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read(&png_out)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
}

#[test]
fn cmv_cli_inspects_checks_frames_and_exports_synthetic_jbpd_movie() {
    let dir = TempDir::new("cmv-smoke");
    let movie = dir.join("synthetic.cmv");
    let rgba_out = dir.join("frame0.rgba");
    let png_out = dir.join("frame0.png");
    fs::write(&movie, synthetic_cmv6()).unwrap();

    let exe = env!("CARGO_BIN_EXE_cmvs-cmv");
    let inspect = Command::new(exe).arg(&movie).output().unwrap();
    let stdout = String::from_utf8(inspect.stdout).unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(
        stdout.contains("CMV6") && stdout.contains("max_sequence=1 rate=24/1 size=16x16"),
        "{stdout}"
    );
    assert!(
        stdout.contains("entries=2 jbpx_frames=1") && stdout.contains("ogg: sequence=1"),
        "{stdout}"
    );

    let check = Command::new(exe)
        .arg(&movie)
        .arg("--check-frames")
        .output()
        .unwrap();
    let stdout = String::from_utf8(check.stdout).unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(stdout.contains("decoded 1 frames successfully"), "{stdout}");

    assert!(
        Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&rgba_out)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&rgba_out).unwrap().len(), 16 * 16 * 4);
    assert!(
        !Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&rgba_out)
            .output()
            .unwrap()
            .status
            .success()
    );

    assert!(
        Command::new(exe)
            .arg(&movie)
            .args(["--frame", "0"])
            .arg(&png_out)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        fs::read(&png_out)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
}

#[cfg(feature = "audio-output")]
#[test]
fn audio_cli_inspects_ogg_and_cmv_and_exports_pcm_without_device() {
    let dir = TempDir::new("audio-cli-smoke");
    let ogg = dir.join("tone.ogg");
    let cmv = dir.join("movie.cmv");
    let pcm = dir.join("tone.f32");
    fs::write(&ogg, OGG_FIXTURE).unwrap();
    fs::write(&cmv, synthetic_cmv6()).unwrap();

    let exe = env!("CARGO_BIN_EXE_cmvs-audio");
    let inspect = Command::new(exe).arg(&ogg).output().unwrap();
    let stdout = String::from_utf8(inspect.stdout).unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(
        stdout.contains("Vorbis channels=1 rate=8000 samples=768 frames=768"),
        "{stdout}"
    );

    let extra = Command::new(exe)
        .arg("--")
        .arg(&ogg)
        .arg("--pcm")
        .arg(&pcm)
        .output()
        .unwrap();
    assert!(!extra.status.success());
    assert!(!pcm.exists());

    let export = Command::new(exe)
        .arg(&cmv)
        .arg("--pcm")
        .arg(&pcm)
        .output()
        .unwrap();
    let stdout = String::from_utf8(export.stdout).unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(stdout.contains("samples=768"), "{stdout}");
    assert_eq!(fs::read(&pcm).unwrap().len(), 768 * 4);

    let pcm_before = dir.join("tone_before.f32");
    let flag_first = Command::new(exe)
        .arg("--pcm")
        .arg(&pcm_before)
        .arg(&ogg)
        .output()
        .unwrap();
    assert!(
        flag_first.status.success(),
        "{}",
        String::from_utf8_lossy(&flag_first.stderr)
    );
    assert_eq!(fs::read(&pcm_before).unwrap().len(), 768 * 4);

    // Refuses to overwrite existing PCM file.
    assert!(
        !Command::new(exe)
            .arg(&ogg)
            .arg("--pcm")
            .arg(&pcm)
            .output()
            .unwrap()
            .status
            .success()
    );
}

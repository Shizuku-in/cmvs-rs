#![cfg(feature = "desktop")]
use cmvs_rs::desktop::TextEncoding;
use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Fixture {
    fn new(encoding: TextEncoding) -> Self {
        let serial = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("cmvs-encoding-{}-{serial}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(root.join("data/pack")).unwrap();
        let text = encoding
            .encode("这是用于自动识别文字编码的中文句子，请检查显示是否正确。")
            .unwrap();
        let code = 0x2001u16.to_le_bytes();
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(code);
        payload.extend(&text);
        payload.push(0);
        let mut script = b"PS2A".to_vec();
        for word in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            text.len() as u32 + 1,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            script.extend(word.to_le_bytes());
        }
        script.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        fs::write(root.join("data/pack/start.ps3"), script).unwrap();
        fs::write(root.join("data/pack/script.cpz"), []).unwrap();
        fs::write(root.join("invalid.ttf"), b"invalid font").unwrap();
        Self(root)
    }
    fn run(&self, encoding: &str, extra: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
            .arg(&self.0)
            .args([
                "--profile",
                "cmvs-20160201",
                "--encoding",
                encoding,
                "--font",
            ])
            .arg(self.0.join("invalid.ttf"))
            .args(extra)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn gbk_font_selection_reaches_font_loading_explicitly_and_via_auto_detection() {
    let fixture = Fixture::new(TextEncoding::Gbk);
    for encoding in ["gbk", "auto"] {
        let output = fixture.run(encoding, &[]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(error.contains("invalid OpenType font"), "{error}");
        if encoding == "auto" {
            assert!(String::from_utf8_lossy(&output.stdout).contains("auto encoding=gbk"));
        }
    }
}

#[test]
fn utf8_playback_is_rejected_explicitly_and_after_auto_detection() {
    let fixture = Fixture::new(TextEncoding::Utf8);
    for encoding in ["utf-8", "auto"] {
        let output = fixture.run(encoding, &[]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(
            error.contains("UTF-8 engine text scanning is not supported"),
            "{error}"
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_cmvs-desktop"))
        .arg(&fixture.0)
        .args(["--encoding", "utf-8", "--record-text"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("UTF-8 engine text scanning is not supported")
    );
}

#[test]
fn font_and_record_text_remain_mutually_exclusive_for_gbk() {
    let fixture = Fixture::new(TextEncoding::Gbk);
    let output = fixture.run("gbk", &["--record-text"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--font cannot be combined with --record-text")
    );
}

#[test]
fn host_encoding_maps_to_the_engine_without_a_utf8_fallback() {
    assert_eq!(
        TextEncoding::ShiftJis.engine_encoding().unwrap(),
        cmvs_rs::TextEncoding::ShiftJis
    );
    assert_eq!(
        TextEncoding::Gbk.engine_encoding().unwrap(),
        cmvs_rs::TextEncoding::Gbk
    );
    assert!(TextEncoding::Utf8.engine_encoding().is_err());
    assert_eq!(
        TextEncoding::Utf8.decode("标题".as_bytes()).unwrap(),
        "标题"
    );
}

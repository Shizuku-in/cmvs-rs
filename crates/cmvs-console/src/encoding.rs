//! The game text encoding: `encoding.txt` in the game directory, or detection from the
//! startup script when that file is absent; and the system fonts its text needs.
use cmvs_core::TextEncoding;
use cmvs_font::text_codec;
use std::{error::Error, io::ErrorKind, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// `encoding.txt` in `root` holds `shift-jis`, `gbk` or `utf-8`; without it the encoding
/// is inferred from `text`, the startup script's text section.
pub fn select(root: &Path, text: &[u8]) -> Result<TextEncoding> {
    match std::fs::read_to_string(root.join("encoding.txt")) {
        Ok(setting) => parse(Some(&setting), text),
        Err(error) if error.kind() == ErrorKind::NotFound => parse(None, text),
        Err(error) => Err(error.into()),
    }
}

fn parse(setting: Option<&str>, text: &[u8]) -> Result<TextEncoding> {
    match setting.map(str::trim) {
        None => text_codec::detect(text).map_err(|_| {
            "cannot detect the game text encoding; put encoding.txt holding shift-jis, gbk or utf-8 in the game directory".into()
        }),
        Some(name) => text_codec::parse(name).map_err(|_| {
            format!("encoding.txt must be shift-jis, gbk or utf-8, not {name:?}").into()
        }),
    }
}

/// A system font family the console players ship with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemFont {
    Japanese,
    SimplifiedChinese,
}

/// System fonts for a game without `font.ttf`, primary first. GBK and UTF-8 games are
/// mostly Chinese fan patches, so Chinese glyph shapes come first and the Japanese font
/// covers what it lacks, such as kana.
pub fn system_fonts(encoding: TextEncoding) -> &'static [SystemFont] {
    match encoding {
        TextEncoding::Gbk | TextEncoding::Utf8 => {
            &[SystemFont::SimplifiedChinese, SystemFont::Japanese]
        }
        _ => &[SystemFont::Japanese],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_overrides_detection() {
        let mut gbk = text_codec::encode(
            TextEncoding::Gbk,
            "这是用于自动识别文字编码的中文句子，请检查显示是否正确。",
        )
        .unwrap();
        gbk.push(0);
        let gbk = gbk.as_slice();
        assert_eq!(parse(None, gbk).unwrap(), TextEncoding::Gbk);
        assert_eq!(parse(Some("utf-8\r\n"), gbk).unwrap(), TextEncoding::Utf8);
        assert_eq!(
            parse(Some("shift-jis"), b"").unwrap(),
            TextEncoding::ShiftJis
        );
        let bad = parse(Some("auto"), gbk).unwrap_err().to_string();
        assert!(bad.contains("\"auto\""), "{bad}");
        let uncertain = parse(None, b"menu.ps3\0").unwrap_err().to_string();
        assert!(uncertain.contains("encoding.txt"), "{uncertain}");
    }

    #[test]
    fn select_reads_the_game_directory() {
        let root =
            std::env::temp_dir().join(format!("cmvs-console-encoding-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(select(&root, b"menu.ps3\0").is_err());
        std::fs::write(root.join("encoding.txt"), "gbk\n").unwrap();
        assert_eq!(select(&root, b"menu.ps3\0").unwrap(), TextEncoding::Gbk);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn chinese_text_gets_a_chinese_system_font() {
        use SystemFont::*;
        assert_eq!(system_fonts(TextEncoding::ShiftJis), [Japanese]);
        assert_eq!(
            system_fonts(TextEncoding::Gbk),
            [SimplifiedChinese, Japanese]
        );
        assert_eq!(
            system_fonts(TextEncoding::Utf8),
            [SimplifiedChinese, Japanese]
        );
    }
}

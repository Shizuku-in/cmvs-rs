//! The game text encoding: `encoding.txt` in the game directory, or detection from the
//! game scripts when that file is absent; and the system fonts its text needs.
use cmvs_core::TextEncoding;
use cmvs_font::text_codec;
use std::{error::Error, io::ErrorKind, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// An explicit game setting overrides bounded detection from startup and archived scripts.
pub fn select(
    root: &Path,
    files: &cmvs_core::profile::GameFiles,
    startup: &cmvs_core::ps3::Script,
) -> Result<TextEncoding> {
    match std::fs::read_to_string(root.join("encoding.txt")) {
        Ok(setting) => {
            let name = setting.trim();
            text_codec::parse(name).map_err(|_| {
                format!("encoding.txt must be shift-jis, gbk or utf-8, not {name:?}").into()
            })
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            text_codec::detect_game(root, files, startup).map_err(|error| {
                if error.downcast_ref::<cmvs_core::FormatError>() == Some(&text_codec::UNCERTAIN) {
                    "cannot detect the game text encoding; put encoding.txt holding shift-jis, gbk or utf-8 in the game directory".into()
                } else {
                    error
                }
            })
        }
        Err(error) => Err(error.into()),
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
    fn explicit_setting_bypasses_missing_archive_and_uncertain_text() {
        let root =
            std::env::temp_dir().join(format!("cmvs-console-encoding-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let files = cmvs_core::profile::GameFiles {
            start_script: "start.ps3".into(),
            script_archive: "missing.cpz".into(),
        };
        let mut raw = b"PS2A".to_vec();
        for word in [48u32, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend([0, 0x20]);
        let startup = cmvs_core::ps3::Script::from_memory_image(&raw).unwrap();
        for (name, expected) in [
            ("gbk\n", TextEncoding::Gbk),
            ("utf-8\r\n", TextEncoding::Utf8),
            ("shift-jis", TextEncoding::ShiftJis),
        ] {
            std::fs::write(root.join("encoding.txt"), name).unwrap();
            assert_eq!(select(&root, &files, &startup).unwrap(), expected);
        }
        std::fs::write(root.join("encoding.txt"), "auto").unwrap();
        assert!(
            select(&root, &files, &startup)
                .unwrap_err()
                .to_string()
                .contains("\"auto\"")
        );
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

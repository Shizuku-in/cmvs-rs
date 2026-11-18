//! The game text encoding: `encoding.txt` or `conversion.json` in the game directory, or
//! detection from the game scripts when those files are absent; and the system fonts its text needs.
use cmvs_core::TextEncoding;
use cmvs_host::{EncodingPolicy, resolve_encoding_with_fs};
use std::{error::Error, io::ErrorKind, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// Error message when a console game lacks `encoding.txt`, `conversion.json`, and decisive script text.
pub const UNCERTAIN_ENCODING_MESSAGE: &str = "cannot detect the game text encoding; put encoding.txt holding shift-jis, gbk or utf-8 in the game directory";

/// An explicit game setting (`encoding.txt` or `conversion.json`) overrides bounded detection
/// from startup and archived scripts.
pub fn select(
    root: &Path,
    files: &cmvs_core::profile::GameFiles,
    startup: &cmvs_core::format::ps3::Script,
) -> Result<TextEncoding> {
    resolve_encoding_with_fs(
        &crate::ConsoleFs,
        root,
        files,
        startup,
        EncodingPolicy::AutoStrict(UNCERTAIN_ENCODING_MESSAGE),
    )
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

/// Loads the host UI font from `base/font.ttf` or falls back to the console's system font.
pub fn load_host_font(
    base: &Path,
    system_fallback: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<ab_glyph::FontVec> {
    let bytes = match std::fs::read(base.join("font.ttf")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => system_fallback()?,
        Err(error) => return Err(error.into()),
    };
    Ok(ab_glyph::FontVec::try_from_vec_and_index(bytes, 0).map_err(|_| "invalid font")?)
}

/// Loads `base/font.ttf` alone when present; otherwise loads the encoding's system fonts,
/// installing subsequent available faces as fallbacks.
pub fn load_game_font(
    base: &Path,
    encoding: TextEncoding,
    missing_message: &'static str,
    mut load_system: impl FnMut(SystemFont) -> Option<Vec<u8>>,
) -> Result<cmvs_font::font::FontRenderer> {
    let renderer = |bytes| -> Result<cmvs_font::font::FontRenderer> {
        Ok(cmvs_font::font::FontRenderer::from_bytes(bytes, 0)?.with_text_encoding(encoding))
    };
    match std::fs::read(base.join("font.ttf")) {
        Ok(bytes) => return renderer(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut fonts = system_fonts(encoding)
        .iter()
        .filter_map(|&font| load_system(font));
    let font = renderer(fonts.next().ok_or(missing_message)?)?;
    for bytes in fonts {
        font.add_fallback(bytes, 0)?;
    }
    Ok(font)
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
        let startup = cmvs_core::format::ps3::Script::from_memory_image(&raw).unwrap();
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
        std::fs::remove_file(root.join("encoding.txt")).unwrap();
        std::fs::write(
            root.join("conversion.json"),
            br#"{"game":"amatsutsumi","encoding":"utf-8"}"#,
        )
        .unwrap();
        assert_eq!(select(&root, &files, &startup).unwrap(), TextEncoding::Utf8);
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

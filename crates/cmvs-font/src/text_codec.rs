//! Host-side game text codecs: decoding and encoding host strings in the game encoding,
//! and inferring that encoding from script text. Core's byte scanners stay
//! dependency-free; the codec tables live here beside the glyph decoding that shares them.
use cmvs_core::{FormatError, TextEncoding};

/// The encoding_rs codec of a game encoding.
pub fn codec(encoding: TextEncoding) -> Result<&'static encoding_rs::Encoding, FormatError> {
    crate::font::codec(encoding)
}

/// The user-facing name: `shift-jis`, `gbk` or `utf-8`.
pub fn name(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::ShiftJis => "shift-jis",
        TextEncoding::Gbk => "gbk",
        TextEncoding::Utf8 => "utf-8",
        _ => "unknown",
    }
}

/// Parses a [`name`].
pub fn parse(name: &str) -> Result<TextEncoding, FormatError> {
    match name {
        "shift-jis" => Ok(TextEncoding::ShiftJis),
        "gbk" => Ok(TextEncoding::Gbk),
        "utf-8" => Ok(TextEncoding::Utf8),
        _ => Err(FormatError("encoding must be shift-jis, gbk or utf-8")),
    }
}

/// Strict decoding: invalid bytes are an error, never replacement characters.
pub fn decode(encoding: TextEncoding, bytes: &[u8]) -> Result<String, FormatError> {
    codec(encoding)?
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|s| s.into_owned())
        .ok_or(FormatError("text is not valid in the game encoding"))
}

/// Strict encoding: an unrepresentable character is an error.
pub fn encode(encoding: TextEncoding, text: &str) -> Result<Vec<u8>, FormatError> {
    let (bytes, _, unmappable) = codec(encoding)?.encode(text);
    if unmappable {
        return Err(FormatError(
            "text is not representable in the game encoding",
        ));
    }
    Ok(bytes.into_owned())
}

/// Characters of `text` the encoding cannot represent, in order, without repeats.
pub fn unmappable(encoding: TextEncoding, text: &str) -> Result<String, FormatError> {
    let codec = codec(encoding)?;
    let mut result = String::new();
    for ch in text.chars() {
        let mut buffer = [0; 4];
        if codec.encode(ch.encode_utf8(&mut buffer)).2 && !result.contains(ch) {
            result.push(ch);
        }
    }
    Ok(result)
}

/// Detection found no reliable evidence; the host must ask for an explicit encoding.
pub const UNCERTAIN: FormatError =
    FormatError("cannot reliably detect the game text encoding from script text");

/// Try startup text first, then at most 32 archived scripts in archive order.
/// Fallback reads and decoded payloads each share a 16 MiB budget; uncertain
/// samples never select a default encoding. Archive and script errors propagate.
pub fn detect_game(
    root: &std::path::Path,
    files: &cmvs_core::profile::GameFiles,
    startup: &cmvs_core::ps3::Script,
) -> Result<TextEncoding, Box<dyn std::error::Error>> {
    use cmvs_core::{
        filesystem::File,
        ps3::{Header, Script},
        script_control::archive_index_from_start,
    };
    if let Ok(encoding) = detect(startup.text()) {
        return Ok(encoding);
    }
    let mut reader = File::open(root.join(&files.script_archive))?;
    let basename = files.script_archive.file_name().ok_or(UNCERTAIN)?;
    let index = archive_index_from_start(&mut reader, startup, basename.as_encoded_bytes())?;
    let mut stored_budget = 16 * 1024 * 1024;
    let mut decoded_budget = stored_budget;
    let mut scripts = 0;
    for (i, entry) in index.entries().iter().enumerate() {
        let extension = entry.name.rsplit(|&b| b == b'.').next().unwrap_or_default();
        if !extension.eq_ignore_ascii_case(b"ps3") && !extension.eq_ignore_ascii_case(b"ps2") {
            continue;
        }
        if scripts == 32 {
            break;
        }
        scripts += 1;
        let size = entry.size as usize;
        if size > stored_budget {
            continue;
        }
        stored_budget -= size;
        let raw = index.read_entry(&mut reader, i)?;
        let header = Header::parse(&raw)?;
        let decoded = header.decoded_size as usize;
        if decoded > decoded_budget {
            continue;
        }
        decoded_budget -= decoded;
        let script = Script::decode(&raw)?;
        if let Ok(encoding) = detect(script.text()) {
            return Ok(encoding);
        }
    }
    Err(UNCERTAIN.into())
}

#[derive(Default)]
struct Evidence {
    non_ascii: usize,
    kana: usize,
    halfwidth: usize,
    han: usize,
}

fn evidence(encoding: TextEncoding, samples: &[&[u8]]) -> Option<Evidence> {
    let mut result = Evidence::default();
    for bytes in samples {
        let text = decode(encoding, bytes).ok()?;
        for ch in text.chars().filter(|ch| !ch.is_ascii()) {
            result.non_ascii += 1;
            result.kana +=
                usize::from(matches!(ch, '\u{3041}'..='\u{3096}' | '\u{30a1}'..='\u{30fa}'));
            result.halfwidth += usize::from(matches!(ch, '\u{ff61}'..='\u{ff9f}'));
            result.han += usize::from(matches!(ch, '\u{3400}'..='\u{9fff}'));
        }
    }
    Some(result)
}

/// Infers the game encoding from the startup script's NUL-terminated text section, or
/// returns [`UNCERTAIN`].
pub fn detect(text: &[u8]) -> Result<TextEncoding, FormatError> {
    const LIMIT: usize = 256 * 1024;
    // Keep string boundaries: truncating a multibyte character would invalidate a
    // candidate. ASCII resource names provide no evidence for any encoding.
    let mut samples = Vec::new();
    let mut total = 0;
    for bytes in text.split_inclusive(|&b| b == 0) {
        if total + bytes.len() > LIMIT {
            break;
        }
        total += bytes.len();
        let bytes = bytes.strip_suffix(&[0]).ok_or(UNCERTAIN)?;
        if !bytes.is_ascii() {
            samples.push(bytes);
        }
    }
    if samples.is_empty() {
        return Err(UNCERTAIN);
    }
    let sjis = evidence(TextEncoding::ShiftJis, &samples);
    let gbk = evidence(TextEncoding::Gbk, &samples);
    let utf8 = evidence(TextEncoding::Utf8, &samples);
    // Short UTF-8 sequences can also be valid legacy text. Require either a
    // unique candidate or enough non-ASCII text before preferring UTF-8.
    if let Some(utf8) = utf8 {
        return if (sjis.is_none() && gbk.is_none()) || utf8.non_ascii >= 8 {
            Ok(TextEncoding::Utf8)
        } else {
            Err(UNCERTAIN)
        };
    }
    match (sjis, gbk) {
        (Some(_), None) => Ok(TextEncoding::ShiftJis),
        (None, Some(_)) => Ok(TextEncoding::Gbk),
        (Some(sjis), Some(gbk)) => {
            // Legacy encodings overlap. Require substantial language evidence,
            // rather than treating successful decoding as identification.
            if sjis.kana >= 8 && sjis.kana * 5 >= sjis.non_ascii && gbk.kana * 4 < sjis.kana {
                Ok(TextEncoding::ShiftJis)
            } else if gbk.han >= 8
                && gbk.han * 10 >= gbk.non_ascii * 8
                && sjis.halfwidth >= 8
                && sjis.halfwidth * 2 >= sjis.non_ascii
                && sjis.kana * 10 < sjis.non_ascii
            {
                Ok(TextEncoding::Gbk)
            } else {
                Err(UNCERTAIN)
            }
        }
        (None, None) => Err(UNCERTAIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_representative_text_in_each_encoding() {
        for (encoding, text) in [
            (
                TextEncoding::ShiftJis,
                "これは文字コードを自動で判定するための日本語の文章です。",
            ),
            (
                TextEncoding::Gbk,
                "这是用于自动识别文字编码的中文句子，请检查显示是否正确。",
            ),
            (TextEncoding::Utf8, "「日本語」と“中文”のUTF-8テキスト"),
        ] {
            let mut bytes = b"menu.ps3\0bg001\0".to_vec();
            bytes.extend(encode(encoding, text).unwrap());
            bytes.push(0);
            assert_eq!(detect(&bytes).unwrap(), encoding);
        }
    }

    #[test]
    fn uncertain_or_invalid_text_is_not_guessed() {
        for bytes in [
            b"".as_slice(),
            b"menu.ps3\0",
            b"\xff\0",
            b"\x93\xfa\0",
            b"\xc2\xa1\0",
            b"\x82\xa0",
        ] {
            assert_eq!(detect(bytes), Err(UNCERTAIN), "{bytes:?}");
        }
        let mut over_limit = vec![b'a'; 256 * 1024];
        over_limit.extend_from_slice(b"\x82\xa0\0");
        assert_eq!(detect(&over_limit), Err(UNCERTAIN));
    }

    fn script_bytes(text: &[u8]) -> Vec<u8> {
        let size = text.len() as u32;
        let mut raw = b"PS2A".to_vec();
        for word in [48, 0, 0, 0, 2, 0, size, 0, 0, size + 2, 0] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(
            [0u8, 0x20]
                .iter()
                .chain(text)
                .map(|b| b.rotate_left(1).wrapping_add(124)),
        );
        raw
    }

    #[test]
    fn fallback_reads_archived_scripts_without_guessing_ascii_or_binary() {
        use cmvs_core::{
            cpz::{ArchiveDirectory, ArchiveFile, Cpz6Index},
            profile::GameFiles,
            ps3::Script,
        };
        use std::{fs, io::Cursor};
        let root = std::env::temp_dir().join(format!("cmvs-font-encoding-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let files = GameFiles {
            start_script: "start.ps3".into(),
            script_archive: "script.cpz".into(),
        };
        let startup = Script::decode(&script_bytes(b"menu.ps3\0")).unwrap();
        for (encoding, text) in [
            (
                TextEncoding::ShiftJis,
                "これは文字コードを自動で判定するための日本語の文章です。",
            ),
            (
                TextEncoding::Gbk,
                "这是用于自动识别文字编码的中文句子，请检查显示是否正确。",
            ),
            (TextEncoding::Utf8, "「日本語」と“中文”のUTF-8テキスト"),
        ] {
            let mut text = encode(encoding, text).unwrap();
            text.push(0);
            let mut archive = Cursor::new(Vec::new());
            Cpz6Index::write(
                &mut archive,
                &[ArchiveDirectory {
                    name: Vec::new(),
                    files: vec![
                        ArchiveFile {
                            name: b"binary.dat".to_vec(),
                            data: vec![0xff],
                        },
                        ArchiveFile {
                            name: b"menu.ps3".to_vec(),
                            data: script_bytes(b"logo.pb3\0"),
                        },
                        ArchiveFile {
                            name: b"story.PS3".to_vec(),
                            data: script_bytes(&text),
                        },
                    ],
                }],
            )
            .unwrap();
            fs::write(root.join(&files.script_archive), archive.into_inner()).unwrap();
            assert_eq!(detect_game(&root, &files, &startup).unwrap(), encoding);
            // Runtime evidence takes priority, without opening the archive.
            let strong_start = Script::decode(&script_bytes(&text)).unwrap();
            let empty = GameFiles {
                script_archive: "missing.cpz".into(),
                ..files.clone()
            };
            assert_eq!(detect_game(&root, &empty, &strong_start).unwrap(), encoding);
        }
        let mut archive = Cursor::new(Vec::new());
        Cpz6Index::write(
            &mut archive,
            &[ArchiveDirectory {
                name: Vec::new(),
                files: (0..32)
                    .map(|i| ArchiveFile {
                        name: format!("{i}.ps3").into_bytes(),
                        data: script_bytes(b"menu.ps3\0"),
                    })
                    .chain(std::iter::once(ArchiveFile {
                        name: b"beyond-limit.ps3".to_vec(),
                        data: script_bytes("日本語の文章をここから開始します。\0".as_bytes()),
                    }))
                    .collect(),
            }],
        )
        .unwrap();
        fs::write(root.join(&files.script_archive), archive.into_inner()).unwrap();
        let error = detect_game(&root, &files, &startup).unwrap_err();
        assert_eq!(error.downcast_ref::<FormatError>(), Some(&UNCERTAIN));
        fs::write(root.join(&files.script_archive), b"broken archive").unwrap();
        let error = detect_game(&root, &files, &startup).unwrap_err();
        assert_ne!(error.downcast_ref::<FormatError>(), Some(&UNCERTAIN));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_round_trip_and_codecs_are_strict() {
        for encoding in [
            TextEncoding::ShiftJis,
            TextEncoding::Gbk,
            TextEncoding::Utf8,
        ] {
            assert_eq!(parse(name(encoding)).unwrap(), encoding);
        }
        assert!(parse("auto").is_err());
        assert_eq!(decode(TextEncoding::Gbk, b"\xd6\xd0").unwrap(), "中");
        assert!(decode(TextEncoding::ShiftJis, b"\x82").is_err());
        assert!(encode(TextEncoding::ShiftJis, "这").is_err());
        assert_eq!(
            unmappable(TextEncoding::ShiftJis, "日这这😀").unwrap(),
            "这😀"
        );
        assert_eq!(unmappable(TextEncoding::Utf8, "这😀").unwrap(), "");
    }
}

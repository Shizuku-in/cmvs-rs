//! Startup-only encoding selection. VM text and glyph semantics stay unchanged.
use crate::TextEncoding;
use cmvs_core::FormatError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodingChoice {
    Auto,
    Explicit(TextEncoding),
}

impl EncodingChoice {
    pub(crate) fn parse(name: &str) -> Result<Self, FormatError> {
        if name == "auto" {
            Ok(Self::Auto)
        } else {
            TextEncoding::parse(name)
                .map(Self::Explicit)
                .map_err(|_| FormatError("encoding must be auto, shift-jis, gbk or utf-8"))
        }
    }

    pub(crate) fn resolve(self, text: &[u8]) -> Result<TextEncoding, FormatError> {
        match self {
            Self::Auto => detect(text),
            Self::Explicit(encoding) => Ok(encoding),
        }
    }
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
        let text = encoding.decode(bytes).ok()?;
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

fn detect(text: &[u8]) -> Result<TextEncoding, FormatError> {
    const LIMIT: usize = 256 * 1024;
    const UNCERTAIN: FormatError = FormatError(
        "cannot reliably detect startup text encoding; select --encoding shift-jis, gbk or utf-8 explicitly",
    );
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
    fn auto_detects_representative_text_in_each_encoding() {
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
            bytes.extend(encoding.encode(text).unwrap());
            bytes.push(0);
            assert_eq!(EncodingChoice::Auto.resolve(&bytes).unwrap(), encoding);
        }
    }

    #[test]
    fn uncertain_or_invalid_text_requires_explicit_selection() {
        for bytes in [
            b"".as_slice(),
            b"menu.ps3\0",
            b"\xff\0",
            b"\x93\xfa\0",
            b"\xc2\xa1\0",
            b"\x82\xa0",
        ] {
            assert!(EncodingChoice::Auto.resolve(bytes).is_err(), "{bytes:?}");
        }
        let mut over_limit = vec![b'a'; 256 * 1024];
        over_limit.extend_from_slice(b"\x82\xa0\0");
        assert!(EncodingChoice::Auto.resolve(&over_limit).is_err());
    }

    #[test]
    fn explicit_selection_bypasses_detection() {
        assert_eq!(EncodingChoice::parse("auto").unwrap(), EncodingChoice::Auto);
        for name in ["shift-jis", "gbk", "utf-8"] {
            let expected = TextEncoding::parse(name).unwrap();
            assert_eq!(
                EncodingChoice::parse(name)
                    .unwrap()
                    .resolve(b"\xff")
                    .unwrap(),
                expected
            );
        }
        assert!(EncodingChoice::parse("aut").is_err());
    }
}

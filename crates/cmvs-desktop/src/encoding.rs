//! Runtime encoding selection shared by host text, VM scanning and font decoding.
use crate::{TextEncoding, text_codec};
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
            text_codec::parse(name)
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

/// The shared startup-script detection, with the desktop's command-line remedy.
fn detect(text: &[u8]) -> Result<TextEncoding, FormatError> {
    text_codec::detect(text).map_err(|_| FormatError(
        "cannot reliably detect startup text encoding; select --encoding shift-jis, gbk or utf-8 explicitly",
    ))
}

/// The legacy built-in face name remains Shift-JIS even for patched game encodings.
pub(crate) fn decode_font_face(
    encoding: TextEncoding,
    bytes: &[u8],
) -> Result<String, FormatError> {
    if bytes == cmvs_core::settings::DEFAULT_FONT_FACE {
        return Ok("ＭＳ ゴシック".to_owned());
    }
    text_codec::decode(encoding, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_selection_bypasses_detection() {
        assert_eq!(EncodingChoice::parse("auto").unwrap(), EncodingChoice::Auto);
        for name in ["shift-jis", "gbk", "utf-8"] {
            let expected = text_codec::parse(name).unwrap();
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

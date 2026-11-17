//! Runtime encoding selection shared by host text, VM scanning and font decoding.
use crate::{TextEncoding, text_codec};
use cmvs_core::FormatError;
use cmvs_host::EncodingPolicy;

pub(crate) const UNCERTAIN_DESKTOP_ENCODING: &str = "cannot reliably detect startup text encoding; select --encoding shift-jis, gbk or utf-8 explicitly";

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

    pub(crate) fn into_policy(self) -> EncodingPolicy {
        match self {
            Self::Auto => EncodingPolicy::AutoStrict(UNCERTAIN_DESKTOP_ENCODING),
            Self::Explicit(encoding) => EncodingPolicy::Explicit(encoding),
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve(self, text: &[u8]) -> Result<TextEncoding, FormatError> {
        match self {
            Self::Auto => {
                text_codec::detect(text).map_err(|_| FormatError(UNCERTAIN_DESKTOP_ENCODING))
            }
            Self::Explicit(encoding) => Ok(encoding),
        }
    }
}

/// The legacy built-in face name remains Shift-JIS even for patched game encodings.
pub(crate) fn decode_font_face(
    encoding: TextEncoding,
    bytes: &[u8],
) -> Result<String, FormatError> {
    if bytes == cmvs_core::host::settings::DEFAULT_FONT_FACE {
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
        assert_eq!(
            EncodingChoice::Auto.into_policy(),
            EncodingPolicy::AutoStrict(UNCERTAIN_DESKTOP_ENCODING)
        );
        for name in ["shift-jis", "gbk", "utf-8"] {
            let expected = text_codec::parse(name).unwrap();
            let choice = EncodingChoice::parse(name).unwrap();
            assert_eq!(choice.resolve(b"\xff").unwrap(), expected);
            assert_eq!(choice.into_policy(), EncodingPolicy::Explicit(expected));
        }
        assert!(EncodingChoice::parse("aut").is_err());
    }
}

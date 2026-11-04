//! Shared protocol validation for external Unicode comment editors.
use crate::NativeError;

pub(crate) fn validate_request(
    title: &str,
    prompt: &str,
    initial: &str,
) -> Result<(), NativeError> {
    if [title, prompt, initial]
        .iter()
        .any(|s| s.len() > 4096 || s.contains('\0'))
    {
        return Err(NativeError::Failed(
            "text dialog request is too long or contains NUL".into(),
        ));
    }
    Ok(())
}

/// Helpers return one line of UTF-8. Remove only the protocol line ending,
/// preserving comment spaces; an empty accepted string differs from cancel.
pub(crate) fn decode_text(bytes: &[u8], max_chars: u32) -> Result<String, NativeError> {
    if bytes.len() > 4096 {
        return Err(NativeError::Failed(
            "text dialog returned excessive output".into(),
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| NativeError::Failed("text dialog returned invalid UTF-8".into()))?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.chars().any(char::is_control) {
        return Err(NativeError::Failed(
            "text dialog returned control characters".into(),
        ));
    }
    // A Unicode boundary, never a partial UTF-8 character. The VM then applies
    // its own game-encoding capacity checks and original full-width filter.
    Ok(text.chars().take(max_chars.min(255) as usize).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_spaces_empty_accept_and_unicode_boundaries() {
        assert_eq!(
            decode_text(" あいう  \n".as_bytes(), 50).unwrap(),
            " あいう  "
        );
        assert_eq!(decode_text(b"\n", 50).unwrap(), "");
        assert_eq!(decode_text("あいう\r\n".as_bytes(), 2).unwrap(), "あい");
        for bytes in [b"a\nb".as_slice(), b"bad\0", b"\xff"] {
            assert!(decode_text(bytes, 50).is_err());
        }
        assert!(decode_text(&[b'x'; 4097], 50).is_err());
        assert!(validate_request("x\0", "", "").is_err());
    }
}

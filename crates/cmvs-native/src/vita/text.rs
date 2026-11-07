//! Text limits of the system dialogs, kept portable so they test off-device.

/// At most `units` UTF-16 code units of `text`, never splitting a character, NUL-ended.
pub(super) fn utf16(text: &str, units: usize) -> Vec<u16> {
    let mut out = Vec::new();
    for ch in text.chars().filter(|&ch| ch != '\0') {
        let mut buffer = [0; 2];
        let encoded = ch.encode_utf16(&mut buffer);
        if out.len() + encoded.len() > units {
            break;
        }
        out.extend_from_slice(encoded);
    }
    out.push(0);
    out
}

pub(super) fn truncate(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_never_split_characters() {
        assert_eq!(utf16("a😀b", 2), [u16::from(b'a'), 0]);
        assert_eq!(utf16("a\0b", 8), [u16::from(b'a'), u16::from(b'b'), 0]);
        assert_eq!(truncate("あい", 4), "あ");
    }
}

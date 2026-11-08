//! Game text encoding, independent of archive names and resource keys.
/// Runtime choice, separate from the executable profile and resource formats.
///
/// Shift-JIS and GBK follow the original single/double-byte engine contract: a lead
/// byte and the following byte form one character whose glyph code is the big-endian
/// pair. UTF-8 is an extension without an original engine: ASCII stays single-byte and
/// every multi-byte sequence is one full-width character whose glyph code is its
/// Unicode scalar value, which must lie in the Basic Multilingual Plane.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextEncoding {
    #[default]
    ShiftJis,
    Gbk,
    Utf8,
}

/// Unicode `、。」』`, the UTF-8 counterparts of the fixed Shift-JIS no-break codes.
const UTF8_NO_BREAK: [u16; 4] = [0x3001, 0x3002, 0x300d, 0x300f];
/// The fixed Shift-JIS no-break codes 、。」』, also used by GBK-patched executables.
const LEGACY_NO_BREAK: [u16; 4] = [0x8141, 0x8142, 0x8176, 0x8178];

impl TextEncoding {
    /// Lead-byte test: whether `byte` starts a wide (full-width) character. Shift-JIS
    /// and GBK deliberately do not validate the trail byte, as in the engine scanners.
    pub const fn is_lead_byte(self, byte: u8) -> bool {
        self.wide_len(byte).is_some()
    }

    /// Encoded length of the wide character starting with `byte`, or `None` for a
    /// single-byte character. A UTF-8 byte that cannot start a sequence counts as a
    /// one-byte wide character, which [`Self::wide_code`] then rejects.
    pub const fn wide_len(self, byte: u8) -> Option<usize> {
        match self {
            Self::ShiftJis if matches!(byte, 0x81..=0x9f | 0xe0..=0xfc) => Some(2),
            Self::Gbk if byte >= 0x80 => Some(2),
            Self::ShiftJis | Self::Gbk => None,
            Self::Utf8 => match byte {
                0x00..=0x7f => None,
                0xc0..=0xdf => Some(2),
                0xe0..=0xef => Some(3),
                0xf0..=0xf7 => Some(4),
                _ => Some(1),
            },
        }
    }

    /// The `u16` glyph code of one complete wide character of [`Self::wide_len`] bytes.
    /// Shift-JIS and GBK combine the pair unvalidated; UTF-8 must be a valid scalar in the
    /// Basic Multilingual Plane.
    pub fn wide_code(self, bytes: &[u8]) -> Result<u16, &'static str> {
        match self {
            Self::ShiftJis | Self::Gbk => match *bytes {
                [lead, trail] => Ok(u16::from_be_bytes([lead, trail])),
                _ => Err("double-byte character must be two bytes"),
            },
            Self::Utf8 => {
                let text = core::str::from_utf8(bytes).map_err(|_| "invalid UTF-8 character")?;
                let mut chars = text.chars();
                let (Some(ch), None) = (chars.next(), chars.next()) else {
                    return Err("invalid UTF-8 character");
                };
                u16::try_from(u32::from(ch))
                    .map_err(|_| "UTF-8 character outside the Basic Multilingual Plane")
            }
        }
    }

    /// The wide character at `bytes[i]` as its glyph code and encoded length, or `None`
    /// for a single byte (or the end of `bytes`). Every trail byte must be present and
    /// non-NUL; `truncated` names that failure.
    pub(crate) fn wide_at(
        self,
        bytes: &[u8],
        i: usize,
        truncated: &'static str,
    ) -> Result<Option<(u16, usize)>, &'static str> {
        let Some(len) = bytes.get(i).and_then(|&lead| self.wide_len(lead)) else {
            return Ok(None);
        };
        let pair = bytes
            .get(i..i + len)
            .filter(|pair| !pair[1..].contains(&0))
            .ok_or(truncated)?;
        Ok(Some((self.wide_code(pair)?, len)))
    }

    /// Like [`Self::wide_at`] for scanners that read past the terminator as NUL: a
    /// Shift-JIS or GBK pair takes the next byte (0 beyond `bytes`) unchecked, as in the
    /// original; a UTF-8 sequence must be complete.
    pub(crate) fn wide_at_unchecked(
        self,
        bytes: &[u8],
        i: usize,
    ) -> Result<Option<(u16, usize)>, &'static str> {
        let byte = |i: usize| bytes.get(i).copied().unwrap_or(0);
        match self.wide_len(byte(i)) {
            None => Ok(None),
            Some(2) if self != Self::Utf8 => {
                Ok(Some((u16::from_be_bytes([byte(i), byte(i + 1)]), 2)))
            }
            Some(len) => {
                let sequence = bytes.get(i..i + len).ok_or("truncated UTF-8 character")?;
                Ok(Some((self.wide_code(sequence)?, len)))
            }
        }
    }

    /// Whether a glyph code is a full-width cell. Legacy single bytes stay below 0x100 or
    /// are sign-extended to 0xff80 and above; UTF-8 single bytes are ASCII.
    pub const fn is_wide_code(self, code: u16) -> bool {
        match self {
            Self::ShiftJis | Self::Gbk => code > 0xff && code < 0xff80,
            Self::Utf8 => code >= 0x80,
        }
    }

    /// 、。」』 that never start a line without 0x211e rules. Shift-JIS and GBK keep the
    /// original executable's fixed Shift-JIS immediates; UTF-8 uses the Unicode characters.
    pub const fn no_break_codes(self) -> [u16; 4] {
        match self {
            Self::ShiftJis | Self::Gbk => LEGACY_NO_BREAK,
            Self::Utf8 => UTF8_NO_BREAK,
        }
    }

    /// ゛, drawn a fifth of the size to the left with a reduced advance.
    pub const fn voiced_mark(self) -> u16 {
        match self {
            Self::ShiftJis | Self::Gbk => 0x814a,
            Self::Utf8 => 0x309b,
        }
    }

    /// ・, the proportional ruby centring point.
    pub const fn middle_dot(self) -> u16 {
        match self {
            Self::ShiftJis | Self::Gbk => 0x8145,
            Self::Utf8 => 0x30fb,
        }
    }

    /// Full-width space between the base and scene titles (0x1400de070 in Shift-JIS).
    pub const fn title_separator(self) -> &'static [u8] {
        match self {
            Self::ShiftJis => &[0x81, 0x40],
            Self::Gbk => &[0xa1, 0xa1],
            Self::Utf8 => "\u{3000}".as_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextEncoding;

    #[test]
    fn lead_byte_boundaries_match_both_engine_variants() {
        for byte in 0..=u8::MAX {
            assert_eq!(
                TextEncoding::ShiftJis.is_lead_byte(byte),
                (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte)
            );
            assert_eq!(TextEncoding::Gbk.is_lead_byte(byte), byte >= 0x80);
            assert_eq!(TextEncoding::Utf8.is_lead_byte(byte), byte >= 0x80);
        }
    }

    #[test]
    fn utf8_wide_characters_map_to_bmp_scalars() {
        let utf8 = TextEncoding::Utf8;
        for ch in ['é', 'あ', '中', '\u{ffef}'] {
            let mut buffer = [0; 4];
            let bytes = ch.encode_utf8(&mut buffer).as_bytes();
            assert_eq!(utf8.wide_len(bytes[0]), Some(bytes.len()));
            assert_eq!(utf8.wide_code(bytes), Ok(ch as u16));
            assert!(utf8.is_wide_code(ch as u16));
        }
        assert_eq!(utf8.wide_len(b'A'), None);
        assert!(!utf8.is_wide_code(u16::from(b'A')));
        for invalid in [
            &b"\x80"[..],
            b"\xc0\x80",
            b"\xe3\x81",
            b"\xe3\x00\x82",
            b"\xff",
        ] {
            assert!(utf8.wide_code(invalid).is_err(), "{invalid:?}");
        }
        assert!(utf8.wide_code("😀".as_bytes()).is_err());
        for (encoding, code) in [
            (TextEncoding::ShiftJis, 0x82a0),
            (TextEncoding::Gbk, 0xd6d0),
        ] {
            assert_eq!(encoding.wide_code(&u16::to_be_bytes(code)), Ok(code));
            assert!(encoding.is_wide_code(code));
            assert!(!encoding.is_wide_code(0xffb1));
        }
    }

    #[test]
    fn utf8_fixed_codes_are_the_unicode_characters() {
        let utf8 = TextEncoding::Utf8;
        let code = |s: &str| s.chars().next().unwrap() as u16;
        assert_eq!(utf8.no_break_codes(), ["、", "。", "」", "』"].map(code));
        assert_eq!(utf8.voiced_mark(), code("゛"));
        assert_eq!(utf8.middle_dot(), code("・"));
        assert_eq!(utf8.title_separator(), "　".as_bytes());
    }
}

//! Game text encoding, independent of archive names and resource keys.
/// Runtime choice, separate from the executable profile and resource formats.
/// More encodings may be added; the current scanners implement the original
/// single/double-byte engine contract. A variable-width encoding also needs a
/// scanner and a mapping into the engine's `u16` glyph domain.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextEncoding {
    #[default]
    ShiftJis,
    Gbk,
}

impl TextEncoding {
    /// Lead-byte test for the supported single/double-byte engine scanners.
    /// Deliberately does not validate the trail byte. This is not a general
    /// character-length or Unicode decoding API.
    pub const fn is_lead_byte(self, byte: u8) -> bool {
        match self {
            Self::ShiftJis => matches!(byte, 0x81..=0x9f | 0xe0..=0xfc),
            Self::Gbk => byte >= 0x80,
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
        }
    }
}

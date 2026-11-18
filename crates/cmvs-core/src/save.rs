//! CSV2 and CVNS CSP1 disk containers and gameplay-save header metadata.
//! Decoded sections are not a restored engine state.
use crate::{FormatError, MAX_PAYLOAD, lzss};
use std::ops::Range;

pub const HEADER_SIZE: usize = 600;
pub const HEADER_READ_LIMIT: usize = 512;
const CMVS_KEY: &[u8] = b"DAME_SAVE_IMAGE";
const CVNS_KEY: &[u8] = b"SAVE_IMAGE_ARC";

/// The gameplay-save signature a menu scan accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFamily {
    /// CMVS `CSV2`..`CSV9`; slot N is `saveNNN.dat`.
    Cmvs,
    /// CVNS `CSP1`..`CSP9` (cvns-20090203 0x46d5f0); slot N is `save(N+1).dat`, because
    /// `save000.dat` holds the CSS0 system save.
    Cvns,
}

impl SaveFamily {
    pub(crate) fn accepts(self, bytes: &[u8]) -> bool {
        let (prefix, first) = match self {
            Self::Cmvs => (b"CSV", b'2'),
            Self::Cvns => (b"CSP", b'1'),
        };
        bytes.len() >= 4 && &bytes[..3] == prefix && (first..=b'9').contains(&bytes[3])
    }
    pub(crate) fn file(self, slot: u16) -> u16 {
        match self {
            Self::Cmvs => slot,
            Self::Cvns => slot + 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveHeader {
    /// ASCII version digit: '2'..='9' for CMVS, '1'..='9' for CVNS.
    pub version: u8,
    /// Original bytes at offset 16, excluding the terminating NUL.
    pub label: Vec<u8>,
    /// Original ordering key; only the last two year digits and even seconds count.
    pub ordering_key: u32,
}

impl SaveHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::parse_family(bytes, SaveFamily::Cmvs)
    }
    pub fn parse_family(bytes: &[u8], family: SaveFamily) -> Result<Self, FormatError> {
        let bytes = &bytes[..bytes.len().min(HEADER_READ_LIMIT)];
        if !family.accepts(bytes) {
            return Err(FormatError("unsupported gameplay-save signature"));
        }
        let label = bytes
            .get(16..)
            .and_then(|b| b.iter().position(|v| *v == 0).map(|end| &b[..end]))
            .ok_or(FormatError("save label is truncated or not NUL terminated"))?;
        // Explicit malformed-input policy: no stale read-buffer bytes or unbounded C strings.
        if label.len() < 19
            || label[4] != b'-'
            || label[7] != b'-'
            || label[10] != b' '
            || label[13] != b':'
            || label[16] != b':'
            || !label[..19]
                .iter()
                .enumerate()
                .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit())
        {
            return Err(FormatError("save label lacks YYYY-MM-DD HH:MM:SS prefix"));
        }
        let pair = |i: usize| u32::from(label[i] - b'0') * 10 + u32::from(label[i + 1] - b'0');
        let ordering_key = pair(2).wrapping_shl(25)
            | ((pair(5) & 15) << 21)
            | ((pair(8) & 31) << 16)
            | ((pair(11) & 31) << 11)
            | ((pair(14) & 63) << 5)
            | ((pair(17) / 2) & 31);
        Ok(Self {
            version: bytes[3],
            label: label.to_vec(),
            ordering_key,
        })
    }
}

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().expect("validated header"))
}
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated header"))
}
fn key_for(family: SaveFamily) -> &'static [u8] {
    match family {
        SaveFamily::Cmvs => CMVS_KEY,
        SaveFamily::Cvns => CVNS_KEY,
    }
}
fn header_matches(bytes: &[u8], family: SaveFamily) -> bool {
    match family {
        SaveFamily::Cmvs => &bytes[..4] == b"CSV2",
        SaveFamily::Cvns => &bytes[..3] == b"CSP" && (b'1'..=b'9').contains(&bytes[3]),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Csv2Layout {
    pub header: SaveHeader,
    /// Snapshot, auxiliary image data, script memory image; raw format interpretation is separate.
    pub sections: [Range<usize>; 3],
    pub decoded_sizes: [usize; 3],
    pub checksum_offset: usize,
}

impl Csv2Layout {
    /// Checks fixed-size metadata and total lengths before any section allocation.
    /// Both the file and aggregate decoded sections are limited to 64 MiB by policy.
    pub fn parse(bytes: &[u8], file_size: u64) -> Result<Self, FormatError> {
        Self::parse_family(bytes, file_size, SaveFamily::Cmvs)
    }

    pub fn parse_family(
        bytes: &[u8],
        file_size: u64,
        family: SaveFamily,
    ) -> Result<Self, FormatError> {
        if bytes.len() < HEADER_SIZE {
            return Err(FormatError("truncated save container header"));
        }
        if !header_matches(bytes, family) || dword(bytes, 4) != HEADER_SIZE as u32 {
            return Err(FormatError("unsupported save container signature"));
        }
        if word(bytes, 546) != 1 {
            return Err(FormatError("unsupported snapshot encoding mode"));
        }
        if file_size > MAX_PAYLOAD as u64 {
            return Err(FormatError("save file exceeds research allocation limit"));
        }
        let end = u64::from(dword(bytes, 540));
        if end + 2 != file_size {
            return Err(FormatError("save file length differs from checksum offset"));
        }
        let mut cursor = HEADER_SIZE as u64;
        let mut ranges = [0..0, 0..0, 0..0];
        let mut decoded_sizes = [0; 3];
        let mut total_decoded = 0u64;
        for i in 0..3 {
            let packed = u64::from(dword(bytes, 560 + i * 4));
            let decoded = u64::from(dword(bytes, 580 + i * 4));
            if (packed == 0) != (decoded == 0) {
                return Err(FormatError("save section has inconsistent empty lengths"));
            }
            let next = cursor + packed;
            total_decoded += decoded;
            if next > end || total_decoded > MAX_PAYLOAD as u64 {
                return Err(FormatError("save section lengths exceed bounds"));
            }
            ranges[i] = cursor as usize..next as usize;
            decoded_sizes[i] = decoded as usize;
            cursor = next;
        }
        if cursor != end {
            return Err(FormatError("save sections do not fill declared payload"));
        }
        Ok(Self {
            header: SaveHeader::parse_family(&bytes[..HEADER_SIZE], family)?,
            sections: ranges,
            decoded_sizes,
            checksum_offset: end as usize,
        })
    }
}

#[derive(Debug)]
pub struct Csv2Save {
    pub layout: Csv2Layout,
    pub snapshot: Vec<u8>,
    pub auxiliary: Vec<u8>,
    /// Already-decoded runtime PS2A/PS3 image, not an encrypted on-disk script input.
    pub script_image: Vec<u8>,
}

impl Csv2Save {
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::decode_family(bytes, SaveFamily::Cmvs)
    }

    pub fn decode_any(bytes: &[u8]) -> Result<(SaveFamily, Self), FormatError> {
        let family = if bytes.starts_with(b"CSP") {
            SaveFamily::Cvns
        } else {
            SaveFamily::Cmvs
        };
        Ok((family, Self::decode_family(bytes, family)?))
    }

    pub fn decode_family(bytes: &[u8], family: SaveFamily) -> Result<Self, FormatError> {
        let layout = Csv2Layout::parse_family(bytes, bytes.len() as u64, family)?;
        let key = key_for(family);
        let first = &bytes[layout.sections[0].clone()];
        let checksum = first
            .iter()
            .fold(0u16, |sum, b| sum.wrapping_add(u16::from(*b)));
        if checksum != word(bytes, layout.checksum_offset) {
            return Err(FormatError("save snapshot checksum mismatch"));
        }
        let decoded_first: Vec<u8> = first
            .iter()
            .enumerate()
            .map(|(i, b)| (b ^ key[i % key.len()]).wrapping_add(128))
            .collect();
        let snapshot = lzss::decode(&decoded_first, layout.decoded_sizes[0])?;
        let auxiliary = lzss::decode(&bytes[layout.sections[1].clone()], layout.decoded_sizes[1])?;
        let script_image =
            lzss::decode(&bytes[layout.sections[2].clone()], layout.decoded_sizes[2])?;
        Ok(Self {
            layout,
            snapshot,
            auxiliary,
            script_image,
        })
    }

    /// Parses the 600-byte header and decompresses only section 1 (BMP thumbnail) when enabled.
    pub fn decode_auxiliary(
        bytes: &[u8],
        family: SaveFamily,
    ) -> Result<(SaveHeader, Vec<u8>), FormatError> {
        if bytes.len() < HEADER_SIZE {
            return Err(FormatError("truncated save container header"));
        }
        if !header_matches(bytes, family) || dword(bytes, 4) != HEADER_SIZE as u32 {
            return Err(FormatError("unsupported save container signature"));
        }
        let header = SaveHeader::parse_family(&bytes[..HEADER_SIZE], family)?;
        let thumb_enabled = word(bytes, 548) != 0;
        let sec0_packed = usize::try_from(dword(bytes, 560))
            .map_err(|_| FormatError("save section length overflow"))?;
        let sec1_packed = usize::try_from(dword(bytes, 564))
            .map_err(|_| FormatError("save section length overflow"))?;
        let sec1_decoded = usize::try_from(dword(bytes, 584))
            .map_err(|_| FormatError("save section length overflow"))?;
        if !thumb_enabled || sec1_packed == 0 || sec1_decoded == 0 {
            return Ok((header, Vec::new()));
        }
        if sec1_decoded > MAX_PAYLOAD {
            return Err(FormatError(
                "save thumbnail exceeds research allocation limit",
            ));
        }
        let start = HEADER_SIZE
            .checked_add(sec0_packed)
            .ok_or(FormatError("save section offset overflow"))?;
        let end = start
            .checked_add(sec1_packed)
            .filter(|&end| end <= bytes.len())
            .ok_or(FormatError("truncated save thumbnail section"))?;
        let auxiliary = lzss::decode(&bytes[start..end], sec1_decoded)?;
        Ok((header, auxiliary))
    }

    /// Writer 0x140080b20 (CMVS) and 0x4506a0 (CVNS): `header` is the prepared 600-byte
    /// block carrying the label and metadata. Fills packed/decoded lengths (560..592),
    /// checksum offset (540), and encoding mode 1 (546), compresses all three sections with
    /// `lzss::encode`, transforms section 0 with `(byte + 0x80) ^ key`, and appends the sum.
    pub fn encode(
        header: &[u8; HEADER_SIZE],
        snapshot: &[u8],
        auxiliary: &[u8],
        script_image: &[u8],
    ) -> Result<Vec<u8>, FormatError> {
        Self::encode_for(SaveFamily::Cmvs, header, snapshot, auxiliary, script_image)
    }

    pub fn encode_for(
        family: SaveFamily,
        header: &[u8; HEADER_SIZE],
        snapshot: &[u8],
        auxiliary: &[u8],
        script_image: &[u8],
    ) -> Result<Vec<u8>, FormatError> {
        if snapshot.is_empty() || script_image.is_empty() {
            return Err(FormatError(
                "save snapshot and script image must not be empty",
            ));
        }
        if !header_matches(header, family) || dword(header, 4) != HEADER_SIZE as u32 {
            return Err(FormatError("unexpected save container header"));
        }
        let sections = [snapshot, auxiliary, script_image];
        let total: usize = sections.iter().map(|s| s.len()).sum();
        if total > MAX_PAYLOAD {
            return Err(FormatError(
                "save sections exceed research allocation limit",
            ));
        }
        let packed = sections.map(lzss::encode);
        let end = HEADER_SIZE + packed.iter().map(Vec::len).sum::<usize>();
        let end32 = u32::try_from(end).map_err(|_| FormatError("save file too large"))?;
        let mut out = header.to_vec();
        out[540..544].copy_from_slice(&end32.to_le_bytes());
        out[546..548].copy_from_slice(&1u16.to_le_bytes());
        for (i, (section, data)) in sections.iter().zip(&packed).enumerate() {
            out[560 + 4 * i..564 + 4 * i].copy_from_slice(&(data.len() as u32).to_le_bytes());
            out[580 + 4 * i..584 + 4 * i].copy_from_slice(&(section.len() as u32).to_le_bytes());
        }
        let key = key_for(family);
        let mut checksum = 0u16;
        out.extend(packed[0].iter().enumerate().map(|(i, b)| {
            let stored = b.wrapping_add(0x80) ^ key[i % key.len()];
            checksum = checksum.wrapping_add(u16::from(stored));
            stored
        }));
        out.extend(&packed[1]);
        out.extend(&packed[2]);
        out.extend(checksum.to_le_bytes());
        Ok(out)
    }
}

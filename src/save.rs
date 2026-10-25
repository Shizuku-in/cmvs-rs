//! CSV2 disk container only. Decoded sections are not a restored engine state.
use crate::{FormatError, MAX_PAYLOAD, lzss, save_menu::SaveHeader};
use std::ops::Range;

pub const HEADER_SIZE: usize = 600;
const KEY: &[u8] = b"DAME_SAVE_IMAGE";

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().expect("validated header"))
}
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated header"))
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
        if bytes.len() < HEADER_SIZE {
            return Err(FormatError("truncated CSV2 header"));
        }
        if &bytes[..4] != b"CSV2" || dword(bytes, 4) != HEADER_SIZE as u32 {
            return Err(FormatError("expected CSV2 with 600-byte header"));
        }
        if word(bytes, 546) != 1 {
            return Err(FormatError("unsupported CSV2 snapshot encoding mode"));
        }
        if file_size > MAX_PAYLOAD as u64 {
            return Err(FormatError("CSV2 file exceeds research allocation limit"));
        }
        let end = u64::from(dword(bytes, 540));
        if end + 2 != file_size {
            return Err(FormatError("CSV2 file length differs from checksum offset"));
        }
        let mut cursor = HEADER_SIZE as u64;
        let mut ranges = [0..0, 0..0, 0..0];
        let mut decoded_sizes = [0; 3];
        let mut total_decoded = 0u64;
        for i in 0..3 {
            let packed = u64::from(dword(bytes, 560 + i * 4));
            let decoded = u64::from(dword(bytes, 580 + i * 4));
            if (packed == 0) != (decoded == 0) {
                return Err(FormatError("CSV2 section has inconsistent empty lengths"));
            }
            let next = cursor + packed;
            total_decoded += decoded;
            if next > end || total_decoded > MAX_PAYLOAD as u64 {
                return Err(FormatError("CSV2 section lengths exceed bounds"));
            }
            ranges[i] = cursor as usize..next as usize;
            decoded_sizes[i] = decoded as usize;
            cursor = next;
        }
        if cursor != end {
            return Err(FormatError("CSV2 sections do not fill declared payload"));
        }
        Ok(Self {
            header: SaveHeader::parse(&bytes[..HEADER_SIZE])?,
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
    /// Already-decoded runtime PS2A image, not an encrypted on-disk PS2A input.
    pub script_image: Vec<u8>,
}

impl Csv2Save {
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let layout = Csv2Layout::parse(bytes, bytes.len() as u64)?;
        let first = &bytes[layout.sections[0].clone()];
        let checksum = first
            .iter()
            .fold(0u16, |sum, b| sum.wrapping_add(u16::from(*b)));
        if checksum != word(bytes, layout.checksum_offset) {
            return Err(FormatError("CSV2 snapshot checksum mismatch"));
        }
        let decoded_first: Vec<u8> = first
            .iter()
            .enumerate()
            .map(|(i, b)| (b ^ KEY[i % KEY.len()]).wrapping_add(128))
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
}

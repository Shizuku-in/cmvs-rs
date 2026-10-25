//! CPZ6 metadata only. Directory/file indexes and checksums are not decoded yet.
//! Header XOR fields follow xmoezzz/CMVS-Engine, pinned in docs/references.md.
use crate::FormatError;

pub const CPZ6_HEADER_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpz6Layout {
    pub directory_count: u32,
    pub directory_index_bytes: u32,
    pub file_index_bytes: u32,
    pub encryption_mode: u32,
    pub data_offset: u64,
    pub data_bytes: u64,
}

impl Cpz6Layout {
    /// `file_bytes` is the actual archive length, not the amount read into memory.
    /// Does not authenticate the header or validate the encrypted index contents.
    pub fn parse(header: &[u8], file_bytes: u64) -> Result<Self, FormatError> {
        if header.len() < CPZ6_HEADER_SIZE || &header[..4] != b"CPZ6" {
            return Err(FormatError("expected 64-byte CPZ6 header"));
        }
        let field = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let directory_count = field(4) ^ 0xfe3a53da;
        let directory_index_bytes = field(8) ^ 0x37f298e8;
        let file_index_bytes = field(12) ^ 0x7a6f3a2d;
        let encryption_mode = field(52) ^ 0xfb73a956;
        let data_offset = CPZ6_HEADER_SIZE as u64
            + u64::from(directory_index_bytes)
            + u64::from(file_index_bytes);
        if data_offset > file_bytes {
            return Err(FormatError("CPZ6 indexes exceed archive length"));
        }
        // Each directory record has 16 fixed bytes and at least a NUL filename.
        if u64::from(directory_count) * 17 > u64::from(directory_index_bytes) {
            return Err(FormatError(
                "CPZ6 directory records cannot fit in declared index",
            ));
        }
        Ok(Self {
            directory_count,
            directory_index_bytes,
            file_index_bytes,
            encryption_mode,
            data_offset,
            data_bytes: file_bytes - data_offset,
        })
    }
}

//! CMV6 movie containers: header/index parsing and bounded JBPD frame reads.
//! The Ogg soundtrack is located but not decoded. Field meanings follow the movie
//! player in the documented binary (0x1400379f0, 0x140037bb0, 0x140037900) and were checked
//! read-only against this title's data/video files.
use crate::FormatError;
use std::io::{Read, Seek, SeekFrom};

pub const HEADER_SIZE: u64 = 44;
const ENTRY_SIZE: u64 = 20;
/// Research limit on index entries (op.cmv has 2908).
const ENTRY_LIMIT: u32 = 1 << 20;

/// Resource kinds in the index.
pub const KIND_OGG: u32 = 0;
pub const KIND_JBPX: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CmvHeader {
    /// Absolute offset of the resource section; entry offsets are relative to it.
    pub resource_offset: u32,
    pub file_size: u32,
    pub unused: u32,
    /// The player decodes frames whose index is below this value (0x140037900).
    pub max_sequence: u32,
    /// Frame rate as `rate / rate_time` frames per second.
    pub rate: u32,
    pub rate_time: u32,
    pub width: u32,
    pub height: u32,
    pub bit_count: u32,
    pub audio_flag: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CmvEntry {
    pub sequence: u32,
    pub stored_size: u32,
    pub decoded_size: u32,
    pub kind: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmvIndex {
    pub header: CmvHeader,
    pub entries: Vec<CmvEntry>,
}

impl CmvIndex {
    /// Reads and bounds-checks the header and `max_sequence + 1` index entries.
    pub fn read(reader: &mut (impl Read + Seek)) -> Result<Self, FormatError> {
        let length = reader
            .seek(SeekFrom::End(0))
            .map_err(|_| FormatError("CMV length unavailable"))?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| FormatError("CMV seek failed"))?;
        let mut head = [0u8; HEADER_SIZE as usize];
        reader
            .read_exact(&mut head)
            .map_err(|_| FormatError("CMV header truncated"))?;
        if &head[..4] != b"CMV6" {
            return Err(FormatError("only the verified CMV6 signature is supported"));
        }
        let word = |i: usize| u32::from_le_bytes(head[4 + 4 * i..8 + 4 * i].try_into().unwrap());
        let header = CmvHeader {
            resource_offset: word(0),
            file_size: word(1),
            unused: word(2),
            max_sequence: word(3),
            rate: word(4),
            rate_time: word(5),
            width: word(6),
            height: word(7),
            bit_count: word(8),
            audio_flag: word(9),
        };
        if u64::from(header.file_size) != length {
            return Err(FormatError("CMV size field differs from the file length"));
        }
        if header.rate == 0 || header.rate_time == 0 {
            return Err(FormatError("CMV frame rate is zero"));
        }
        let count = header
            .max_sequence
            .checked_add(1)
            .filter(|count| *count <= ENTRY_LIMIT)
            .ok_or(FormatError("CMV entry count outside research limit"))?;
        if u64::from(header.resource_offset) != HEADER_SIZE + u64::from(count) * ENTRY_SIZE {
            return Err(FormatError("CMV resources do not follow the index"));
        }
        let mut table = vec![0u8; (u64::from(count) * ENTRY_SIZE) as usize];
        reader
            .read_exact(&mut table)
            .map_err(|_| FormatError("CMV index truncated"))?;
        let mut entries = Vec::with_capacity(count as usize);
        for raw in table.chunks_exact(ENTRY_SIZE as usize) {
            let field = |i: usize| u32::from_le_bytes(raw[4 * i..4 * i + 4].try_into().unwrap());
            let entry = CmvEntry {
                sequence: field(0),
                stored_size: field(1),
                decoded_size: field(2),
                kind: field(3),
                offset: field(4),
            };
            if entry.file_offset(&header) + u64::from(entry.stored_size) > length {
                return Err(FormatError("CMV entry outside the file"));
            }
            entries.push(entry);
        }
        Ok(Self { header, entries })
    }
    /// The Ogg soundtrack the player hands to device channel 2.
    pub fn audio(&self) -> Option<&CmvEntry> {
        self.entries.iter().find(|entry| entry.kind == KIND_OGG)
    }
    pub fn frames(&self) -> impl Iterator<Item = &CmvEntry> {
        self.entries.iter().filter(|entry| entry.kind == KIND_JBPX)
    }
    /// Read one bounded encoded frame and validate its dimensions against the container.
    pub fn decode_frame(
        &self,
        reader: &mut (impl Read + Seek),
        frame: u32,
    ) -> Result<crate::raster::RgbaImage, Box<dyn std::error::Error>> {
        let entry = self
            .entries
            .get(frame as usize)
            .filter(|e| {
                e.kind == KIND_JBPX && e.sequence == frame && frame < self.header.max_sequence
            })
            .ok_or("CMV frame index is not a video entry")?;
        let decoded_size = u64::from(self.header.width)
            .checked_mul(u64::from(self.header.height))
            .and_then(|n| n.checked_mul(3));
        if self.header.bit_count != 24 || Some(u64::from(entry.decoded_size)) != decoded_size {
            return Err("CMV decoded frame size or bit depth unsupported".into());
        }
        if entry.stored_size as usize > crate::MAX_PAYLOAD {
            return Err("CMV encoded frame exceeds research limit".into());
        }
        reader.seek(SeekFrom::Start(entry.file_offset(&self.header)))?;
        let mut data = vec![0; entry.stored_size as usize];
        reader.read_exact(&mut data)?;
        let image = crate::jbpd::decode(&data)?;
        if image.width() != self.header.width || image.height() != self.header.height {
            return Err("JBPD dimensions differ from CMV header".into());
        }
        Ok(image)
    }
}

impl CmvEntry {
    pub fn file_offset(&self, header: &CmvHeader) -> u64 {
        u64::from(header.resource_offset) + u64::from(self.offset)
    }
}

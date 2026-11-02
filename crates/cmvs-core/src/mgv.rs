//! Bounded MGV1 container and nominal-frame to Theora-packet mapping.
//! REALIVE mog3x64.dll SHA-256 053156399e46565fcac4d5c4ce2ebede8bdb8abf1caaec46a29a5f76d2192a83:
//! MGV_InputFile RVA 0x22cf0; engine frame mapping 0x14001c180.
use crate::{FormatError, MAX_PAYLOAD, raster::RgbaImage};
use std::{
    error::Error,
    fmt::Debug,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameEntry {
    pub packet: u16,
    pub key_frame: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MgvIndex {
    pub width: u16,
    pub height: u16,
    pub rate: u32,
    pub rate_time: u32,
    pub frames: Vec<FrameEntry>,
    pub audio_offset: u64,
    pub audio_size: u32,
    pub video_offset: u64,
    pub video_size: u32,
}

impl MgvIndex {
    pub fn read(reader: &mut (impl Read + Seek + ?Sized)) -> Result<Self, FormatError> {
        let length = reader
            .seek(SeekFrom::End(0))
            .map_err(|_| FormatError("MGV length unavailable"))?;
        reader
            .rewind()
            .map_err(|_| FormatError("MGV seek failed"))?;
        let mut bytes = [0; 32];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| FormatError("MGV header truncated"))?;
        let word = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let count = word(16);
        if &bytes[..4] != b"MGV1" || word(4) != 32 {
            return Err(FormatError("expected MGV1 header version 32"));
        }
        let width = u16::from_le_bytes(bytes[20..22].try_into().unwrap());
        let height = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) * 4 > MAX_PAYLOAD as u64
        {
            return Err(FormatError("MGV dimensions outside research limit"));
        }
        if count == 0 || count > 1 << 20 || word(24) == 0 || word(28) == 0 {
            return Err(FormatError(
                "MGV frame count or rate outside supported range",
            ));
        }
        let audio_offset = 32 + u64::from(count) * 4;
        let video_offset = audio_offset + u64::from(word(12));
        if word(12) as usize > MAX_PAYLOAD
            || word(8) == 0
            || video_offset + u64::from(word(8)) != length
        {
            return Err(FormatError("MGV stream ranges disagree with file length"));
        }
        let mut frames = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut entry = [0; 4];
            reader
                .read_exact(&mut entry)
                .map_err(|_| FormatError("MGV frame table truncated"))?;
            let packet = u16::from_le_bytes(entry[2..4].try_into().unwrap());
            let flags = u16::from_le_bytes(entry[..2].try_into().unwrap());
            if flags > 1
                || frames
                    .last()
                    .map_or(packet != 0 || flags != 1, |last: &FrameEntry| {
                        packet < last.packet
                            || u32::from(packet) > u32::from(last.packet) + 1
                            || packet == last.packet && (flags != u16::from(last.key_frame))
                    })
            {
                return Err(FormatError(
                    "MGV frame map has invalid flags or packet order",
                ));
            }
            frames.push(FrameEntry {
                packet,
                key_frame: flags != 0,
            });
        }
        Ok(Self {
            width,
            height,
            rate: word(28),
            rate_time: word(24),
            frames,
            audio_offset,
            audio_size: word(12),
            video_offset,
            video_size: word(8),
        })
    }

    pub fn packet_count(&self) -> u32 {
        self.frames
            .last()
            .map_or(0, |entry| u32::from(entry.packet) + 1)
    }
}

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// A stateful decoder may replay packets for backwards access; requests use nominal frames.
pub trait Decoder: Debug {
    fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error>>;
    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

/// Codec dependencies live in a backend; container parsing remains in the core.
pub trait DecoderFactory: Debug {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn Decoder>, Box<dyn Error>>;
}

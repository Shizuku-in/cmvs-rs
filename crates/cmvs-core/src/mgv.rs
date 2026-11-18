//! MGV1 movie indexing and frame-to-Theora-packet mapping.
//!
//! The container reader validates dimensions and packet bounds. A host installs a
//! [`DecoderFactory`] for Theora decoding; [`crate::movie`] controls playback.

// REALIVE mog3x64.dll SHA-256 053156399e46565fcac4d5c4ce2ebede8bdb8abf1caaec46a29a5f76d2192a83:
// MGV_InputFile RVA 0x22cf0; engine frame mapping 0x14001c180.
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
        Self::read_io(reader).map_err(|err| match err.downcast::<FormatError>() {
            Ok(format_err) => *format_err,
            Err(_) => FormatError("MGV I/O failure"),
        })
    }

    /// Like [`Self::read`], preserving underlying [`std::io::Error`] failures instead of
    /// collapsing them into [`FormatError`].
    pub fn read_io(
        reader: &mut (impl Read + Seek + ?Sized),
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let length = reader.seek(SeekFrom::End(0))?;
        reader.rewind()?;
        let mut bytes = [0; 32];
        reader.read_exact(&mut bytes).map_err(|err| {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Box::new(FormatError("MGV header truncated"))
                    as Box<dyn std::error::Error + Send + Sync>
            } else {
                Box::new(err)
            }
        })?;
        let word = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let count = word(16);
        if &bytes[..4] != b"MGV1" || word(4) != 32 {
            return Err(FormatError("expected MGV1 header version 32").into());
        }
        let width = u16::from_le_bytes(bytes[20..22].try_into().unwrap());
        let height = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) * 4 > MAX_PAYLOAD as u64
        {
            return Err(FormatError("MGV dimensions outside research limit").into());
        }
        if count == 0 || count > 1 << 20 || word(24) == 0 || word(28) == 0 {
            return Err(FormatError("MGV frame count or rate outside supported range").into());
        }
        let audio_offset = 32 + u64::from(count) * 4;
        let video_offset = audio_offset + u64::from(word(12));
        if word(12) as usize > MAX_PAYLOAD
            || word(8) == 0
            || video_offset + u64::from(word(8)) != length
        {
            return Err(FormatError("MGV stream ranges disagree with file length").into());
        }
        let mut table = vec![0u8; count as usize * 4];
        reader.read_exact(&mut table).map_err(|err| {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Box::new(FormatError("MGV frame table truncated"))
                    as Box<dyn std::error::Error + Send + Sync>
            } else {
                Box::new(err)
            }
        })?;
        let mut frames = Vec::with_capacity(count as usize);
        for entry in table.as_chunks::<4>().0 {
            let packet = u16::from_le_bytes([entry[2], entry[3]]);
            let flags = u16::from_le_bytes([entry[0], entry[1]]);
            if flags > 1
                || frames
                    .last()
                    .map_or(packet != 0 || flags != 1, |last: &FrameEntry| {
                        packet < last.packet
                            || u32::from(packet) > u32::from(last.packet) + 1
                            || packet == last.packet && (flags != u16::from(last.key_frame))
                    })
            {
                return Err(FormatError("MGV frame map has invalid flags or packet order").into());
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

/// `Send` so a backend may decode on its own thread.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// A stateful decoder may replay packets for backwards access; requests use nominal frames.
pub trait Decoder: Debug + Send {
    fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error + Send + Sync>>;
    fn finish(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
}

/// Codec dependencies live in a backend; container parsing remains in the core.
pub trait DecoderFactory: Debug + Send {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn Decoder>, Box<dyn Error + Send + Sync>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    fn synthetic_mgv() -> Vec<u8> {
        let mut b = b"MGV1".to_vec();
        for word in [32u32, 4, 3, 4] {
            b.extend(word.to_le_bytes());
        }
        b.extend(32u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(1000u32.to_le_bytes());
        b.extend(24000u32.to_le_bytes());
        for word in [1u32, 1 << 16, 1 << 16, (2 << 16) | 1] {
            b.extend(word.to_le_bytes());
        }
        b.extend(b"oggdata");
        b
    }

    struct FaultyReader {
        data: Vec<u8>,
        reported_len: u64,
        pos: usize,
        fail_seek: Option<io::ErrorKind>,
        fail_read_at: Option<(usize, Option<io::ErrorKind>)>,
    }

    impl FaultyReader {
        fn new(data: Vec<u8>) -> Self {
            let reported_len = data.len() as u64;
            Self {
                data,
                reported_len,
                pos: 0,
                fail_seek: None,
                fail_read_at: None,
            }
        }
    }

    impl Read for FaultyReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            if let Some((at, action)) = self.fail_read_at {
                if self.pos >= at {
                    return match action {
                        Some(kind) => Err(io::Error::new(kind, "injected MGV read error")),
                        None => Ok(0),
                    };
                }
                let allowed = (at - self.pos).min(buf.len());
                let src = &self.data[self.pos..self.data.len().min(self.pos + allowed)];
                buf[..src.len()].copy_from_slice(src);
                self.pos += src.len();
                return Ok(src.len());
            }
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl Seek for FaultyReader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            if let Some(kind) = self.fail_seek {
                return Err(io::Error::new(kind, "injected MGV seek error"));
            }
            let next = match pos {
                SeekFrom::Start(offset) => offset as i64,
                SeekFrom::End(delta) => self.reported_len as i64 + delta,
                SeekFrom::Current(delta) => self.pos as i64 + delta,
            };
            if next < 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "negative seek"));
            }
            self.pos = next as usize;
            Ok(next as u64)
        }
    }

    #[test]
    fn read_io_preserves_non_eof_io_errors_and_maps_truncation_to_format_error() {
        let valid = synthetic_mgv();
        assert_eq!(
            MgvIndex::read_io(&mut Cursor::new(&valid)).unwrap(),
            MgvIndex::read(&mut Cursor::new(&valid)).unwrap()
        );

        // Seek failure preserves io::Error.
        let mut seek_err = FaultyReader::new(valid.clone());
        seek_err.fail_seek = Some(io::ErrorKind::PermissionDenied);
        let err = MgvIndex::read_io(&mut seek_err).unwrap_err();
        assert!(err.downcast_ref::<FormatError>().is_none());
        assert_eq!(
            err.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );

        // Header and frame-table non-EOF read failures preserve io::Error.
        for (at, kind) in [
            (12, io::ErrorKind::PermissionDenied),
            (36, io::ErrorKind::Other),
        ] {
            let mut reader = FaultyReader::new(valid.clone());
            reader.fail_read_at = Some((at, Some(kind)));
            let err = MgvIndex::read_io(&mut reader).unwrap_err();
            assert!(err.downcast_ref::<FormatError>().is_none());
            assert_eq!(err.downcast_ref::<io::Error>().unwrap().kind(), kind);
        }

        // Truncated header buffer (`UnexpectedEof`) still produces FormatError.
        let short_err = MgvIndex::read_io(&mut Cursor::new(&valid[..16])).unwrap_err();
        assert_eq!(
            short_err.downcast_ref::<FormatError>(),
            Some(&FormatError("MGV header truncated"))
        );

        // Truncated frame-table buffer (`UnexpectedEof` after valid header) still produces FormatError.
        let mut truncated_table = FaultyReader::new(valid);
        truncated_table.fail_read_at = Some((36, None));
        let table_err = MgvIndex::read_io(&mut truncated_table).unwrap_err();
        assert_eq!(
            table_err.downcast_ref::<FormatError>(),
            Some(&FormatError("MGV frame table truncated"))
        );
    }
}

//! CMV3/CMV4/CMV6/CMV8 movie headers, frame indices, and bounded frame decoding.
//!
//! [`CmvIndex`] reads container metadata and locates image frames and the Ogg
//! soundtrack. Audio decoding and playback are separate host services.

// Field meanings follow the hash-bound movie player at 0x1400379f0,
// 0x140037bb0, and 0x140037900, checked read-only against the game's video files.
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
    /// CMV8 (0x1400390b0 type 4) frames are 32-bit JBPE images with alpha; CMV6 frames are
    /// opaque 24-bit JBPD images.
    pub alpha: bool,
    /// CMV3 (cvns-20090203 op.cmv) and CMV4 (cmvs-20070119 op.cmv) keep the 44-byte CMV6
    /// container with opaque JBP1 frames; those players check no container signature.
    pub jbp1: bool,
}

impl CmvIndex {
    /// Reads and bounds-checks a CMV3/CMV4/CMV6 header and its `max_sequence + 1` index entries.
    pub fn read(reader: &mut (impl Read + Seek + ?Sized)) -> Result<Self, FormatError> {
        Self::read_signature(reader, false)
    }
    /// Scene movies also accept CMV8, whose container fields match CMV6.
    pub fn read_scene(reader: &mut (impl Read + Seek + ?Sized)) -> Result<Self, FormatError> {
        Self::read_signature(reader, true)
    }
    /// Like [`Self::read`], preserving underlying [`std::io::Error`] failures instead of
    /// collapsing them into [`FormatError`].
    pub fn read_io(
        reader: &mut (impl Read + Seek + ?Sized),
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::read_signature_io(reader, false)
    }
    /// Like [`Self::read_scene`], preserving underlying [`std::io::Error`] failures.
    pub fn read_scene_io(
        reader: &mut (impl Read + Seek + ?Sized),
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::read_signature_io(reader, true)
    }
    fn read_signature(
        reader: &mut (impl Read + Seek + ?Sized),
        allow_alpha: bool,
    ) -> Result<Self, FormatError> {
        Self::read_signature_io(reader, allow_alpha).map_err(|err| {
            match err.downcast::<FormatError>() {
                Ok(format_err) => *format_err,
                Err(_) => FormatError("CMV I/O failure"),
            }
        })
    }
    fn read_signature_io(
        reader: &mut (impl Read + Seek + ?Sized),
        allow_alpha: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let length = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut head = [0u8; HEADER_SIZE as usize];
        reader.read_exact(&mut head).map_err(|err| {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Box::new(FormatError("CMV header truncated"))
                    as Box<dyn std::error::Error + Send + Sync>
            } else {
                Box::new(err)
            }
        })?;
        let (alpha, jbp1) = match &head[..4] {
            b"CMV6" => (false, false),
            b"CMV3" | b"CMV4" => (false, true),
            b"CMV8" if allow_alpha => (true, false),
            _ => {
                return Err(FormatError(
                    "only the verified CMV3/CMV4/CMV6 signatures are supported",
                )
                .into());
            }
        };
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
            return Err(FormatError("CMV size field differs from the file length").into());
        }
        if header.rate == 0 || header.rate_time == 0 {
            return Err(FormatError("CMV frame rate is zero").into());
        }
        let count = header
            .max_sequence
            .checked_add(1)
            .filter(|count| *count <= ENTRY_LIMIT)
            .ok_or(FormatError("CMV entry count outside research limit"))?;
        if u64::from(header.resource_offset) != HEADER_SIZE + u64::from(count) * ENTRY_SIZE {
            return Err(FormatError("CMV resources do not follow the index").into());
        }
        let mut table = vec![0u8; (u64::from(count) * ENTRY_SIZE) as usize];
        reader.read_exact(&mut table).map_err(|err| {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Box::new(FormatError("CMV index truncated"))
                    as Box<dyn std::error::Error + Send + Sync>
            } else {
                Box::new(err)
            }
        })?;
        let mut entries = Vec::with_capacity(count as usize);
        for raw in table.as_chunks::<{ ENTRY_SIZE as usize }>().0 {
            let field = |i: usize| u32::from_le_bytes(raw[4 * i..4 * i + 4].try_into().unwrap());
            let entry = CmvEntry {
                sequence: field(0),
                stored_size: field(1),
                decoded_size: field(2),
                kind: field(3),
                offset: field(4),
            };
            if entry.file_offset(&header) + u64::from(entry.stored_size) > length {
                return Err(FormatError("CMV entry outside the file").into());
            }
            entries.push(entry);
        }
        Ok(Self {
            header,
            entries,
            alpha,
            jbp1,
        })
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
        reader: &mut (impl Read + Seek + ?Sized),
        frame: u32,
    ) -> Result<crate::raster::RgbaImage, Box<dyn std::error::Error + Send + Sync>> {
        let entry = self
            .entries
            .get(frame as usize)
            .filter(|e| {
                e.kind == KIND_JBPX && e.sequence == frame && frame < self.header.max_sequence
            })
            .ok_or("CMV frame index is not a video entry")?;
        let channels = if self.alpha { 4 } else { 3 };
        let decoded_size = u64::from(self.header.width)
            .checked_mul(u64::from(self.header.height))
            .and_then(|n| n.checked_mul(channels));
        // Scene CMV6/CMV8 indexes also record the 54-byte BMP header in this metadata
        // size (mv_ic01.cmv, st-snow.cmv). JBPD output itself is still just the surface.
        if u64::from(self.header.bit_count) != channels * 8
            || !decoded_size.is_some_and(|size| {
                u64::from(entry.decoded_size) == size || u64::from(entry.decoded_size) == size + 54
            })
        {
            return Err("CMV decoded frame size or bit depth unsupported".into());
        }
        if entry.stored_size as usize > crate::MAX_PAYLOAD {
            return Err("CMV encoded frame exceeds research limit".into());
        }
        reader.seek(SeekFrom::Start(entry.file_offset(&self.header)))?;
        let mut data = vec![0; entry.stored_size as usize];
        reader.read_exact(&mut data)?;
        let image = if self.jbp1 {
            let frame = crate::jbp1::decode(&data)?;
            let (width, height) = (u32::from(frame.width), u32::from(frame.height));
            let pixels = frame.rgba;
            crate::raster::RgbaImage::new(width, height, pixels)?
        } else if self.alpha {
            crate::jbpd::decode_alpha(&data)?
        } else {
            crate::jbpd::decode(&data)?
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    fn synthetic_cmv() -> Vec<u8> {
        let frames = 2u32;
        let count = frames + 1;
        let resource = 44 + count * 20;
        let data = frames * 4 + 3;
        let mut bytes = b"CMV6".to_vec();
        for word in [
            resource,
            resource + data,
            0,
            frames,
            24,
            1,
            1280,
            720,
            24,
            2,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        for i in 0..frames {
            for word in [i, 4, 1280 * 720 * 3, KIND_JBPX, i * 4] {
                bytes.extend(word.to_le_bytes());
            }
        }
        for word in [frames, 3, 3, KIND_OGG, frames * 4] {
            bytes.extend(word.to_le_bytes());
        }
        for i in 0..frames {
            bytes.extend(b"JBP");
            bytes.push(i as u8);
        }
        bytes.extend(b"Ogg");
        bytes
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
                        Some(kind) => Err(io::Error::new(kind, "injected CMV read error")),
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
                return Err(io::Error::new(kind, "injected CMV seek error"));
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
        let valid = synthetic_cmv();
        assert_eq!(
            CmvIndex::read_io(&mut Cursor::new(&valid)).unwrap(),
            CmvIndex::read(&mut Cursor::new(&valid)).unwrap()
        );

        // Seek failure preserves io::Error.
        let mut seek_err = FaultyReader::new(valid.clone());
        seek_err.fail_seek = Some(io::ErrorKind::PermissionDenied);
        let err = CmvIndex::read_io(&mut seek_err).unwrap_err();
        assert!(err.downcast_ref::<FormatError>().is_none());
        assert_eq!(
            err.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );

        // Header and index non-EOF read failures preserve io::Error on both read_io and read_scene_io.
        for (at, kind) in [
            (12, io::ErrorKind::PermissionDenied),
            (52, io::ErrorKind::Other),
        ] {
            let mut reader = FaultyReader::new(valid.clone());
            reader.fail_read_at = Some((at, Some(kind)));
            let err = CmvIndex::read_io(&mut reader).unwrap_err();
            assert!(err.downcast_ref::<FormatError>().is_none());
            assert_eq!(err.downcast_ref::<io::Error>().unwrap().kind(), kind);

            let mut scene_reader = FaultyReader::new(valid.clone());
            scene_reader.fail_read_at = Some((at, Some(kind)));
            let scene_err = CmvIndex::read_scene_io(&mut scene_reader).unwrap_err();
            assert!(scene_err.downcast_ref::<FormatError>().is_none());
            assert_eq!(scene_err.downcast_ref::<io::Error>().unwrap().kind(), kind);
        }

        // Truncated header buffer (`UnexpectedEof`) still produces FormatError.
        let short_err = CmvIndex::read_io(&mut Cursor::new(&valid[..20])).unwrap_err();
        assert_eq!(
            short_err.downcast_ref::<FormatError>(),
            Some(&FormatError("CMV header truncated"))
        );

        // Truncated index buffer (`UnexpectedEof` after valid header) still produces FormatError.
        let mut truncated_index = FaultyReader::new(valid);
        truncated_index.fail_read_at = Some((52, None));
        let idx_err = CmvIndex::read_io(&mut truncated_index).unwrap_err();
        assert_eq!(
            idx_err.downcast_ref::<FormatError>(),
            Some(&FormatError("CMV index truncated"))
        );
    }
}

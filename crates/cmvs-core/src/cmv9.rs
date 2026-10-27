//! CMV9 scene movies with a static MCU bitmap and independently decoded dynamic blocks.
//! Evidence: hash-bound 0x140039c90, 0x1400388c0, 0x140038640 and 0x1400b19c0.
use crate::{FormatError, MAX_PAYLOAD, cmv::CmvEntry, jbpd, raster::RgbaImage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmv9Index {
    pub width: u32,
    pub height: u32,
    pub max_sequence: u32,
    pub rate: u32,
    pub rate_time: u32,
    pub resource_offset: usize,
    pub entries: Vec<CmvEntry>,
    base_offset: usize,
    base_size: usize,
}

impl Cmv9Index {
    /// Only the verified opaque, silent CMV9 branch. Other versions/audio stay explicit.
    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() > MAX_PAYLOAD || data.get(..4) != Some(b"CMV9") {
            return Err(FormatError("expected bounded CMV9 scene movie"));
        }
        let word = |at| jbpd::word(data, at);
        if word(8)? as usize != data.len() || word(36)? != 24 || word(40)? != 0 {
            return Err(FormatError("unsupported CMV9 size, bit depth or audio"));
        }
        let max_sequence = word(16)?;
        let count = max_sequence
            .checked_add(1)
            .filter(|&n| n <= 1 << 20)
            .ok_or(FormatError("CMV9 index count outside research limit"))?
            as usize;
        let base_offset = 44 + count * 20;
        let resource_offset = word(4)? as usize;
        let base_size = word(base_offset)? as usize;
        if base_size < 8
            || base_offset.checked_add(base_size) != Some(resource_offset)
            || resource_offset > data.len()
        {
            return Err(FormatError("CMV9 static block outside header"));
        }
        let (width, height, rate, rate_time) = (word(28)?, word(32)?, word(20)?, word(24)?);
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > (MAX_PAYLOAD / 4) as u64
            || rate == 0
            || rate_time == 0
        {
            return Err(FormatError("invalid CMV9 dimensions or rate"));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let at = 44 + i * 20;
            let entry = CmvEntry {
                sequence: word(at)?,
                stored_size: word(at + 4)?,
                decoded_size: word(at + 8)?,
                kind: word(at + 12)?,
                offset: word(at + 16)?,
            };
            if resource_offset as u64 + u64::from(entry.offset) + u64::from(entry.stored_size)
                > data.len() as u64
            {
                return Err(FormatError("CMV9 entry outside file"));
            }
            entries.push(entry);
        }
        Ok(Self {
            width,
            height,
            max_sequence,
            rate,
            rate_time,
            resource_offset,
            entries,
            base_offset,
            base_size,
        })
    }

    /// Non-consecutive-frame path: rebuild the static base and decode all dynamic MCUs.
    pub fn decode_frame(&self, data: &[u8], frame: u32) -> Result<RgbaImage, FormatError> {
        let _entry = self
            .entries
            .get(frame as usize)
            .filter(|e| frame < self.max_sequence && e.sequence == frame && e.kind == 2)
            .ok_or(FormatError("CMV9 frame is not a video entry"))?;
        let base = self.decode_base(data)?;
        self.decode_with_base(data, frame, &base)
    }

    pub(crate) fn decode_base(&self, data: &[u8]) -> Result<RgbaImage, FormatError> {
        let base = data
            .get(self.base_offset..self.base_offset + self.base_size)
            .ok_or(FormatError("CMV9 static block truncated"))?;
        let grid = jbpd::word(base, 4)?;
        let (cols, rows) = ((grid & 65535) as usize, (grid >> 16) as usize);
        if cols != (self.width as usize).div_ceil(16) || rows != (self.height as usize).div_ceil(16)
        {
            return Err(FormatError("CMV9 static grid differs from dimensions"));
        }
        let mask_end = 8 + (cols * rows).div_ceil(8);
        let mask = base
            .get(8..mask_end)
            .ok_or(FormatError("CMV9 static bitmap truncated"))?;
        let image = jbpd::decode_masked(
            base.get(mask_end..)
                .ok_or(FormatError("CMV9 static image truncated"))?,
            mask,
            true,
            None,
        )?;
        if (image.width(), image.height()) != (self.width, self.height) {
            return Err(FormatError("CMV9 static image dimensions differ"));
        }
        Ok(image)
    }

    pub(crate) fn decode_with_base(
        &self,
        data: &[u8],
        frame: u32,
        image: &RgbaImage,
    ) -> Result<RgbaImage, FormatError> {
        let entry = self
            .entries
            .get(frame as usize)
            .filter(|e| frame < self.max_sequence && e.sequence == frame && e.kind == 2)
            .ok_or(FormatError("CMV9 frame is not a video entry"))?;
        let count = (self.width as usize).div_ceil(16) * (self.height as usize).div_ceil(16);
        let mask = data
            .get(self.base_offset + 8..self.base_offset + 8 + count.div_ceil(8))
            .ok_or(FormatError("CMV9 static bitmap truncated"))?;
        let start = self.resource_offset + entry.offset as usize;
        let frame_data = data
            .get(start..start + entry.stored_size as usize)
            .ok_or(FormatError("CMV9 frame truncated"))?;
        jbpd::decode_masked(frame_data, mask, false, Some(image))
    }
}

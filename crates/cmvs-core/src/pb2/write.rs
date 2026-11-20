//! Lossless CVNS static PB2A images.
use super::{HEADER_SIZE, KEY_TAIL_OFFSET};
use crate::{
    FormatError, MAX_PAYLOAD,
    pb_write::{Compression, blocked_plane, chunks, tiles},
    pb3::EncodeDepth,
    raster::RgbaImage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticType {
    Swizzled = 1,
    Blocked = 2,
}

pub fn encode(
    img: &RgbaImage,
    kind: StaticType,
    depth: EncodeDepth,
) -> Result<Vec<u8>, FormatError> {
    encode_static(img, kind, depth, Compression::Lzss)
}

pub fn encode_static(
    img: &RgbaImage,
    kind: StaticType,
    depth: EncodeDepth,
    mode: Compression,
) -> Result<Vec<u8>, FormatError> {
    let w = u16::try_from(img.width()).map_err(|_| FormatError("PB2 width exceeds 16 bits"))?;
    let h = u16::try_from(img.height()).map_err(|_| FormatError("PB2 height exceeds 16 bits"))?;
    if depth == EncodeDepth::Rgb24 && img.pixels().iter().skip(3).step_by(4).any(|&a| a != 255) {
        return Err(FormatError("24-bit PB2 requires opaque pixels"));
    }
    let lanes = &[2, 1, 0, 3][..usize::from(depth.bpp() / 8)];
    let (flags, data) = match kind {
        StaticType::Swizzled => {
            let mut unpacked = Vec::with_capacity(w as usize * h as usize * lanes.len());
            for &lane in lanes {
                tiles(img, lane, 8, |tile| unpacked.extend_from_slice(tile));
            }
            mode.split(&unpacked)
        }
        StaticType::Blocked => {
            let (flags, data) = lanes
                .iter()
                .map(|&lane| blocked_plane(img, lane, 8, mode))
                .unzip();
            (chunks(flags), chunks(data))
        }
    };
    let data_start = HEADER_SIZE + flags.len();
    // A separate zero trailer makes the header transform the identity.
    let size = data_start + data.len() + KEY_TAIL_OFFSET;
    if size > MAX_PAYLOAD {
        return Err(FormatError("encoded PB2 exceeds allocation limit"));
    }
    let mut raw = Vec::with_capacity(size);
    raw.extend_from_slice(b"PB2A");
    raw.extend((size as u32).to_le_bytes());
    raw.extend(1u32.to_le_bytes());
    raw.extend(0u32.to_le_bytes());
    for field in [kind as u16, w, h, depth.bpp()] {
        raw.extend(field.to_le_bytes());
    }
    raw.extend((HEADER_SIZE as u32).to_le_bytes());
    raw.extend((data_start as u32).to_le_bytes());
    raw.extend(flags);
    raw.extend(data);
    raw.resize(size, 0);
    Ok(raw)
}

/// Builds a type-5 animation or type-6 part collection one decoded image at a time.
pub struct CollectionWriter {
    base: RgbaImage,
    bytes: Vec<u8>,
    named: bool,
    mode: Compression,
    count: u32,
}

fn part_chunk(name: &[u8], payload: &[u8]) -> Result<Vec<u8>, FormatError> {
    if name.len() >= 128 || name.contains(&0) {
        return Err(FormatError("invalid PB2 part name"));
    }
    let start = 4 + (name.len() + 4) / 4 * 4;
    let mut out = vec![0; start];
    crate::pb_write::put(&mut out, 0, start + payload.len());
    out[4..4 + name.len()].copy_from_slice(name);
    out.extend(payload);
    Ok(out)
}

impl CollectionWriter {
    /// Some(name) selects type 6, including an empty name for its base part.
    pub fn new(
        base: RgbaImage,
        name: Option<&[u8]>,
        bpp: u16,
        mode: Compression,
    ) -> Result<Self, FormatError> {
        use crate::pb_write::{planes, put};
        let w =
            u16::try_from(base.width()).map_err(|_| FormatError("PB2 width exceeds 16 bits"))?;
        let h =
            u16::try_from(base.height()).map_err(|_| FormatError("PB2 height exceeds 16 bits"))?;
        let mut bytes = vec![0; 32];
        bytes[..4].copy_from_slice(b"PB2A");
        let xor = planes(&base, true, mode);
        if let Some(name) = name {
            bytes.extend(part_chunk(name, &xor)?);
            put(&mut bytes, 12, 32);
        } else {
            bytes.extend(xor);
            let first = bytes.len();
            put(&mut bytes, 12, first);
        }
        if bytes.len() + KEY_TAIL_OFFSET > MAX_PAYLOAD {
            return Err(FormatError("PB2 collection exceeds allocation limit"));
        }
        for (i, v) in [if name.is_some() { 6 } else { 5 }, w, h, bpp]
            .into_iter()
            .enumerate()
        {
            bytes[16 + 2 * i..18 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        Ok(Self {
            base,
            bytes,
            named: name.is_some(),
            count: 1,
            mode,
        })
    }
    pub fn push(&mut self, img: &RgbaImage, name: Option<&[u8]>) -> Result<(), FormatError> {
        if self.named != name.is_some() {
            return Err(FormatError("PB2 frame/part type mismatch"));
        }
        let patch = crate::pb_write::patch(&self.base, img, !self.named)?;
        let (flags, data) = self.mode.split(&patch);
        let mut payload = Vec::new();
        let size = 16 + flags.len() + data.len();
        let words = if self.named {
            [flags.len(), data.len(), patch.len(), 0]
        } else {
            [size, flags.len(), data.len(), patch.len()]
        };
        for v in words {
            payload.extend((v as u32).to_le_bytes());
        }
        payload.extend(flags);
        payload.extend(data);
        if let Some(name) = name {
            payload = part_chunk(name, &payload)?;
        }
        if self.bytes.len() + payload.len() + KEY_TAIL_OFFSET > MAX_PAYLOAD {
            return Err(FormatError("PB2 collection exceeds allocation limit"));
        }
        self.bytes.extend(payload);
        self.count += 1;
        Ok(())
    }
    pub fn finish(mut self) -> Vec<u8> {
        let size = self.bytes.len() + KEY_TAIL_OFFSET;
        crate::pb_write::put(&mut self.bytes, 4, size);
        crate::pb_write::put(&mut self.bytes, 8, self.count as usize);
        self.bytes.resize(size, 0);
        self.bytes
    }
}

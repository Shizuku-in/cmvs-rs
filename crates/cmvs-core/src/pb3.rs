//! PB3B header, the type-1 tiled channel decoder and the type-3 JBP1 layer. Other codecs are
//! explicitly unsupported. Format reference: pinned MIT CMVS-Engine PB3B.cpp/CPB.cpp/JBP1.cpp;
//! checked against current IDA.
use crate::{FormatError, MAX_PAYLOAD, jbp1, raster::RgbaImage};
use std::ops::Range;

fn u16_at(b: &[u8], at: usize) -> Result<u16, FormatError> {
    let v = b.get(at..at + 2).ok_or(FormatError("truncated PB3 u16"))?;
    Ok(u16::from_le_bytes([v[0], v[1]]))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, FormatError> {
    let v = b.get(at..at + 4).ok_or(FormatError("truncated PB3 u32"))?;
    Ok(u32::from_le_bytes(v.try_into().expect("four bytes")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub kind: u16,
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
    pub block: u32,
    pub flags_offset: u32,
    pub data_offset: u32,
    pub field08: u32,
}
impl Header {
    pub fn parse(raw: &[u8]) -> Result<Self, FormatError> {
        if raw.len() < 99 || raw.len() > MAX_PAYLOAD || &raw[..4] != b"PB3B" {
            return Err(FormatError("expected bounded PB3B header and key data"));
        }
        let mut header = raw[..52].to_vec();
        for i in 0..44 {
            header[8 + i] =
                (header[8 + i] ^ raw[raw.len() - 3 + i % 2]).wrapping_sub(raw[raw.len() - 47 + i]);
        }
        let h = Self {
            kind: u16_at(&header, 28)?,
            width: u16_at(&header, 30)?,
            height: u16_at(&header, 32)?,
            bpp: u16_at(&header, 34)?,
            block: u32_at(&header, 24)?,
            flags_offset: u32_at(&header, 44)?,
            data_offset: u32_at(&header, 48)?,
            field08: u32_at(&header, 8)?,
        };
        let size = u64::from(h.width) * u64::from(h.height) * 4;
        if h.width == 0 || h.height == 0 || size > MAX_PAYLOAD as u64 {
            return Err(FormatError("PB3 dimensions exceed research bounds"));
        }
        Ok(h)
    }
}

fn chunks(raw: &[u8], base: u32, count: usize) -> Result<Vec<Range<usize>>, FormatError> {
    let base = base as usize;
    if base < 52 || base > raw.len() || count * 4 > raw.len() - base {
        return Err(FormatError("PB3 channel table outside file"));
    }
    let mut at = base + count * 4;
    let mut ranges = Vec::with_capacity(count);
    for i in 0..count {
        let n = u32_at(raw, base + i * 4)? as usize;
        let end = at
            .checked_add(n)
            .ok_or(FormatError("PB3 channel length overflow"))?;
        if end > raw.len() {
            return Err(FormatError("PB3 channel data outside file"));
        }
        ranges.push(at..end);
        at = end;
    }
    Ok(ranges)
}

fn split_lzss(flags: &[u8], data: &[u8], expected: usize) -> Result<Vec<u8>, FormatError> {
    let mut ring = [0u8; 2048];
    let mut write = 2014;
    let mut at = 0;
    let mut token = 0;
    let mut output = Vec::with_capacity(expected);
    while output.len() < expected {
        let bit = flags
            .get(token / 8)
            .ok_or(FormatError("truncated PB3 LZSS flag plane"))?
            & (128 >> (token % 8));
        token += 1;
        if bit == 0 {
            let value = *data
                .get(at)
                .ok_or(FormatError("truncated PB3 LZSS literal"))?;
            at += 1;
            output.push(value);
            ring[write] = value;
            write = (write + 1) & 2047;
        } else {
            let pair = u16_at(data, at)?;
            at += 2;
            let offset = usize::from(pair >> 5);
            let n = usize::from(pair & 31) + 3;
            if n > expected - output.len() {
                return Err(FormatError("PB3 LZSS reference exceeds output"));
            }
            for j in 0..n {
                let value = ring[(offset + j) & 2047];
                output.push(value);
                ring[write] = value;
                write = (write + 1) & 2047;
            }
        }
    }
    if at != data.len() || token.div_ceil(8) != flags.len() {
        return Err(FormatError("unused PB3 LZSS stream bytes"));
    }
    Ok(output)
}

/// 0x1400a0700 for type 3: JBP1 data at offset 52 decodes to padded samples, rows are
/// cropped to the PB3 width, and alpha is 255 for 24 bits or run-length coded at the +44
/// offset for 32 bits: 0x00/0xff bytes are followed by a repeat count, others stand alone.
/// The original does not stop a run at the last pixel; Rust rejects such runs. The JBP1
/// size must match the PB3 size, which the original uses unchecked for cropping.
fn decode_jbp1(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if !matches!(h.bpp, 24 | 32) {
        return Err(FormatError("invalid PB3 type-3 bit depth"));
    }
    let image = jbp1::decode(&raw[52..])?;
    if image.width != h.width || image.height != h.height {
        return Err(FormatError("JBP1 size differs from the PB3 header"));
    }
    let (width, height) = (usize::from(h.width), usize::from(h.height));
    let mut pixels = vec![255; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let from = (y * image.padded_width + x) * 3;
            let to = (y * width + x) * 4;
            pixels[to] = image.bgr[from + 2];
            pixels[to + 1] = image.bgr[from + 1];
            pixels[to + 2] = image.bgr[from];
        }
    }
    if h.bpp == 32 {
        let mut at = h.flags_offset as usize;
        let mut pixel = 0;
        while pixel < width * height {
            let value = *raw
                .get(at)
                .ok_or(FormatError("truncated PB3 type-3 alpha runs"))?;
            let count = if matches!(value, 0 | 255) {
                let count = *raw
                    .get(at + 1)
                    .ok_or(FormatError("truncated PB3 type-3 alpha runs"))?;
                at += 2;
                usize::from(count)
            } else {
                at += 1;
                1
            };
            if count > width * height - pixel {
                return Err(FormatError("PB3 type-3 alpha run passes the last pixel"));
            }
            for alpha in pixels[pixel * 4 + 3..(pixel + count) * 4]
                .iter_mut()
                .step_by(4)
            {
                *alpha = value;
            }
            pixel += count;
        }
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

pub fn decode(raw: &[u8]) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind == 3 {
        return decode_jbp1(raw, &h);
    }
    if h.kind != 1 {
        return Err(FormatError(
            "PB3 codec not implemented; only types 1 and 3 are supported",
        ));
    }
    if !matches!(h.bpp, 24 | 32) || h.block == 0 {
        return Err(FormatError("invalid PB3 type-1 bit depth or block size"));
    }
    let channels = usize::from(h.bpp / 8);
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let count = width * height;
    let flags = chunks(raw, h.flags_offset, channels)?;
    let data = chunks(raw, h.data_offset, channels)?;
    if flags.last().expect("channels").end > h.data_offset as usize {
        return Err(FormatError("overlapping PB3 channel regions"));
    }
    let mut pixels = vec![0; count * 4];
    for alpha in pixels[3..].iter_mut().step_by(4) {
        *alpha = 255;
    }
    let block = h.block as usize;
    let tiles = width.div_ceil(block) * height.div_ceil(block);
    for channel in 0..channels {
        let f = &raw[flags[channel].clone()];
        let c = &raw[data[channel].clone()];
        let flag_bytes = u32_at(f, 0)? as usize;
        let solid_bytes = u32_at(f, 4)? as usize;
        let literal_count = u32_at(f, 8)? as usize;
        let solid_at = 12usize
            .checked_add(flag_bytes)
            .ok_or(FormatError("PB3 block table overflow"))?;
        let lzss_at = solid_at
            .checked_add(solid_bytes)
            .ok_or(FormatError("PB3 block table overflow"))?;
        if flag_bytes != tiles.div_ceil(8) || lzss_at > f.len() || literal_count > count {
            return Err(FormatError("PB3 block metadata exceeds bounds"));
        }
        let literals = split_lzss(&f[lzss_at..], c, literal_count)?;
        let mut literal = 0;
        let mut solid = 0;
        let mut tile = 0;
        let component = [2, 1, 0, 3][channel];
        for y in (0..height).step_by(block) {
            for x in (0..width).step_by(block) {
                let is_solid = f[12 + tile / 8] & (128 >> (tile % 8)) != 0;
                tile += 1;
                let constant = if is_solid {
                    if solid >= solid_bytes {
                        return Err(FormatError("PB3 solid block values exhausted"));
                    }
                    let value = f[solid_at + solid];
                    solid += 1;
                    value
                } else {
                    0
                };
                for yy in y..height.min(y.saturating_add(block)) {
                    for xx in x..width.min(x.saturating_add(block)) {
                        let value = if is_solid {
                            constant
                        } else {
                            let value = *literals
                                .get(literal)
                                .ok_or(FormatError("PB3 literal pixels exhausted"))?;
                            literal += 1;
                            value
                        };
                        pixels[(yy * width + xx) * 4 + component] = value;
                    }
                }
            }
        }
        if solid != solid_bytes || literal != literal_count {
            return Err(FormatError("unused PB3 block values"));
        }
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

//! PB3B header, types 1/2/3/5 and contextual type-6 replacement blocks. Other codecs are
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
        let header = plain_header(raw)?;
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

fn plain_header(raw: &[u8]) -> Result<Vec<u8>, FormatError> {
    if raw.len() < 99 || raw.len() > MAX_PAYLOAD || &raw[..4] != b"PB3B" {
        return Err(FormatError("expected bounded PB3B header and key data"));
    }
    let mut header = raw[..52].to_vec();
    for i in 0..44 {
        header[8 + i] =
            (header[8 + i] ^ raw[raw.len() - 3 + i % 2]).wrapping_sub(raw[raw.len() - 47 + i]);
    }
    Ok(header)
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
    let (output, tokens, bytes) = lzss_prefix(flags, data, expected)?;
    if bytes != data.len() || tokens.div_ceil(8) != flags.len() {
        return Err(FormatError("unused PB3 LZSS stream bytes"));
    }
    Ok(output)
}

// Types 5/6 store starting offsets rather than exact stream lengths. The trailing
// header-key sample may also be compressed data; stop at the declared output length.
fn lzss_prefix(
    flags: &[u8],
    data: &[u8],
    expected: usize,
) -> Result<(Vec<u8>, usize, usize), FormatError> {
    if expected > MAX_PAYLOAD {
        return Err(FormatError("PB3 LZSS output exceeds research bounds"));
    }
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
    Ok((output, token, at))
}

/// 0x14009fad0: four independently compressed delta planes, cumulative across rows.
fn decode_delta(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if h.bpp != 32 {
        return Err(FormatError("PB3 type 5 requires 32 bits per pixel"));
    }
    let count = usize::from(h.width) * usize::from(h.height);
    let mut pixels = vec![0; count * 4];
    for (plane, component) in [2, 1, 0, 3].into_iter().enumerate() {
        let flag_at = offset(84, u32_at(raw, 52 + plane * 8)?)?;
        let data_at = offset(84, u32_at(raw, 56 + plane * 8)?)?;
        let flags = raw
            .get(flag_at..data_at)
            .ok_or(FormatError("invalid PB3 type-5 flag offsets"))?;
        let data = raw
            .get(data_at..)
            .ok_or(FormatError("invalid PB3 type-5 data offset"))?;
        let (deltas, _, _) = lzss_prefix(flags, data, count)?;
        let mut value = 0u8;
        for (i, delta) in deltas.into_iter().enumerate() {
            value = value.wrapping_add(delta);
            pixels[i * 4 + component] = value;
        }
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

fn offset(base: usize, relative: u32) -> Result<usize, FormatError> {
    base.checked_add(relative as usize)
        .ok_or(FormatError("PB3 offset overflow"))
}

/// 0x140036ae0 case 6: encrypted, NUL-terminated 32-byte stem plus `.pb3`.
/// Unlike the reference extractor, the original recursively decodes any supported base.
pub fn reference_name(raw: &[u8]) -> Result<Option<Vec<u8>>, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind != 6 {
        return Ok(None);
    }
    if h.bpp != 32 {
        return Err(FormatError("PB3 type 6 requires 32 bits per pixel"));
    }
    let key = [
        0xa6, 0x75, 0xf3, 0x9c, 0xc5, 0x69, 0x78, 0xa3, 0x3e, 0xa5, 0x4f, 0x79, 0x59, 0xfe, 0x3a,
        0xc7,
    ];
    let stem: Vec<_> = raw[52..84]
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % 16])
        .collect();
    let end = stem
        .iter()
        .position(|b| *b == 0)
        .ok_or(FormatError("unterminated PB3 reference name"))?;
    if end == 0 {
        return Err(FormatError("empty PB3 reference name"));
    }
    let mut name = stem[..end].to_vec();
    name.extend(b".pb3");
    crate::cpz::lookup_key(&name)?;
    Ok(Some(name))
}

/// 0x14009f820: copy BGRA replacements into 8x8 tiles with a zero mask bit.
/// A one bit preserves the base, including its alpha; there is no alpha blending.
pub fn decode_with_base(raw: &[u8], base: RgbaImage) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind != 6 || h.bpp != 32 {
        return Err(FormatError("expected a 32-bit PB3 type-6 image"));
    }
    if base.width() != u32::from(h.width) || base.height() != u32::from(h.height) {
        return Err(FormatError(
            "PB3 reference dimensions differ from the overlay",
        ));
    }
    let header = plain_header(raw)?;
    let flag_at = offset(32, u32_at(&header, 12)?)?;
    if flag_at < 84 {
        return Err(FormatError("PB3 type-6 flags overlap the reference name"));
    }
    let data_at = offset(flag_at, h.flags_offset)?;
    let flags = raw
        .get(flag_at..data_at)
        .ok_or(FormatError("invalid PB3 type-6 flag offsets"))?;
    let data = raw
        .get(data_at..)
        .ok_or(FormatError("invalid PB3 type-6 data offset"))?;
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let tiles = width.div_ceil(8) * height.div_ceil(8);
    if h.block as usize > 8 + tiles.div_ceil(8) + width * height * 4 {
        return Err(FormatError("PB3 type-6 patch exceeds the image byte bound"));
    }
    let (patch, _, _) = lzss_prefix(flags, data, h.block as usize)?;
    let masks = u32_at(&patch, 0)? as usize;
    if masks != tiles.div_ceil(8) || masks > patch.len().saturating_sub(8) {
        return Err(FormatError("invalid PB3 type-6 tile mask length"));
    }
    let mut at = 8 + masks;
    let mut pixels = base.pixels().to_vec();
    let mut tile = 0;
    for y in (0..height).step_by(8) {
        for x in (0..width).step_by(8) {
            if patch[8 + tile / 8] & (128 >> (tile % 8)) == 0 {
                for yy in y..height.min(y + 8) {
                    for xx in x..width.min(x + 8) {
                        let bgra = patch
                            .get(at..at + 4)
                            .ok_or(FormatError("truncated PB3 type-6 replacement pixels"))?;
                        let to = (yy * width + xx) * 4;
                        pixels[to..to + 4].copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
                        at += 4;
                    }
                }
            }
            tile += 1;
        }
    }
    if at != patch.len() {
        return Err(FormatError("unused PB3 type-6 replacement pixels"));
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

/// 0x1400a0700 for types 2/3: JBP1 data at offset 52 decodes to padded samples, rows are
/// cropped to the PB3 width, and alpha is 255 for 24 bits or run-length coded at the +44
/// offset for 32 bits: 0x00/0xff bytes are followed by a repeat count, others stand alone.
/// The original does not stop a run at the last pixel; Rust rejects such runs. The JBP1
/// size must match the PB3 size, which the original uses unchecked for cropping.
fn decode_jbp1(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if !matches!(h.bpp, 24 | 32) {
        return Err(FormatError("invalid PB3 JBP1 bit depth"));
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
                .ok_or(FormatError("truncated PB3 JBP1 alpha runs"))?;
            let count = if matches!(value, 0 | 255) {
                let count = *raw
                    .get(at + 1)
                    .ok_or(FormatError("truncated PB3 JBP1 alpha runs"))?;
                at += 2;
                usize::from(count)
            } else {
                at += 1;
                1
            };
            if count > width * height - pixel {
                return Err(FormatError("PB3 JBP1 alpha run passes the last pixel"));
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
    if matches!(h.kind, 2 | 3) {
        return decode_jbp1(raw, &h);
    }
    if h.kind == 5 {
        return decode_delta(raw, &h);
    }
    if h.kind == 6 {
        return Err(FormatError("PB3 type 6 requires a reference image source"));
    }
    if h.kind != 1 {
        return Err(FormatError(
            "PB3 codec not implemented; standalone types 1, 2, 3 and 5 are supported",
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

//! PB3B header, types 1/2/3/5/7 and contextual type-6/8 replacement blocks. Other codecs are
//! explicitly unsupported. Format reference: pinned MIT CMVS-Engine PB3B.cpp/CPB.cpp/JBP1.cpp;
//! checked against current IDA.
use crate::{FormatError, MAX_PAYLOAD, image::DecodeThreads, jbp1, raster::RgbaImage};
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
    // The original keeps a 2048-byte ring, zeroed, whose writes start at 2014: output byte
    // i lands in slot (2014 + i) & 2047. Reading slot `offset` is therefore reading the
    // output `distance` bytes back (2048 for the slot about to be written), or zero
    // before the first write reached that slot. No ring is kept.
    let mut at = 0;
    let mut token = 0;
    let mut output = vec![0u8; expected];
    let mut length = 0;
    while length < expected {
        let flag = *flags
            .get(token / 8)
            .ok_or(FormatError("truncated PB3 LZSS flag plane"))?;
        if flag == 0
            && token % 8 == 0
            && expected - length >= 8
            && let Some(values) = data.get(at..at + 8)
        {
            // Eight literals.
            let values: &[u8; 8] = values.try_into().unwrap();
            let target: &mut [u8; 8] = (&mut output[length..length + 8]).try_into().unwrap();
            *target = *values;
            token += 8;
            at += 8;
            length += 8;
            continue;
        }
        // A flag byte covers up to eight tokens; stop early once the output is complete.
        for bit in token % 8..8 {
            if length == expected {
                break;
            }
            token += 1;
            if flag & (128 >> bit) == 0 {
                output[length] = *data
                    .get(at)
                    .ok_or(FormatError("truncated PB3 LZSS literal"))?;
                at += 1;
                length += 1;
            } else {
                let pair = u16_at(data, at)?;
                at += 2;
                let offset = usize::from(pair >> 5);
                let n = usize::from(pair & 31) + 3;
                if n > expected - length {
                    return Err(FormatError("PB3 LZSS reference exceeds output"));
                }
                let distance = match (2014 + length).wrapping_sub(offset) & 2047 {
                    0 => 2048,
                    d => d,
                };
                if length >= distance {
                    for i in length..length + n {
                        output[i] = output[i - distance];
                    }
                } else {
                    for i in length..length + n {
                        output[i] = if i >= distance {
                            output[i - distance]
                        } else {
                            0
                        };
                    }
                }
                length += n;
            }
        }
    }
    Ok((output, token, at))
}

/// 0x14009fad0: four independently compressed delta planes, cumulative across rows.
fn decode_delta(raw: &[u8], h: &Header, threads: DecodeThreads) -> Result<RgbaImage, FormatError> {
    if h.bpp != 32 {
        return Err(FormatError("PB3 type 5 requires 32 bits per pixel"));
    }
    let count = usize::from(h.width) * usize::from(h.height);
    let planes = map_planes(threads, count, &[0, 1, 2, 3], |&plane| {
        let flag_at = offset(84, u32_at(raw, 52 + plane * 8)?)?;
        let data_at = offset(84, u32_at(raw, 56 + plane * 8)?)?;
        let flags = raw
            .get(flag_at..data_at)
            .ok_or(FormatError("invalid PB3 type-5 flag offsets"))?;
        let data = raw
            .get(data_at..)
            .ok_or(FormatError("invalid PB3 type-5 data offset"))?;
        let (mut values, _, _) = lzss_prefix(flags, data, count)?;
        let mut value = 0u8;
        for delta in &mut values {
            value = value.wrapping_add(*delta);
            *delta = value;
        }
        Ok(values)
    })?;
    RgbaImage::new(
        u32::from(h.width),
        u32::from(h.height),
        interleave(&planes, count),
    )
}

/// Decodes the planes of an image, in file order (blue, green, red, alpha), on the
/// calling thread and on `threads` when the image is large enough to gain from it.
/// The first failing plane in file order decides the error, as when decoding in turn.
fn map_planes(
    threads: DecodeThreads,
    pixels: usize,
    planes: &[usize],
    decode: impl Fn(&usize) -> Result<Vec<u8>, FormatError> + Sync,
) -> Result<Vec<Vec<u8>>, FormatError> {
    let threads = if pixels >= 1 << 16 {
        threads
    } else {
        DecodeThreads::default()
    };
    threads.map(planes, decode).into_iter().collect()
}

/// RGBA from planes in file order; a missing alpha plane is opaque.
fn interleave(planes: &[Vec<u8>], count: usize) -> Vec<u8> {
    let mut pixels = vec![255; count * 4];
    let [blue, green, red] = [&planes[0], &planes[1], &planes[2]];
    let colors = red.iter().zip(green).zip(blue);
    for ((pixel, ((r, g), b)), i) in pixels
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(colors)
        .zip(0..)
    {
        *pixel = [*r, *g, *b, planes.get(3).map_or(255, |alpha| alpha[i])];
    }
    pixels
}

fn offset(base: usize, relative: u32) -> Result<usize, FormatError> {
    base.checked_add(relative as usize)
        .ok_or(FormatError("PB3 offset overflow"))
}

/// HASSAKU x64 0x140091f00: one split LZSS stream of interleaved BGRA bytes.
fn decode_interleaved(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if h.bpp != 32 {
        return Err(FormatError("PB3 type 7 requires 32 bits per pixel"));
    }
    let flag_at = offset(60, u32_at(raw, 52)?)?;
    let data_at = offset(60, u32_at(raw, 56)?)?;
    let flags = raw
        .get(flag_at..data_at)
        .ok_or(FormatError("invalid PB3 type-7 flag offsets"))?;
    let data = raw
        .get(data_at..)
        .ok_or(FormatError("invalid PB3 type-7 data offset"))?;
    let (mut pixels, _, _) = lzss_prefix(
        flags,
        data,
        usize::from(h.width) * usize::from(h.height) * 4,
    )?;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

/// 0x140036ae0 case 6: encrypted, NUL-terminated 32-byte stem plus `.pb3`.
/// Unlike the reference extractor, the original recursively decodes any supported base.
pub fn reference_name(raw: &[u8]) -> Result<Option<Vec<u8>>, FormatError> {
    let h = Header::parse(raw)?;
    if !matches!(h.kind, 6 | 8) {
        return Ok(None);
    }
    if h.bpp != 32 {
        return Err(FormatError("PB3 types 6/8 require 32 bits per pixel"));
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
    if !matches!(h.kind, 6 | 8) || h.bpp != 32 {
        return Err(FormatError("expected a 32-bit PB3 type-6/8 image"));
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
                let columns = width.min(x + 8) - x;
                for yy in y..height.min(y + 8) {
                    let bgra = patch
                        .get(at..at + columns * 4)
                        .ok_or(FormatError("truncated PB3 type-6 replacement pixels"))?;
                    let to = (yy * width + x) * 4;
                    for (target, source) in pixels[to..to + columns * 4]
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(bgra.as_chunks::<4>().0)
                    {
                        *target = [source[2], source[1], source[0], source[3]];
                    }
                    at += columns * 4;
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

/// 0x1400a0700 for types 2/3: JBP1 data at offset 52 decodes to padded samples cropped to
/// the PB3 size (the JBP1 decoder stores only the cropped rows), and alpha is 255 for 24 bits or run-length coded at the +44
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
    let mut pixels = image.rgba;
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
    decode_with(raw, DecodeThreads::default())
}

/// [`decode`], spreading independently coded planes (types 1 and 5) over `threads`.
pub fn decode_with(raw: &[u8], threads: DecodeThreads) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    if matches!(h.kind, 2 | 3) {
        return decode_jbp1(raw, &h);
    }
    if h.kind == 7 {
        return decode_interleaved(raw, &h);
    }
    if h.kind == 5 {
        return decode_delta(raw, &h, threads);
    }
    if matches!(h.kind, 6 | 8) {
        return Err(FormatError(
            "PB3 types 6/8 require a reference image source",
        ));
    }
    if h.kind != 1 {
        return Err(FormatError(
            "PB3 codec not implemented; standalone types 1, 2, 3, 5 and 7 are supported",
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
    let block = h.block as usize;
    let tiles = width.div_ceil(block) * height.div_ceil(block);
    let channels: Vec<usize> = (0..channels).collect();
    let planes = map_planes(threads, count, &channels, |&channel| {
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
        let mut plane = vec![0; count];
        let mut literal = 0;
        let mut solid = 0;
        let mut tile = 0;
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
                let columns = width.min(x.saturating_add(block)) - x;
                for yy in y..height.min(y.saturating_add(block)) {
                    let row = &mut plane[yy * width + x..][..columns];
                    if is_solid {
                        row.fill(constant);
                    } else {
                        let values = literals
                            .get(literal..literal + columns)
                            .ok_or(FormatError("PB3 literal pixels exhausted"))?;
                        literal += columns;
                        row.copy_from_slice(values);
                    }
                }
            }
        }
        if solid != solid_bytes || literal != literal_count {
            return Err(FormatError("unused PB3 block values"));
        }
        Ok(plane)
    })?;
    let pixels = interleave(&planes, count);
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

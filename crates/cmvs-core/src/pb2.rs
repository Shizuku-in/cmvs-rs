//! PB2A image decoding used by CVNS.
//!
//! Supports codecs 1..=6 (`LZSS` swizzled planes, block-solid `LZSS`, `JBP1`,
//! multi-frame XOR-delta animations, and named multi-part character sprites).

// Format reference: cvns.exe SHA-256 8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa,
// sub_43C990 and decoders sub_479180 / sub_479700 / sub_479B00 / sub_479F00 / sub_47A0E0 (base 0x400000).
use crate::{
    FormatError, MAX_PAYLOAD, TextEncoding,
    image::DecodeThreads,
    jbp1,
    pb3::{apply_jbp1_alpha, interleave, lzss_prefix, map_planes, u16_at, u32_at},
    raster::RgbaImage,
    text::equal_ignoring_ascii_case,
};
use std::ops::Range;

#[cfg(feature = "encode")]
mod write;
#[cfg(feature = "encode")]
pub use write::{CollectionWriter, StaticType, encode, encode_static};

const HEADER_SIZE: usize = 32;
const KEY_TAIL_OFFSET: usize = 31;

/// Parsed PB2A image header after unscrambling bytes `8..32`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub payload_size: u32,
    pub count: u32,
    pub diff_offset: u32,
    pub kind: u16,
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
    pub flags_offset: u32,
    pub data_offset: u32,
}

impl Header {
    /// Parses and unscrambles the 32-byte PB2A header and validates image dimensions.
    pub fn parse(raw: &[u8]) -> Result<Self, FormatError> {
        let header = plain_header(raw)?;
        let h = Self {
            payload_size: u32_at(&header, 4)?,
            count: u32_at(&header, 8)?,
            diff_offset: u32_at(&header, 12)?,
            kind: u16_at(&header, 16)?,
            width: u16_at(&header, 18)?,
            height: u16_at(&header, 20)?,
            bpp: u16_at(&header, 22)?,
            flags_offset: u32_at(&header, 24)?,
            data_offset: u32_at(&header, 28)?,
        };
        let size = u64::from(h.width) * u64::from(h.height) * 4;
        if h.width == 0 || h.height == 0 || size > MAX_PAYLOAD as u64 {
            return Err(FormatError("PB2 dimensions exceed research bounds"));
        }
        Ok(h)
    }
}

/// Decodes a PB2A image into top-down RGBA pixels, taking base frame/part 0 for types 5/6.
pub fn decode(raw: &[u8]) -> Result<RgbaImage, FormatError> {
    decode_with(raw, DecodeThreads::default())
}

/// [`decode`] using caller-configured decode worker threads.
pub fn decode_with(raw: &[u8], threads: DecodeThreads) -> Result<RgbaImage, FormatError> {
    decode_part_with(raw, None, threads)
}

/// Decodes a named sub-image inside a PB2A type-6 archive, or the main image for other types.
///
/// For type 6, passing `None` returns chunk 0 (the base pose/image); passing `Some(name)`
/// searches chunks using Shift-JIS ASCII casefolding as in `sub_479180` / `sub_472CD0`.
pub fn decode_part_with(
    raw: &[u8],
    part: Option<&[u8]>,
    threads: DecodeThreads,
) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    match h.kind {
        1 => decode_swizzled(raw, &h),
        2 => decode_blocked(raw, &h, threads),
        3 | 4 => decode_jbp1(raw, &h),
        5 => decode_frame_with(raw, &h, 0, threads),
        6 => decode_multipart(raw, &h, part, None, threads),
        _ => Err(FormatError("unsupported PB2 image codec")),
    }
}

/// Decodes frame `frame` (`0..header.count`) of a PB2A type-5 animation.
/// Decodes a single frame (0 = base, 1..count-1 = delta frame) from a PB2 type-5 animation.
pub fn decode_frame(raw: &[u8], frame: usize) -> Result<RgbaImage, FormatError> {
    decode_frame_with_threads(raw, frame, DecodeThreads::default())
}

/// [`decode_frame`] using caller-configured decode worker threads.
pub fn decode_frame_with_threads(
    raw: &[u8],
    frame: usize,
    threads: DecodeThreads,
) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind != 5 {
        return Err(FormatError("expected PB2 type-5 animation"));
    }
    decode_frame_with(raw, &h, frame, threads)
}

/// Lists all named sub-images present in a PB2A type-6 file.
pub fn part_names(raw: &[u8]) -> Result<Vec<Vec<u8>>, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind != 6 {
        return Ok(Vec::new());
    }
    // Each part needs a length word, a padded name, and a 16-byte payload header.
    const MIN_CHUNK_SIZE: usize = 4 + 4 + 16;
    if u64::from(h.count) > ((raw.len() - HEADER_SIZE) / MIN_CHUNK_SIZE) as u64 {
        return Err(FormatError("PB2 type-6 chunk count exceeds file size"));
    }
    let mut names = Vec::new();
    let mut pos = HEADER_SIZE;
    for _ in 0..h.count {
        let (chunk_size, name, _) = read_part_chunk(raw, pos)?;
        names.push(name.to_vec());
        pos = pos
            .checked_add(chunk_size)
            .ok_or(FormatError("PB2 type-6 chunk offset overflow"))?;
    }
    Ok(names)
}

fn plain_header(raw: &[u8]) -> Result<[u8; HEADER_SIZE], FormatError> {
    if raw.len() < 64 || raw.len() > MAX_PAYLOAD || raw.get(..4) != Some(b"PB2A") {
        return Err(FormatError("expected bounded PB2A header and key data"));
    }
    let key = u16_at(raw, raw.len() - 3)?;
    let tail = &raw[raw.len() - KEY_TAIL_OFFSET..];
    let mut header: [u8; HEADER_SIZE] = raw[..HEADER_SIZE].try_into().expect("32 bytes");
    for chunk in header[8..].as_chunks_mut::<2>().0 {
        let unxored = u16::from_le_bytes([chunk[0], chunk[1]]) ^ key;
        chunk.copy_from_slice(&unxored.to_le_bytes());
    }
    for (i, byte) in header[8..].iter_mut().enumerate() {
        *byte = byte.wrapping_sub(tail[4 + i]);
    }
    Ok(header)
}

/// 0x47A0E0: single LZSS stream unpacked into sequential channels of 8x8 tile blocks.
fn decode_swizzled(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if !matches!(h.bpp, 24 | 32) {
        return Err(FormatError("invalid PB2 type-1 bit depth"));
    }
    let channels = usize::from(h.bpp / 8);
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let expected = width * height * channels;
    let flags = raw
        .get(h.flags_offset as usize..)
        .ok_or(FormatError("invalid PB2 type-1 flag offset"))?;
    let data = raw
        .get(h.data_offset as usize..)
        .ok_or(FormatError("invalid PB2 type-1 data offset"))?;
    let (unpacked, _, _) = lzss_prefix(flags, data, expected)?;
    let mut pixels = vec![255u8; width * height * 4];
    let mut at = 0;
    for channel in 0..channels {
        let lane = match channel {
            0 => 2,
            1 => 1,
            2 => 0,
            _ => 3,
        };
        for y in (0..height).step_by(8) {
            let th = (height - y).min(8);
            for x in (0..width).step_by(8) {
                let tw = (width - x).min(8);
                for dy in 0..th {
                    let row = (y + dy) * width + x;
                    for dx in 0..tw {
                        pixels[(row + dx) * 4 + lane] = unpacked[at];
                        at += 1;
                    }
                }
            }
        }
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), pixels)
}

/// 0x479B00: per-channel 8x8 solid-block masks plus LZSS residual pixels.
fn decode_blocked(
    raw: &[u8],
    h: &Header,
    threads: DecodeThreads,
) -> Result<RgbaImage, FormatError> {
    if !matches!(h.bpp, 24 | 32) {
        return Err(FormatError("invalid PB2 type-2 bit depth"));
    }
    let channels = usize::from(h.bpp / 8);
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let count = width * height;
    let flags = chunks(raw, h.flags_offset, channels)?;
    let data = chunks(raw, h.data_offset, channels)?;
    let tiles = width.div_ceil(8) * height.div_ceil(8);
    let channel_ids: Vec<usize> = (0..channels).collect();
    let planes = map_planes(threads, count, &channel_ids, |&channel| {
        let f = &raw[flags[channel].clone()];
        let d = &raw[data[channel].clone()];
        let mask_bytes = u32_at(f, 0)? as usize;
        let solid_bytes = u32_at(f, 4)? as usize;
        let literal_count = u32_at(f, 8)? as usize;
        let solid_at = 12usize
            .checked_add(mask_bytes)
            .ok_or(FormatError("PB2 block table overflow"))?;
        let lzss_at = solid_at
            .checked_add(solid_bytes)
            .ok_or(FormatError("PB2 block table overflow"))?;
        if mask_bytes != tiles.div_ceil(8) || lzss_at > f.len() || literal_count > count {
            return Err(FormatError("PB2 block metadata exceeds bounds"));
        }
        let (literals, _, _) = lzss_prefix(&f[lzss_at..], d, literal_count)?;
        let mut plane = vec![0u8; count];
        let mut literal = 0;
        let mut solid = 0;
        let mut tile = 0;
        for y in (0..height).step_by(8) {
            let rows = height.min(y + 8);
            for x in (0..width).step_by(8) {
                let is_solid = f[12 + tile / 8] & (128 >> (tile % 8)) != 0;
                tile += 1;
                let constant = if is_solid {
                    let value = *f
                        .get(solid_at + solid)
                        .filter(|_| solid < solid_bytes)
                        .ok_or(FormatError("PB2 solid block values exhausted"))?;
                    solid += 1;
                    value
                } else {
                    0
                };
                let cols = width.min(x + 8) - x;
                for yy in y..rows {
                    let row = &mut plane[yy * width + x..][..cols];
                    if is_solid {
                        row.fill(constant);
                    } else {
                        let values = literals
                            .get(literal..literal + cols)
                            .ok_or(FormatError("PB2 literal pixels exhausted"))?;
                        literal += cols;
                        row.copy_from_slice(values);
                    }
                }
            }
        }
        if solid != solid_bytes || literal != literal_count {
            return Err(FormatError("unused PB2 block values"));
        }
        Ok(plane)
    })?;
    RgbaImage::new(
        u32::from(h.width),
        u32::from(h.height),
        interleave(&planes, count),
    )
}

/// 0x479F00: JBP1 body at offset 32 with optional 32-bpp RLE alpha stream at `flags_offset`.
fn decode_jbp1(raw: &[u8], h: &Header) -> Result<RgbaImage, FormatError> {
    if !matches!(h.bpp, 24 | 32) {
        return Err(FormatError("invalid PB2 JBP1 bit depth"));
    }
    let mut image = jbp1::decode(&raw[HEADER_SIZE..])?;
    if image.width != h.width || image.height != h.height {
        return Err(FormatError("JBP1 size differs from PB2 header"));
    }
    if h.bpp == 32 {
        let stream = raw
            .get(h.flags_offset as usize..)
            .ok_or(FormatError("truncated PB2 JBP1 alpha runs"))?;
        apply_jbp1_alpha(
            stream,
            &mut image.rgba,
            usize::from(h.width),
            usize::from(h.height),
        )?;
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), image.rgba)
}

/// 0x479700: 4-plane XOR-coded base image with optional 8x8 BGR delta frame overlay.
fn decode_frame_with(
    raw: &[u8],
    h: &Header,
    frame: usize,
    threads: DecodeThreads,
) -> Result<RgbaImage, FormatError> {
    if frame >= h.count as usize {
        return Err(FormatError("PB2 type-5 frame index out of range"));
    }
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let mut rgba = decode_xor_base(raw, HEADER_SIZE, 64, width, height, threads)?;
    if frame > 0 {
        let mut pos = usize::try_from(h.diff_offset)
            .ok()
            .filter(|&p| p >= 64 && p < raw.len())
            .ok_or(FormatError("PB2 type-5 diff offset out of bounds"))?;
        for _ in 1..frame {
            let chunk_size = u32_at(raw, pos)? as usize;
            if chunk_size < 16 {
                return Err(FormatError("invalid PB2 type-5 chunk size"));
            }
            pos = pos
                .checked_add(chunk_size)
                .ok_or(FormatError("PB2 type-5 chunk offset overflow"))?;
            if pos >= raw.len() {
                return Err(FormatError("PB2 type-5 chunk out of bounds"));
            }
        }
        let chunk = &raw[pos..];
        let flags_len = u32_at(chunk, 4)? as usize;
        let unpacked_len = u32_at(chunk, 12)? as usize;
        let (bw, bh) = (width / 8, height / 8);
        let max_patch = 8usize
            .saturating_add((bw * bh).div_ceil(8))
            .saturating_add(bw * bh * 64 * 3);
        if unpacked_len > max_patch || unpacked_len > MAX_PAYLOAD {
            return Err(FormatError("PB2 type-5 diff exceeds bounds"));
        }
        let flags = chunk
            .get(16..)
            .ok_or(FormatError("truncated PB2 type-5 diff header"))?;
        let data = chunk
            .get(16 + flags_len..)
            .ok_or(FormatError("PB2 type-5 diff data offset out of bounds"))?;
        let (patch, _, _) = lzss_prefix(flags, data, unpacked_len)?;
        let mask_bytes = u32_at(&patch, 0)? as usize;
        if mask_bytes > patch.len().saturating_sub(8) {
            return Err(FormatError("invalid PB2 type-5 tile mask length"));
        }
        let mut at = 8 + mask_bytes;
        let mut tile = 0;
        for by in 0..bh {
            for bx in 0..bw {
                if patch
                    .get(8 + tile / 8)
                    .ok_or(FormatError("truncated PB2 type-5 tile mask"))?
                    & (128 >> (tile % 8))
                    == 0
                {
                    for dy in 0..8 {
                        let bgr = patch
                            .get(at..at + 8 * 3)
                            .ok_or(FormatError("truncated PB2 type-5 pixels"))?;
                        let to = ((by * 8 + dy) * width + bx * 8) * 4;
                        for (dst, src) in rgba[to..to + 8 * 4]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip(bgr.as_chunks::<3>().0)
                        {
                            *dst = [src[2], src[1], src[0], 255];
                        }
                        at += 8 * 3;
                    }
                }
                tile += 1;
            }
        }
    }
    RgbaImage::new(u32::from(h.width), u32::from(h.height), rgba)
}

/// 0x479180: named chunk 0 base image plus optional named 8x8 BGRA replacement chunks.
/// Decode a part in file order, including shadowed or empty names.
pub fn decode_part_index(raw: &[u8], index: usize) -> Result<RgbaImage, FormatError> {
    let h = Header::parse(raw)?;
    if h.kind != 6 || index >= h.count as usize {
        return Err(FormatError("invalid PB2 part index"));
    }
    decode_multipart(raw, &h, None, Some(index), DecodeThreads::default())
}

fn decode_multipart(
    raw: &[u8],
    h: &Header,
    part: Option<&[u8]>,
    index: Option<usize>,
    threads: DecodeThreads,
) -> Result<RgbaImage, FormatError> {
    if h.count == 0 {
        return Err(FormatError("PB2 type-6 archive has zero chunks"));
    }
    let width = usize::from(h.width);
    let height = usize::from(h.height);
    let (chunk0_size, base_name, payload_at) = read_part_chunk(raw, HEADER_SIZE)?;
    let mut rgba = decode_xor_base(raw, payload_at, payload_at + 32, width, height, threads)?;
    let matches = |i, name: &[u8]| {
        if let Some(index) = index {
            return Ok(i == index);
        }
        match part {
            None => Ok(i == 0),
            Some([]) => Err(FormatError("PB2 type-6 part name is empty")),
            Some(target) => equal_ignoring_ascii_case(TextEncoding::ShiftJis, name, target)
                .map_err(|_| FormatError("invalid Shift-JIS sequence in PB2 part name")),
        }
    };
    if matches(0, base_name)? {
        return RgbaImage::new(u32::from(h.width), u32::from(h.height), rgba);
    }

    let mut pos = HEADER_SIZE
        .checked_add(chunk0_size)
        .ok_or(FormatError("PB2 type-6 chunk offset overflow"))?;
    for i in 1..h.count {
        let (chunk_size, name, sub_at) = read_part_chunk(raw, pos)?;
        if matches(i as usize, name)? {
            let flags_off = u32_at(raw, sub_at)? as usize;
            let unpacked_len = u32_at(raw, sub_at + 8)? as usize;
            let tiles = width.div_ceil(8) * height.div_ceil(8);
            let max_patch = 8usize
                .saturating_add(tiles.div_ceil(8))
                .saturating_add(width * height * 4);
            if unpacked_len > max_patch || unpacked_len > MAX_PAYLOAD {
                return Err(FormatError("PB2 type-6 patch exceeds image byte bound"));
            }
            let flags_at = sub_at
                .checked_add(16)
                .ok_or(FormatError("PB2 type-6 sub-header overflow"))?;
            let data_at = flags_at
                .checked_add(flags_off)
                .ok_or(FormatError("PB2 type-6 data offset overflow"))?;
            let flags = raw
                .get(flags_at..)
                .ok_or(FormatError("truncated PB2 type-6 patch flags"))?;
            let data = raw
                .get(data_at..)
                .ok_or(FormatError("truncated PB2 type-6 patch data"))?;
            let (patch, _, _) = lzss_prefix(flags, data, unpacked_len)?;
            let mask_bytes = u32_at(&patch, 0)? as usize;
            if mask_bytes != tiles.div_ceil(8) || mask_bytes > patch.len().saturating_sub(8) {
                return Err(FormatError("invalid PB2 type-6 tile mask length"));
            }
            let mut at = 8 + mask_bytes;
            let mut tile = 0;
            for y in (0..height).step_by(8) {
                let rows = height.min(y + 8);
                for x in (0..width).step_by(8) {
                    if patch[8 + tile / 8] & (128 >> (tile % 8)) == 0 {
                        let cols = width.min(x + 8) - x;
                        for yy in y..rows {
                            let bgra = patch
                                .get(at..at + cols * 4)
                                .ok_or(FormatError("truncated PB2 type-6 replacement pixels"))?;
                            let to = (yy * width + x) * 4;
                            for (dst, src) in rgba[to..to + cols * 4]
                                .as_chunks_mut::<4>()
                                .0
                                .iter_mut()
                                .zip(bgra.as_chunks::<4>().0)
                            {
                                *dst = [src[2], src[1], src[0], src[3]];
                            }
                            at += cols * 4;
                        }
                    }
                    tile += 1;
                }
            }
            if at != patch.len() {
                return Err(FormatError("unused PB2 type-6 replacement pixels"));
            }
            return RgbaImage::new(u32::from(h.width), u32::from(h.height), rgba);
        }
        pos = pos
            .checked_add(chunk_size)
            .ok_or(FormatError("PB2 type-6 chunk offset overflow"))?;
    }
    Err(FormatError("PB2 type-6 part name not found"))
}

fn read_part_chunk(raw: &[u8], pos: usize) -> Result<(usize, &[u8], usize), FormatError> {
    let chunk_size = u32_at(raw, pos)? as usize;
    let end = pos
        .checked_add(chunk_size)
        .filter(|&end| end <= raw.len())
        .ok_or(FormatError("PB2 type-6 chunk exceeds file"))?;
    let name_region = raw
        .get(pos + 4..end)
        .ok_or(FormatError("truncated PB2 type-6 chunk name"))?;
    let nul = name_region
        .iter()
        .position(|&b| b == 0)
        .filter(|&len| len < 128)
        .ok_or(FormatError("unterminated PB2 type-6 chunk name"))?;
    let aligned = (nul + 4) & !3;
    let payload_at = pos + 4 + aligned;
    if payload_at + 16 > end {
        return Err(FormatError("truncated PB2 type-6 chunk payload header"));
    }
    Ok((chunk_size, &name_region[..nul], payload_at))
}

fn decode_xor_base(
    raw: &[u8],
    table_at: usize,
    stream_base: usize,
    width: usize,
    height: usize,
    threads: DecodeThreads,
) -> Result<Vec<u8>, FormatError> {
    let count = width * height;
    let table = raw
        .get(table_at..table_at + 32)
        .ok_or(FormatError("truncated PB2 XOR plane table"))?;
    let planes = map_planes(threads, count, &[0, 1, 2, 3], |&plane| {
        let f_off = u32_at(table, plane * 8)? as usize;
        let d_off = u32_at(table, plane * 8 + 4)? as usize;
        let flags = raw
            .get(
                stream_base
                    .checked_add(f_off)
                    .ok_or(FormatError("PB2 plane flag offset overflow"))?..,
            )
            .ok_or(FormatError("invalid PB2 plane flag offset"))?;
        let data = raw
            .get(
                stream_base
                    .checked_add(d_off)
                    .ok_or(FormatError("PB2 plane data offset overflow"))?..,
            )
            .ok_or(FormatError("invalid PB2 plane data offset"))?;
        let (values, _, _) = lzss_prefix(flags, data, count)?;
        Ok(values)
    })?;
    let mut rgba = vec![0u8; count * 4];
    for (i, dst) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let a = planes[3][i];
        let r = planes[2][i] ^ a;
        let g = planes[1][i] ^ r;
        let b = planes[0][i] ^ g;
        *dst = [r, g, b, a];
    }
    Ok(rgba)
}

fn chunks(raw: &[u8], base: u32, count: usize) -> Result<Vec<Range<usize>>, FormatError> {
    let base = base as usize;
    if base < HEADER_SIZE || base > raw.len() || count * 4 > raw.len() - base {
        return Err(FormatError("PB2 channel table outside file"));
    }
    let mut at = base + count * 4;
    let mut ranges = Vec::with_capacity(count);
    for i in 0..count {
        let n = u32_at(raw, base + i * 4)? as usize;
        let end = at
            .checked_add(n)
            .ok_or(FormatError("PB2 channel length overflow"))?;
        if end > raw.len() {
            return Err(FormatError("PB2 channel data outside file"));
        }
        ranges.push(at..end);
        at = end;
    }
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lzss_literals(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        (vec![0u8; bytes.len().div_ceil(8)], bytes.to_vec())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_pb2(
        kind: u16,
        width: u16,
        height: u16,
        bpp: u16,
        count: u32,
        diff_offset: u32,
        flags_offset: u32,
        data_offset: u32,
        body: &[u8],
    ) -> Vec<u8> {
        let mut raw = vec![0u8; HEADER_SIZE];
        raw.extend_from_slice(body);
        if raw.len() < 33 {
            raw.resize(33, 0);
        }
        let mut tail = [0u8; KEY_TAIL_OFFSET];
        for (i, byte) in tail[4..28].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        let key = 0x5a3cu16;
        tail[28..30].copy_from_slice(&key.to_le_bytes());
        raw.extend_from_slice(&tail);

        let total = raw.len() as u32;
        raw[..4].copy_from_slice(b"PB2A");
        raw[4..8].copy_from_slice(&total.to_le_bytes());
        raw[8..12].copy_from_slice(&count.to_le_bytes());
        raw[12..16].copy_from_slice(&diff_offset.to_le_bytes());
        raw[16..18].copy_from_slice(&kind.to_le_bytes());
        raw[18..20].copy_from_slice(&width.to_le_bytes());
        raw[20..22].copy_from_slice(&height.to_le_bytes());
        raw[22..24].copy_from_slice(&bpp.to_le_bytes());
        raw[24..28].copy_from_slice(&flags_offset.to_le_bytes());
        raw[28..32].copy_from_slice(&data_offset.to_le_bytes());

        for (i, byte) in raw[8..32].iter_mut().enumerate() {
            *byte = byte.wrapping_add(tail[4 + i]);
        }
        for chunk in raw[8..32].as_chunks_mut::<2>().0 {
            let xored = u16::from_le_bytes([chunk[0], chunk[1]]) ^ key;
            chunk.copy_from_slice(&xored.to_le_bytes());
        }
        raw
    }

    fn xor_base_block(planes: [&[u8]; 4]) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        let mut stream = Vec::new();
        for (p, plane) in planes.iter().enumerate() {
            let (flags, data) = lzss_literals(plane);
            let f_off = stream.len() as u32;
            stream.extend_from_slice(&flags);
            let d_off = stream.len() as u32;
            stream.extend_from_slice(&data);
            out[p * 8..p * 8 + 4].copy_from_slice(&f_off.to_le_bytes());
            out[p * 8 + 4..p * 8 + 8].copy_from_slice(&d_off.to_le_bytes());
        }
        out.extend_from_slice(&stream);
        out
    }

    fn minimal_jbp1_2x2() -> Vec<u8> {
        let dc = [0u8; 4];
        let ac = [0xffu8, 0xff, 0xff, 0x00];
        let mut b = b"JBP1".to_vec();
        for word in [
            44u32,
            0x58c0_0002,
            100,
            2 | (2 << 16),
            24,
            0,
            dc.len() as u32,
            ac.len() as u32,
        ] {
            b.extend(word.to_le_bytes());
        }
        b.extend([0u8; 8]);
        for _ in 0..32 {
            b.extend(1u32.to_le_bytes());
        }
        b.extend([1u8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        b.extend([1u8; 128]);
        b.extend(dc);
        b.extend(ac);
        b
    }

    #[test]
    fn decodes_type1_swizzled_planes_for_24_and_32_bpp() {
        let mut unpacked = Vec::new();
        for lane_base in [10u8, 40, 70, 200] {
            for idx in 0..18u8 {
                unpacked.push(lane_base + idx);
            }
        }
        let (flags32, data32) = lzss_literals(&unpacked);
        let mut body32 = flags32.clone();
        body32.extend_from_slice(&data32);
        let raw32 = build_pb2(1, 9, 2, 32, 1, 0, 32, 32 + flags32.len() as u32, &body32);
        let img32 = decode(&raw32).unwrap();
        assert_eq!((img32.width(), img32.height()), (9, 2));
        // Tile (0,0) covers x=0..8, y=0..2 (16 samples per plane); tile (1,0) covers x=8, y=0..2.
        assert_eq!(&img32.pixels()[..4], &[70, 40, 10, 200]);
        assert_eq!(&img32.pixels()[(8 * 4)..(8 * 4 + 4)], &[86, 56, 26, 216]);
        assert_eq!(
            &img32.pixels()[((9 + 8) * 4)..((9 + 8) * 4 + 4)],
            &[87, 57, 27, 217]
        );

        let (flags24, data24) = lzss_literals(&unpacked[..54]);
        let mut body24 = flags24.clone();
        body24.extend_from_slice(&data24);
        let raw24 = build_pb2(1, 9, 2, 24, 1, 0, 32, 32 + flags24.len() as u32, &body24);
        let img24 = decode(&raw24).unwrap();
        assert_eq!(&img24.pixels()[..4], &[70, 40, 10, 255]);
    }

    #[test]
    fn decodes_type2_blocked_solid_and_literal_tiles() {
        let mut flag_chunks = Vec::new();
        let mut flag_lens = Vec::new();
        let mut data_chunks = Vec::new();
        let mut data_lens = Vec::new();
        for c in 0..3u8 {
            let lits = [50 + c, 60 + c];
            let (lflags, ldata) = lzss_literals(&lits);
            let mut fc = Vec::new();
            fc.extend(1u32.to_le_bytes()); // mask_bytes = ceil(2/8)
            fc.extend(1u32.to_le_bytes()); // solid_bytes = 1 (tile 0 solid)
            fc.extend(2u32.to_le_bytes()); // literal_count = 2 (tile 1 literal 1x2)
            fc.push(0x80); // tile 0 solid, tile 1 literal
            fc.push(10 + c);
            fc.extend_from_slice(&lflags);
            flag_lens.push(fc.len() as u32);
            flag_chunks.extend_from_slice(&fc);
            data_lens.push(ldata.len() as u32);
            data_chunks.extend_from_slice(&ldata);
        }
        let mut body = Vec::new();
        for len in flag_lens {
            body.extend(len.to_le_bytes());
        }
        body.extend_from_slice(&flag_chunks);
        let data_offset = 32 + body.len() as u32;
        for len in data_lens {
            body.extend(len.to_le_bytes());
        }
        body.extend_from_slice(&data_chunks);

        let raw = build_pb2(2, 9, 2, 24, 1, 0, 32, data_offset, &body);
        let img = decode(&raw).unwrap();
        assert_eq!(&img.pixels()[..4], &[12, 11, 10, 255]);
        assert_eq!(&img.pixels()[8 * 4..8 * 4 + 4], &[52, 51, 50, 255]);
        assert_eq!(
            &img.pixels()[(9 + 8) * 4..(9 + 8) * 4 + 4],
            &[62, 61, 60, 255]
        );
    }

    #[test]
    fn decodes_type3_and_type4_jbp1_with_optional_alpha_rle() {
        let jbp = minimal_jbp1_2x2();
        let raw3 = build_pb2(3, 2, 2, 24, 1, 0, 0, 0, &jbp);
        let img3 = decode(&raw3).unwrap();
        assert_eq!(img3.pixels(), &[128, 128, 128, 255].repeat(4));

        let mut body4 = jbp.clone();
        let alpha_off = 32 + body4.len() as u32;
        body4.extend_from_slice(&[0x00, 1, 0xff, 3]);
        let raw4 = build_pb2(4, 2, 2, 32, 1, 0, alpha_off, 0, &body4);
        let img4 = decode(&raw4).unwrap();
        assert_eq!(
            img4.pixels(),
            &[
                128, 128, 128, 0, 128, 128, 128, 255, 128, 128, 128, 255, 128, 128, 128, 255,
            ]
        );
    }

    #[test]
    fn decodes_type5_xor_base_and_delta_frame() {
        let mut body = xor_base_block([&[0x10; 64], &[0x03; 64], &[0x05; 64], &[0x0f; 64]]);
        let diff_off = 32 + body.len() as u32;

        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes()); // mask_bytes
        patch.extend(0u32.to_le_bytes());
        patch.push(0x00); // tile 0 changed
        patch.extend_from_slice(&[1u8, 2, 3].repeat(64));
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk_size = 16 + pflags.len() + pdata.len();
        body.extend((chunk_size as u32).to_le_bytes());
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);

        let raw = build_pb2(5, 8, 8, 32, 2, diff_off, 0, 0, &body);
        let frame0 = decode(&raw).unwrap();
        // a = 15, r = 5^15 = 10, g = 3^10 = 9, b = 16^9 = 25
        assert_eq!(&frame0.pixels()[..4], &[10, 9, 25, 15]);
        let frame1 = decode_frame(&raw, 1).unwrap();
        assert_eq!(&frame1.pixels()[..4], &[3, 2, 1, 255]);
        assert!(decode_frame(&raw, 2).is_err());
    }

    #[test]
    fn decodes_type6_multipart_and_lists_casefolded_parts() {
        let xor = xor_base_block([&[0x10; 18], &[0x03; 18], &[0x05; 18], &[0x0f; 18]]);
        let chunk0_size = 4 + 8 + xor.len();
        let mut body = Vec::new();
        body.extend((chunk0_size as u32).to_le_bytes());
        body.extend_from_slice(b"Base\0\0\0\0");
        body.extend_from_slice(&xor);

        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes()); // mask_bytes = ceil(2/8)
        patch.extend(0u32.to_le_bytes());
        patch.push(0x80); // tile 0 preserved, tile 1 (x=8, y=0..2) replaced
        patch.extend_from_slice(&[100, 110, 120, 200, 101, 111, 121, 201]);
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk1_size = 4 + 8 + 16 + pflags.len() + pdata.len();
        body.extend((chunk1_size as u32).to_le_bytes());
        body.extend_from_slice(b"Face_A\0\0");
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);

        let raw = build_pb2(6, 9, 2, 32, 2, 0, 0, 0, &body);
        assert_eq!(
            part_names(&raw).unwrap(),
            vec![b"Base".to_vec(), b"Face_A".to_vec()]
        );

        let base = decode_part_with(&raw, Some(b"base"), DecodeThreads::default()).unwrap();
        assert_eq!(&base.pixels()[8 * 4..8 * 4 + 4], &[10, 9, 25, 15]);

        let part = decode_part_with(&raw, Some(b"face_a"), DecodeThreads::default()).unwrap();
        assert_eq!(&part.pixels()[..4], &[10, 9, 25, 15]);
        assert_eq!(&part.pixels()[8 * 4..8 * 4 + 4], &[120, 110, 100, 200]);
        assert_eq!(
            &part.pixels()[(9 + 8) * 4..(9 + 8) * 4 + 4],
            &[121, 111, 101, 201]
        );
        assert!(decode_part_with(&raw, Some(b"missing"), DecodeThreads::default()).is_err());
        assert!(decode_part_with(&raw, Some(b""), DecodeThreads::default()).is_err());
    }

    #[test]
    fn rejects_invalid_magic_bounds_and_codecs() {
        assert!(decode(&[0u8; 64]).is_err());
        assert!(decode(b"PB2A").is_err());
        assert!(decode(&build_pb2(9, 2, 2, 24, 1, 0, 0, 0, &[])).is_err());
        assert!(decode(&build_pb2(1, 0, 2, 24, 1, 0, 0, 0, &[])).is_err());
        assert!(decode(&build_pb2(1, 2, 2, 16, 1, 0, 0, 0, &[])).is_err());
    }
}

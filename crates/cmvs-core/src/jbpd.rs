//! Bounded CMV6 JBPD decoder. Evidence: documented cmvs64.exe hash,
//! 0x1400ab650 (tiles/entropy), 0x1400b51c0 (integer IDCT), 0x1400a2300 (color).
//! Always decodes all blocks, like the original non-consecutive-frame path.
use crate::{FormatError, MAX_PAYLOAD, raster::RgbaImage};

/// Zigzag scan; JBP1's 0x1400e4730 holds entries 1..64 of the same order.
pub(crate) const ORDER: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const ROW: [[i32; 32]; 4] = [
    [
        16384, 21407, 16384, 8867, 16384, -8867, 16384, -21407, 16384, 8867, -16384, -21407,
        -16384, 21407, 16384, -8867, 22725, 19266, 19266, -4520, 12873, -22725, 4520, -12873,
        12873, 4520, -22725, -12873, 4520, 19266, 19266, -22725,
    ],
    [
        22725, 29692, 22725, 12288, 22725, -12299, 22725, -29692, 22725, 12299, -22725, -29692,
        -22725, 29692, 22725, -12299, 31521, 26722, 26722, -6270, 17855, -31521, 6270, -17855,
        17855, 6270, -31521, -17855, 6270, 26722, 26722, -31521,
    ],
    [
        21407, 27969, 21407, 11585, 21407, -11585, 21407, -27969, 21407, 11585, -21407, -27969,
        -21407, 27969, 21407, -11585, 29692, 25172, 25172, -5906, 16819, -29692, 5906, -16819,
        16819, 5906, -29692, -16819, 5906, 25172, 25172, -29692,
    ],
    [
        19266, 25172, 19266, 10426, 19266, -10426, 19266, -25172, 19266, 10426, -19266, -25172,
        -19266, 25172, 19266, -10426, 26722, 22654, 22654, -5315, 15137, -26722, 5315, -15137,
        15137, 5315, -26722, -15137, 5315, 22654, 22654, -26722,
    ],
];
fn sat(x: i32) -> i16 {
    x.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
fn add(a: i16, b: i16) -> i16 {
    a.saturating_add(b)
}
fn sub(a: i16, b: i16) -> i16 {
    a.saturating_sub(b)
}
fn mul(a: i16, b: i16) -> i16 {
    ((i32::from(a) * i32::from(b)) >> 16) as i16
}
fn idct(block: &mut [i16; 64]) {
    for (r, &table) in [0, 1, 2, 3, 0, 3, 2, 1].iter().enumerate() {
        let x = &block[r * 8..r * 8 + 8];
        let c = &ROW[table];
        let mut row = [0i16; 8];
        for j in 0..4 {
            let e = 1024i32
                .wrapping_add(i32::from(x[0]) * c[2 * j])
                .wrapping_add(i32::from(x[2]) * c[2 * j + 1])
                .wrapping_add(i32::from(x[4]) * c[8 + 2 * j])
                .wrapping_add(i32::from(x[6]) * c[9 + 2 * j]);
            let o = (i32::from(x[1]) * c[16 + 2 * j])
                .wrapping_add(i32::from(x[3]) * c[17 + 2 * j])
                .wrapping_add(i32::from(x[5]) * c[24 + 2 * j])
                .wrapping_add(i32::from(x[7]) * c[25 + 2 * j]);
            row[j] = sat(e.wrapping_add(o) >> 11);
            row[7 - j] = sat(e.wrapping_sub(o) >> 11);
        }
        block[r * 8..r * 8 + 8].copy_from_slice(&row);
    }
    for c in 0..8 {
        let x: [i16; 8] = std::array::from_fn(|r| block[r * 8 + c]);
        let a = sub(mul(13036, x[1]), x[7]);
        let b = add(mul(x[7], 13036), x[1]);
        let d = add(add(mul(x[5], -21746), x[5]), x[3]);
        let e = sub(x[5], add(mul(-21746, x[3]), x[3]));
        let f = add(add(d, b), 1);
        let g = add(sub(a, e), 1);
        let h = add(a, e);
        let i = sub(b, d);
        let j = add(mul(x[6], 27146), x[2]);
        let k = sub(mul(27146, x[2]), x[6]);
        let l = add(x[4], x[0]);
        let m = sub(l, j);
        let n = add(add(i, g), mul(-19195, add(i, g))) | 1;
        let p = add(mul(-19195, sub(i, g)), sub(i, g)) | 1;
        let q = add(add(l, j), 32);
        let s = sub(x[0], x[4]);
        let t = add(sub(s, k), 31);
        let u = add(add(s, k), 32);
        let v = add(m, 31);
        let out = [
            add(f, q),
            add(u, n),
            add(t, p),
            add(h, v),
            sub(v, h),
            sub(t, p),
            sub(u, n),
            sub(q, f),
        ];
        for r in 0..8 {
            block[r * 8 + c] = out[r] >> 6;
        }
    }
}

/// Bits taken low bit first from each byte (the originals load 32-bit words).
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    pub(crate) fn bit(&mut self) -> Result<usize, FormatError> {
        let b = *self
            .data
            .get(self.at / 8)
            .ok_or(FormatError("JBP bitstream truncated"))?;
        let v = usize::from((b >> (self.at % 8)) & 1);
        self.at += 1;
        Ok(v)
    }
    pub(crate) fn signed(&mut self, n: usize) -> Result<i16, FormatError> {
        if n == 0 {
            return Ok(0);
        }
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.bit()? as i32;
        }
        if v < (1 << (n - 1)) {
            v -= (1 << n) - 1;
        }
        Ok(v as i16)
    }
}
/// Frequency tree shared by JBPD (0x1400a0960) and JBP1 (0x1400b3b40): repeatedly join the
/// two lowest weights, lower node index first, the first one on bit 0.
pub(crate) struct Tree {
    children: Vec<[usize; 2]>,
}
impl Tree {
    pub(crate) fn new(data: &[u8]) -> Result<Self, FormatError> {
        let mut weights: Vec<u32> = data
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut children = vec![[0, 0]; 16];
        let mut active = vec![true; 16];
        for _ in 0..15 {
            let mut ids: Vec<_> = (0..weights.len()).filter(|&i| active[i]).collect();
            ids.sort_by_key(|&i| (weights[i], i));
            let (a, b) = (ids[0], ids[1]);
            let sum = weights[a]
                .checked_add(weights[b])
                .filter(|&v| v < 2099999999)
                .ok_or(FormatError("JBP Huffman frequencies overflow"))?;
            active[a] = false;
            active[b] = false;
            weights.push(sum);
            active.push(true);
            children.push([a, b]);
        }
        Ok(Self { children })
    }
    pub(crate) fn symbol(&self, bits: &mut Bits<'_>) -> Result<usize, FormatError> {
        let mut node = 30;
        while node >= 16 {
            node = self.children[node][bits.bit()?];
        }
        Ok(node)
    }
}
pub(crate) fn word(data: &[u8], at: usize) -> Result<u32, FormatError> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4)
            .ok_or(FormatError("JBP header truncated"))?
            .try_into()
            .unwrap(),
    ))
}

/// Decode a self-contained CMV6 JBPD frame to top-down opaque RGBA.
/// Other JBP versions, subsampling modes, and missing quantization tables are rejected.
pub fn decode(data: &[u8]) -> Result<RgbaImage, FormatError> {
    decode_inner(data, None)
}

/// CMV9: the outer bitmap marks static blocks. Base data contains the set bits;
/// ordinary frames contain the clear bits. Frames decode independently over the base.
/// The base stream's table offset lacks the regular frame's four-byte tile mask.
pub(crate) fn decode_masked(
    data: &[u8],
    mask: &[u8],
    static_blocks: bool,
    base: Option<&RgbaImage>,
) -> Result<RgbaImage, FormatError> {
    decode_inner(data, Some((mask, static_blocks, base)))
}

fn decode_inner(
    data: &[u8],
    masked: Option<(&[u8], bool, Option<&RgbaImage>)>,
) -> Result<RgbaImage, FormatError> {
    if data.len() > MAX_PAYLOAD || data.get(..4) != Some(b"JBPD") {
        return Err(FormatError("expected bounded JBPD frame"));
    }
    let flags = word(data, 8)?;
    // Verified 4:2:0, explicit quantizers, CMV6 tile layout. Bit 12 is an optional reuse mask.
    if flags & !0x1000 != 0x58c00002 {
        return Err(FormatError("unsupported JBPD flags"));
    }
    let dimensions = word(data, 16)?;
    let w = (dimensions & 65535) as usize;
    let h = (dimensions >> 16) as usize;
    if w == 0
        || h == 0
        || w * h > MAX_PAYLOAD / 4
        || word(data, 20)? != 24
        || (masked.is_none() && (!w.is_multiple_of(16) || !h.is_multiple_of(16)))
    {
        return Err(FormatError("unsupported JBPD dimensions or bit depth"));
    }
    let table = usize::try_from(word(data, 4)?)
        .ok()
        .and_then(|n| {
            n.checked_add(
                if masked.is_some_and(|(_, static_blocks, _)| static_blocks) {
                    0
                } else {
                    4
                },
            )
        })
        .filter(|&n| n >= 44 && n <= data.len().saturating_sub(272))
        .ok_or(FormatError("JBPD table offset outside frame"))?;
    let head = data
        .get(table..table + 272)
        .ok_or(FormatError("JBPD tables truncated"))?;
    let dc_tree = Tree::new(&head[..64])?;
    let ac_tree = Tree::new(&head[64..128])?;
    let runs = &head[128..144];
    let quant = &head[144..272];
    let layout = table + 272;
    let layout_word = word(data, layout)?;
    let rows = (layout_word & 255) as usize;
    let cols = ((layout_word >> 8) & 255) as usize;
    let count = (layout_word >> 16) as usize;
    if rows != h.div_ceil(256) || cols != w.div_ceil(256) || count != rows * cols {
        return Err(FormatError("JBPD tile grid differs from dimensions"));
    }
    let tiles = data
        .get(layout + 4..layout + 4 + count * 2)
        .ok_or(FormatError("JBPD tiles truncated"))?;
    let mut start = layout + (4 + count * 2).next_multiple_of(4);
    if flags & 0x1000 != 0 {
        if masked.is_some() {
            let length = data
                .get(start..start + 2)
                .ok_or(FormatError("CMV9 reuse mask truncated"))?;
            start += 2 + usize::from(u16::from_le_bytes(length.try_into().unwrap()));
        } else {
            start += (w / 16 * h / 16).div_ceil(8).next_multiple_of(4);
        }
    }
    let split = start
        .checked_add(word(data, 28)? as usize)
        .ok_or(FormatError("JBPD stream offset overflow"))?;
    let end = split
        .checked_add(word(data, 32)? as usize)
        .ok_or(FormatError("JBPD stream size overflow"))?;
    let dc_data = data
        .get(start..split)
        .ok_or(FormatError("JBPD DC stream outside frame"))?;
    let ac_data = data
        .get(split..end)
        .ok_or(FormatError("JBPD AC stream outside frame"))?;
    let mut dc = Bits::new(dc_data);
    let mut ac = Bits::new(ac_data);
    let mcus = w.div_ceil(16) * h.div_ceil(16);
    if let Some((mask, _, base)) = masked
        && (mask.len() != mcus.div_ceil(8)
            || base.is_some_and(|b| b.width() as usize != w || b.height() as usize != h))
    {
        return Err(FormatError("CMV9 base mask or dimensions differ"));
    }
    let present = |index: usize| {
        masked.is_none_or(|(mask, set, _)| ((mask[index / 8] & (128 >> (index % 8))) != 0) == set)
    };
    let blocks = (0..mcus).filter(|&i| present(i)).count() * 6;
    if masked.is_some() && word(data, 36)? as usize != blocks {
        return Err(FormatError("CMV9 DC count differs from block mask"));
    }
    let mut values = Vec::with_capacity(blocks);
    let mut previous = 0i16;
    for _ in 0..blocks {
        let n = dc_tree.symbol(&mut dc)?;
        previous = previous.wrapping_add(dc.signed(n)?);
        values.push(previous);
    }
    let mut pixels = masked
        .and_then(|(_, _, base)| base)
        .map_or_else(|| vec![0u8; w * h * 4], |base| base.pixels().to_vec());
    let mut dc_index = 0;
    let mut mcu_index = 0;
    for tile in 0..count {
        let ty = tile / cols * 256;
        let tx = tile % cols * 256;
        let nr = usize::from(tiles[tile * 2]);
        let nc = usize::from(tiles[tile * 2 + 1]);
        if nr != (h - ty).min(256).div_ceil(16) || nc != (w - tx).min(256).div_ceil(16) {
            return Err(FormatError("JBPD tile extent differs from grid"));
        }
        for by in 0..nr {
            for bx in 0..nc {
                let draw = present(mcu_index);
                mcu_index += 1;
                if !draw {
                    continue;
                }
                let mut samples = [[0i16; 64]; 6];
                for (n, block) in samples.iter_mut().enumerate() {
                    let q = &quant[if n < 4 { 0..64 } else { 64..128 }];
                    block[0] = values[dc_index].wrapping_mul(i16::from(q[0]));
                    dc_index += 1;
                    let mut pos = 1;
                    while pos < 64 {
                        let symbol = ac_tree.symbol(&mut ac)?;
                        if symbol == 15 {
                            break;
                        }
                        if symbol == 0 {
                            let mut run = 0;
                            while ac.bit()? != 0 {
                                run += 1;
                                if run == 16 {
                                    return Err(FormatError(
                                        "JBPD zero run selector outside table",
                                    ));
                                }
                            }
                            pos += usize::from(runs[run]) + 1;
                            if pos > 64 {
                                return Err(FormatError("JBPD zero run outside block"));
                            }
                        } else {
                            block[ORDER[pos]] =
                                ac.signed(symbol)?.wrapping_mul(i16::from(q[ORDER[pos]]));
                            pos += 1;
                        }
                    }
                    idct(block);
                }
                for y in 0..16 {
                    for x in 0..16 {
                        if ty + by * 16 + y >= h || tx + bx * 16 + x >= w {
                            continue;
                        }
                        let l = i32::from(samples[y / 8 * 2 + x / 8][y % 8 * 8 + x % 8]);
                        let cb = i32::from(samples[4][y / 2 * 8 + x / 2]);
                        let cr = i32::from(samples[5][y / 2 * 8 + x / 2]);
                        let rgb = [
                            l + 128 + ((91888 * cr) >> 16),
                            l + 128 - ((22544 * cb) >> 16) - ((46784 * cr) >> 16),
                            l + 128 + ((116112 * cb) >> 16),
                        ];
                        let at = ((ty + by * 16 + y) * w + tx + bx * 16 + x) * 4;
                        for c in 0..3 {
                            pixels[at + c] = rgb[c].clamp(0, 255) as u8;
                        }
                        pixels[at + 3] = 255;
                    }
                }
            }
        }
    }
    RgbaImage::new(w as u32, h as u32, pixels)
}

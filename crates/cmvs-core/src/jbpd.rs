//! Bounded CMV6 JBPD and CMV8 JBPE decoder. Evidence: documented cmvs64.exe hash,
//! 0x1400ab650 / 0x1400af100 (tiles/entropy), 0x1400b51c0 (integer IDCT), 0x1400a2300
//! (color), 0x1400a3850 (alpha).
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
/// Row tables of 0x1400b51c0, by row.
const ROW_TABLE: [usize; 8] = [0, 1, 2, 3, 0, 3, 2, 1];

#[inline(always)]
fn idct_row(x: &[i16], c: &[i32; 32]) -> [i16; 8] {
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
    row
}

#[inline(always)]
fn idct_column(x: [i16; 8]) -> [i16; 8] {
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
    [
        add(f, q) >> 6,
        add(u, n) >> 6,
        add(t, p) >> 6,
        add(h, v) >> 6,
        sub(v, h) >> 6,
        sub(t, p) >> 6,
        sub(u, n) >> 6,
        sub(q, f) >> 6,
    ]
}

/// 0x1400b51c0. `rows` has bit r set for every row that may hold a nonzero coefficient
/// (a superset is fine); a zero row's pass yields zeros, so it is skipped. `dc_only`
/// means only `block[0]` may be nonzero, which makes every column equal.
fn idct(block: &mut [i16; 64], rows: u8, dc_only: bool) {
    if dc_only {
        // Row 0 with only x[0]: every output is (1024 + x0 * 16384) >> 11 (table 0).
        let r0 = sat(1024i32.wrapping_add(i32::from(block[0]) * ROW[0][0]) >> 11);
        let column = idct_column([r0, 0, 0, 0, 0, 0, 0, 0]);
        for (r, &value) in column.iter().enumerate() {
            block[r * 8..r * 8 + 8].fill(value);
        }
        return;
    }
    for r in 0..8 {
        if rows & (1 << r) != 0 {
            let row = idct_row(&block[r * 8..r * 8 + 8], &ROW[ROW_TABLE[r]]);
            block[r * 8..r * 8 + 8].copy_from_slice(&row);
        }
    }
    if rows == 1 {
        for c in 0..8 {
            let out = idct_column([block[c], 0, 0, 0, 0, 0, 0, 0]);
            for r in 0..8 {
                block[r * 8 + c] = out[r];
            }
        }
        return;
    }
    for c in 0..8 {
        let out = idct_column(std::array::from_fn(|r| block[r * 8 + c]));
        for r in 0..8 {
            block[r * 8 + c] = out[r];
        }
    }
}

/// Bits taken low bit first from each byte (the originals load 32-bit words), refilled
/// 64 at a time.
/// Reads past the end see zero bits and are reported by [`Bits::check`], which every
/// caller runs before using what it read, so truncation fails where reading bit by bit would.
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Next byte to load.
    next: usize,
    buffer: u64,
    /// Valid bits in `buffer`, low bits first.
    count: u32,
    /// Bits consumed past the end of `data`.
    consumed: usize,
    total: usize,
}
impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            next: 0,
            buffer: 0,
            count: 0,
            consumed: 0,
            total: data.len() * 8,
        }
    }
    /// At least 57 bits are valid afterwards.
    #[inline(always)]
    fn refill(&mut self) {
        if self.count > 56 {
            return;
        }
        if let Some(bytes) = self.data.get(self.next..self.next + 8) {
            let word = u64::from_le_bytes(bytes.try_into().unwrap());
            self.buffer |= word << self.count;
            // Bits of a partly counted byte are loaded again at the same position later.
            let taken = (64 - self.count) / 8;
            self.next += taken as usize;
            self.count += taken * 8;
        } else {
            while self.count <= 56 {
                let byte = self.data.get(self.next).copied().unwrap_or(0);
                self.buffer |= u64::from(byte) << self.count;
                self.next += 1;
                self.count += 8;
            }
        }
    }
    #[inline(always)]
    fn peek(&self) -> u64 {
        self.buffer
    }
    #[inline(always)]
    fn consume(&mut self, n: u32) {
        self.buffer >>= n;
        self.count -= n;
        self.consumed += n as usize;
    }
    #[inline(always)]
    fn check(&self) -> Result<(), FormatError> {
        if self.consumed > self.total {
            Err(FormatError("JBP bitstream truncated"))
        } else {
            Ok(())
        }
    }
    /// The zero-run selector: ones ended by a zero. `None` for sixteen ones, which no
    /// table entry follows.
    #[inline(always)]
    pub(crate) fn run_selector(&mut self) -> Result<Option<usize>, FormatError> {
        self.refill();
        let ones = self.peek().trailing_ones().min(16);
        self.consume(if ones == 16 { 16 } else { ones + 1 });
        self.check()?;
        Ok((ones < 16).then_some(ones as usize))
    }
    /// `n` bits, the first one most significant, at most 16; values below half of the
    /// range are negative.
    #[inline(always)]
    pub(crate) fn signed(&mut self, n: usize) -> Result<i16, FormatError> {
        if n == 0 {
            return Ok(0);
        }
        self.refill();
        let raw = (self.peek() as u32) & ((1u32 << n) - 1);
        self.consume(n as u32);
        self.check()?;
        let mut v = (raw.reverse_bits() >> (32 - n)) as i32;
        if v < (1 << (n - 1)) {
            v -= (1 << n) - 1;
        }
        Ok(v as i16)
    }
}

const LOOKUP_BITS: u32 = 10;
/// [`Tree`] decoding (from node 30, bit 0 to the first child) through a table of the next [`LOOKUP_BITS`] bits; longer codes walk
/// the tree from the node the table reaches.
pub(crate) struct Huffman {
    children: Vec<[usize; 2]>,
    /// Leaf: symbol | length << 8. Otherwise the internal node after LOOKUP_BITS bits,
    /// with bit 15 set.
    table: Vec<u16>,
}
impl Huffman {
    pub(crate) fn new(tree: Tree) -> Self {
        let mut table = vec![0u16; 1 << LOOKUP_BITS];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut node = 30;
            let mut depth = 0;
            while node >= 16 && depth < LOOKUP_BITS {
                node = tree.children[node][(index >> depth) & 1];
                depth += 1;
            }
            *entry = if node < 16 {
                node as u16 | (depth as u16) << 8
            } else {
                node as u16 | 0x8000
            };
        }
        Self {
            children: tree.children,
            table,
        }
    }
    #[inline(always)]
    pub(crate) fn symbol(&self, bits: &mut Bits<'_>) -> Result<usize, FormatError> {
        bits.refill();
        let entry = self.table[(bits.peek() & ((1 << LOOKUP_BITS) - 1)) as usize];
        if entry & 0x8000 == 0 {
            bits.consume(u32::from(entry >> 8));
            bits.check()?;
            return Ok(usize::from(entry & 255));
        }
        bits.consume(LOOKUP_BITS);
        let mut node = usize::from(entry & 0x7fff);
        // A tree over 16 leaves is at most 15 deep, within one refill.
        while node >= 16 {
            node = self.children[node][(bits.peek() & 1) as usize];
            bits.consume(1);
        }
        bits.check()?;
        Ok(node)
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
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
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
    decode_inner(data, None, false)
}

/// Decode a self-contained CMV8 `JBPE` frame (0x1400af100) to top-down RGBA with alpha.
/// After the six colour blocks each MCU carries four alpha blocks laid out like luma and
/// dequantized with a third table. The DC count is stored after the three tables; the
/// optional reuse mask covers the MCU grid rounded up to 16 pixels. The original does not
/// check the signature; only the observed `JBPE` is accepted here.
pub fn decode_alpha(data: &[u8]) -> Result<RgbaImage, FormatError> {
    decode_inner(data, None, true)
}

/// 0x1400a3850 writes byte 0 of `0x1400e3b30[4 * v + 512]` for each alpha IDCT sample:
/// clamp(v, 0, 255) for v in -128..=383. Other entries hold unrelated values and samples
/// below -128 read before the table, so they are rejected.
fn alpha_sample(v: i16) -> Result<u8, FormatError> {
    match v {
        -128..=383 => Ok(v.clamp(0, 255) as u8),
        _ => Err(FormatError(
            "JBPE alpha sample outside the verified clamp table",
        )),
    }
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
    decode_inner(data, Some((mask, static_blocks, base)), false)
}

fn decode_inner(
    data: &[u8],
    masked: Option<(&[u8], bool, Option<&RgbaImage>)>,
    alpha: bool,
) -> Result<RgbaImage, FormatError> {
    let signature: &[u8] = if alpha { b"JBPE" } else { b"JBPD" };
    if data.len() > MAX_PAYLOAD || data.get(..4) != Some(signature) {
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
    if w == 0 || h == 0 || w * h > MAX_PAYLOAD / 4 || word(data, 20)? != if alpha { 32 } else { 24 }
    {
        return Err(FormatError("unsupported JBPD dimensions or bit depth"));
    }
    // JBPE has a third (alpha) quantization table.
    let head_size = if alpha { 336 } else { 272 };
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
        .filter(|&n| n >= 44 && n <= data.len().saturating_sub(head_size))
        .ok_or(FormatError("JBPD table offset outside frame"))?;
    let head = data
        .get(table..table + head_size)
        .ok_or(FormatError("JBPD tables truncated"))?;
    let dc_tree = Huffman::new(Tree::new(&head[..64])?);
    let ac_tree = Huffman::new(Tree::new(&head[64..128])?);
    let runs = &head[128..144];
    let quant = &head[144..head_size];
    let mcus = w.div_ceil(16) * h.div_ceil(16);
    let per_mcu = if alpha { 10 } else { 6 };
    let mut layout = table + head_size;
    if alpha {
        if word(data, layout)? as usize != mcus * per_mcu {
            return Err(FormatError("JBPE DC count differs from the MCU grid"));
        }
        layout += 4;
    }
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
            // 0x1400a0ad0 rounds both dimensions up to 16 for 4:2:0; 0x1400ab650 (JBPD) and
            // 0x1400af100 (JBPE) size the mask from that MCU grid (hot_n_mov.cmv: 640x360).
            start += mcus.div_ceil(8).next_multiple_of(4);
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
    if let Some((mask, _, base)) = masked
        && (mask.len() != mcus.div_ceil(8)
            || base.is_some_and(|b| b.width() as usize != w || b.height() as usize != h))
    {
        return Err(FormatError("CMV9 base mask or dimensions differ"));
    }
    let present = |index: usize| {
        masked.is_none_or(|(mask, set, _)| ((mask[index / 8] & (128 >> (index % 8))) != 0) == set)
    };
    let blocks = (0..mcus).filter(|&i| present(i)).count() * per_mcu;
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
                // Luma 0..3, Cb 4, Cr 5, then JBPE alpha 6..9 with the third table.
                let mut samples = [[0i16; 64]; 10];
                for (n, block) in samples[..per_mcu].iter_mut().enumerate() {
                    let q = &quant[match n {
                        0..4 => 0..64,
                        4 | 5 => 64..128,
                        _ => 128..192,
                    }];
                    block[0] = values[dc_index].wrapping_mul(i16::from(q[0]));
                    dc_index += 1;
                    let mut rows = 1u8;
                    let mut pos = 1;
                    while pos < 64 {
                        let symbol = ac_tree.symbol(&mut ac)?;
                        if symbol == 15 {
                            break;
                        }
                        if symbol == 0 {
                            let Some(run) = ac.run_selector()? else {
                                return Err(FormatError("JBPD zero run selector outside table"));
                            };
                            pos += usize::from(runs[run]) + 1;
                            if pos > 64 {
                                return Err(FormatError("JBPD zero run outside block"));
                            }
                        } else {
                            let at = ORDER[pos];
                            block[at] = ac.signed(symbol)?.wrapping_mul(i16::from(q[at]));
                            rows |= 1 << (at / 8);
                            pos += 1;
                        }
                    }
                    idct(
                        block,
                        rows,
                        rows == 1 && block[1..8].iter().all(|&v| v == 0),
                    );
                }
                let (y0, x0) = (ty + by * 16, tx + bx * 16);
                let (bh, bw) = ((h - y0).min(16), (w - x0).min(16));
                for y in 0..bh {
                    // Fixed-size rows let the conversion vectorize without bounds checks.
                    let mut luma = [0i16; 16];
                    luma[..8].copy_from_slice(&samples[y / 8 * 2][y % 8 * 8..][..8]);
                    luma[8..].copy_from_slice(&samples[y / 8 * 2 + 1][y % 8 * 8..][..8]);
                    let cb: &[i16; 8] = samples[4][y / 2 * 8..][..8].try_into().unwrap();
                    let cr: &[i16; 8] = samples[5][y / 2 * 8..][..8].try_into().unwrap();
                    let mut cb_wide = [0i32; 16];
                    let mut cr_wide = [0i32; 16];
                    for x in 0..16 {
                        cb_wide[x] = i32::from(cb[x / 2]);
                        cr_wide[x] = i32::from(cr[x / 2]);
                    }
                    let mut words = [0u32; 16];
                    for x in 0..16 {
                        let l = i32::from(luma[x]) + 128;
                        let (cb, cr) = (cb_wide[x], cr_wide[x]);
                        let r = (l + ((91888 * cr) >> 16)).clamp(0, 255) as u32;
                        let g =
                            (l - ((22544 * cb) >> 16) - ((46784 * cr) >> 16)).clamp(0, 255) as u32;
                        let b = (l + ((116112 * cb) >> 16)).clamp(0, 255) as u32;
                        words[x] = r | g << 8 | b << 16 | 0xff00_0000;
                    }
                    let mut out = [0u8; 64];
                    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
                        *bytes = word.to_le_bytes();
                    }
                    if alpha {
                        for x in 0..bw {
                            out[x * 4 + 3] =
                                alpha_sample(samples[6 + y / 8 * 2 + x / 8][y % 8 * 8 + x % 8])?;
                        }
                    }
                    let at = ((y0 + y) * w + x0) * 4;
                    pixels[at..at + bw * 4].copy_from_slice(&out[..bw * 4]);
                }
            }
        }
    }
    RgbaImage::new(w as u32, h as u32, pixels)
}

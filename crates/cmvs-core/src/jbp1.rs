//! Bounded JBP1 decoder for PB3B type 3; addresses refer to the documented cmvs64.exe hash.
//! 0x1400a0ad0 reads the header, 0x1400b4020 the trees, streams and MCU layout,
//! 0x1400b3c80 dequantizes and runs the scalar integer IDCT, and the 768-byte table at
//! 0x1400e47b0 clamps the color conversion. Entropy coding is shared with JBPD.
use crate::{
    FormatError, MAX_PAYLOAD,
    jbpd::{Bits, ORDER, Tree, word},
};

/// The only flags met in this game: 4:2:0 (bits 28-29 = 1), explicit quantizers (bit 27).
const FLAGS: u32 = 0x58c0_0002;

/// Decoded JBP1 samples before the PB3 layer crops and adds alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jbp1 {
    /// Header +16/+18.
    pub width: u16,
    pub height: u16,
    /// Both rounded up to 16, as 0x1400a0ad0 does for mode 1; `bgr` uses this stride.
    pub padded_width: usize,
    pub padded_height: usize,
    /// Blue, green and red per padded pixel, top-down.
    pub bgr: Vec<u8>,
}

/// `(c * v) >> 16` with the 32-bit wrapping multiply and arithmetic shift of the original.
fn scale(c: i32, v: i32) -> i32 {
    c.wrapping_mul(v) >> 16
}

/// The odd half shared by both IDCT passes, from inputs 1, 3, 5, 7.
fn odd(x1: i32, x3: i32, x5: i32, x7: i32) -> [i32; 4] {
    let a = scale(77062, x5.wrapping_add(x1).wrapping_add(x7).wrapping_add(x3));
    let b = a.wrapping_add(scale(-25570, x5.wrapping_add(x1)));
    let c = a.wrapping_add(scale(-128553, x7.wrapping_add(x3)));
    let d = scale(-167963, x5.wrapping_add(x3));
    let e = scale(-58980, x7.wrapping_add(x1));
    [
        e.wrapping_add(b).wrapping_add(scale(98390, x1)),
        d.wrapping_add(c).wrapping_add(scale(201373, x3)),
        d.wrapping_add(b).wrapping_add(scale(134553, x5)),
        c.wrapping_add(e).wrapping_add(scale(19571, x7)),
    ]
}

/// The even half: [x0+x4 ± t2, x0-x4 ± t6] from inputs 0, 2, 4, 6.
fn even(x0: i32, x2: i32, x4: i32, x6: i32) -> [i32; 4] {
    let z = scale(35467, x2.wrapping_add(x6));
    let t2 = z.wrapping_add(scale(50159, x2));
    let t6 = z.wrapping_add(scale(-121094, x6));
    let sum = x0.wrapping_add(x4);
    let diff = x0.wrapping_sub(x4);
    [
        sum.wrapping_add(t2),
        diff.wrapping_add(t6),
        diff.wrapping_sub(t6),
        sum.wrapping_sub(t2),
    ]
}

/// 0x1400b3c80: columns first, multiplying each coefficient by the natural-order quantizer
/// into 32 bits (a column whose rows 1..7 are zero is filled with the 16-bit product of row
/// 0); rows second, shifting right by 3. Every intermediate is 32-bit wrapping and only the
/// stored results are truncated to 16 bits. No rounding or level shift happens here.
fn idct(block: &mut [i16; 64], quant: &[u8]) {
    for c in 0..8 {
        if (1..8).all(|r| block[r * 8 + c] == 0) {
            let value = i32::from(block[c]).wrapping_mul(i32::from(quant[c])) as i16;
            for r in 0..8 {
                block[r * 8 + c] = value;
            }
            continue;
        }
        let x: [i32; 8] = std::array::from_fn(|r| {
            i32::from(block[r * 8 + c]).wrapping_mul(i32::from(quant[r * 8 + c]))
        });
        let [e0, e1, e2, e3] = even(x[0], x[2], x[4], x[6]);
        let [o0, o1, o2, o3] = odd(x[1], x[3], x[5], x[7]);
        let out = [
            e0.wrapping_add(o0),
            e1.wrapping_add(o1),
            e2.wrapping_add(o2),
            e3.wrapping_add(o3),
            e3.wrapping_sub(o3),
            e2.wrapping_sub(o2),
            e1.wrapping_sub(o1),
            e0.wrapping_sub(o0),
        ];
        for (r, value) in out.into_iter().enumerate() {
            block[r * 8 + c] = value as i16;
        }
    }
    for r in 0..8 {
        let x: [i32; 8] = std::array::from_fn(|c| i32::from(block[r * 8 + c]));
        let [e0, e1, e2, e3] = even(x[0], x[2], x[4], x[6]);
        let [o0, o1, o2, o3] = odd(x[1], x[3], x[5], x[7]);
        let out = [
            e0.wrapping_add(o0),
            e1.wrapping_add(o1),
            e2.wrapping_add(o2),
            e3.wrapping_add(o3),
            e3.wrapping_sub(o3),
            e2.wrapping_sub(o2),
            e1.wrapping_sub(o1),
            e0.wrapping_sub(o0),
        ];
        for (c, value) in out.into_iter().enumerate() {
            block[r * 8 + c] = (value >> 3) as i16;
        }
    }
}

/// The 0x1400e47b0 lookup: index 0..256 gives 0, 256..512 gives index - 256, 512..768 gives
/// 255. Indices are the luma plus 384 plus a chroma term; outside the table the original
/// reads neighbouring data, which is rejected here.
fn clamp(index: i64) -> Result<u8, FormatError> {
    match index {
        0..256 => Ok(0),
        256..512 => Ok((index - 256) as u8),
        512..768 => Ok(255),
        _ => Err(FormatError("JBP1 sample outside the original clamp table")),
    }
}

/// Decode a JBP1 stream (PB3B type 3 data at file offset 52) to padded BGR samples.
pub fn decode(data: &[u8]) -> Result<Jbp1, FormatError> {
    if data.len() > MAX_PAYLOAD || data.get(..4) != Some(b"JBP1") {
        return Err(FormatError("expected bounded JBP1 stream"));
    }
    if word(data, 8)? != FLAGS {
        return Err(FormatError("unsupported JBP1 flags"));
    }
    let dimensions = word(data, 16)?;
    let (width, height) = (dimensions as u16, (dimensions >> 16) as u16);
    let padded_width = usize::from(width).next_multiple_of(16);
    let padded_height = usize::from(height).next_multiple_of(16);
    if width == 0
        || height == 0
        || padded_width * padded_height > MAX_PAYLOAD / 4
        || word(data, 20)? != 24
    {
        return Err(FormatError("unsupported JBP1 dimensions or bit depth"));
    }
    let table = usize::try_from(word(data, 4)?)
        .ok()
        .filter(|&n| n >= 36 && n <= data.len().saturating_sub(272))
        .ok_or(FormatError("JBP1 table offset outside stream"))?;
    let head = &data[table..table + 272];
    let dc_tree = Tree::new(&head[..64])?;
    let ac_tree = Tree::new(&head[64..128])?;
    let runs = &head[128..144];
    let quant = &head[144..272];
    let start = table + 272;
    let split = start
        .checked_add(word(data, 28)? as usize)
        .ok_or(FormatError("JBP1 stream offset overflow"))?;
    let end = split
        .checked_add(word(data, 32)? as usize)
        .ok_or(FormatError("JBP1 stream size overflow"))?;
    let mut dc = Bits::new(
        data.get(start..split)
            .ok_or(FormatError("JBP1 DC stream outside data"))?,
    );
    let mut ac = Bits::new(
        data.get(split..end)
            .ok_or(FormatError("JBP1 AC stream outside data"))?,
    );
    let (columns, rows) = (padded_width / 16, padded_height / 16);
    // All DC differences come first and accumulate over the whole image (kept as 16 bits).
    let mut values = Vec::with_capacity(columns * rows * 6);
    let mut previous = 0i16;
    for _ in 0..columns * rows * 6 {
        let n = dc_tree.symbol(&mut dc)?;
        previous = previous.wrapping_add(dc.signed(n)?);
        values.push(previous);
    }
    let mut bgr = vec![0u8; padded_width * padded_height * 3];
    let mut dc_index = 0;
    for my in 0..rows {
        for mx in 0..columns {
            let mut samples = [[0i16; 64]; 6];
            for (n, block) in samples.iter_mut().enumerate() {
                block[0] = values[dc_index];
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
                                return Err(FormatError("JBP1 zero run selector outside table"));
                            }
                        }
                        pos += usize::from(runs[run]) + 1;
                        if pos > 64 {
                            return Err(FormatError("JBP1 zero run outside block"));
                        }
                    } else {
                        block[ORDER[pos]] = ac.signed(symbol)?;
                        pos += 1;
                    }
                }
                idct(block, &quant[if n < 4 { 0..64 } else { 64..128 }]);
            }
            for y in 0..16 {
                for x in 0..16 {
                    let luma =
                        i64::from(i32::from(samples[y / 8 * 2 + x / 8][y % 8 * 8 + x % 8]) + 384);
                    let cb = i32::from(samples[4][y / 2 * 8 + x / 2]);
                    let cr = i32::from(samples[5][y / 2 * 8 + x / 2]);
                    let green = scale(46784, cr).wrapping_add(scale(22544, cb));
                    let at = ((my * 16 + y) * padded_width + mx * 16 + x) * 3;
                    bgr[at] = clamp(luma + i64::from(scale(116112, cb)))?;
                    bgr[at + 1] = clamp(luma - i64::from(green))?;
                    bgr[at + 2] = clamp(luma + i64::from(scale(91888, cr)))?;
                }
            }
        }
    }
    Ok(Jbp1 {
        width,
        height,
        padded_width,
        padded_height,
        bgr,
    })
}

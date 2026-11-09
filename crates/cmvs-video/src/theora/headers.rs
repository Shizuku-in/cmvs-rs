use super::bits::{Bits, Huffman, Result};
use cmvs_core::{FormatError, MAX_PAYLOAD};

#[derive(Clone)]
pub(crate) struct Ident {
    pub width: usize,
    pub height: usize,
    pub picw: u32,
    pub pich: u32,
    pub picx: u8,
    pub picy: u8,
    pub frn: u32,
    pub frd: u32,
}

fn body(packet: &[u8], kind: u8) -> Result<&[u8]> {
    if packet.first() != Some(&kind) || packet.get(1..7) != Some(b"theora") {
        return Err(FormatError("invalid Theora header signature"));
    }
    Ok(&packet[7..])
}

impl Ident {
    pub fn parse(packet: &[u8]) -> Result<Self> {
        let mut b = Bits::new(body(packet, 0x80)?);
        if b.read(8)? != 3 || b.read(8)? != 2 {
            return Err(FormatError("unsupported Theora version (expected 3.2)"));
        }
        b.skip(8)?; // Revision is forward compatible.
        let width = b.read(16)? as usize * 16;
        let height = b.read(16)? as usize * 16;
        let picw = b.read(24)?;
        let pich = b.read(24)?;
        let picx = b.read(8)? as u8;
        let picy = b.read(8)? as u8;
        let frn = b.read(32)?;
        let frd = b.read(32)?;
        b.skip(24)?; // Pixel aspect ratio, informational.
        b.skip(24)?;
        b.skip(8)?; // Colour space; the host retains its BT.601 conversion.
        b.skip(24)?; // Nominal bitrate.
        b.skip(11)?; // Quality and keyframe granule shift.
        if b.read(2)? != 0 || b.read(3)? != 0 {
            return Err(FormatError("Theora requires YUV420 and zero reserved bits"));
        }
        if width == 0
            || height == 0
            || frn == 0
            || frd == 0
            || picw == 0
            || pich == 0
            || u64::from(picw) + u64::from(picx) > width as u64
            || u64::from(pich) + u64::from(picy) > height as u64
            || width as u64 * height as u64 * 4 > MAX_PAYLOAD as u64
        {
            return Err(FormatError("invalid or oversized Theora geometry/rate"));
        }
        Ok(Self {
            width,
            height,
            picw,
            pich,
            picx,
            picy,
            frn,
            frd,
        })
    }
}

pub(crate) fn comments(packet: &[u8]) -> Result<()> {
    let mut rest = body(packet, 0x81)?;
    fn word(rest: &mut &[u8]) -> Result<usize> {
        let bytes = rest
            .get(..4)
            .ok_or(FormatError("truncated Theora comment"))?;
        let value = u32::from_le_bytes(bytes.try_into().unwrap()) as usize;
        *rest = &rest[4..];
        Ok(value)
    }
    fn string(rest: &mut &[u8]) -> Result<()> {
        let len = word(rest)?;
        *rest = rest
            .get(len..)
            .ok_or(FormatError("truncated Theora comment string"))?;
        Ok(())
    }
    string(&mut rest)?;
    let count = word(&mut rest)?;
    if count > rest.len() / 4 {
        return Err(FormatError("oversized Theora comment count"));
    }
    for _ in 0..count {
        string(&mut rest)?;
    }
    Ok(())
}

pub(super) struct Setup {
    pub limits: [u8; 64],
    // Quantization tables are computed once, rather than twice for each 8x8 block.
    pub quant: Vec<[[u16; 64]; 64]>, // [type * 3 + plane][qi][coefficient]
    pub huffman: Vec<Huffman>,
}

impl Setup {
    pub fn parse(packet: &[u8]) -> Result<Self> {
        let mut b = Bits::new(body(packet, 0x82)?);
        let n = b.read(3)?;
        let mut limits = [0; 64];
        for v in &mut limits {
            *v = b.read(n)? as u8;
        }
        let mut scales = [[0u32; 64]; 2]; // AC, DC
        for scale in &mut scales {
            let n = b.read(4)? + 1;
            for v in scale {
                *v = b.read(n)?;
            }
        }
        let count = b.read(9)? as usize + 1;
        if count > 384 {
            return Err(FormatError("too many Theora base matrices"));
        }
        let mut base = vec![[0u32; 64]; count];
        for matrix in &mut base {
            for v in matrix {
                *v = b.read(8)?;
            }
        }
        let index_bits = usize::BITS - (count - 1).leading_zeros();
        let mut ranges: Vec<Vec<(usize, usize)>> = Vec::with_capacity(6);
        for table in 0..6 {
            if table != 0 && b.read(1)? == 0 {
                let source = if table >= 3 && b.read(1)? != 0 {
                    table - 3
                } else {
                    table - 1
                };
                ranges.push(ranges[source].clone());
                continue;
            }
            let first = b.read(index_bits)? as usize;
            if first >= count {
                return Err(FormatError("invalid Theora base matrix index"));
            }
            let mut range = vec![(0, first)];
            let mut qi = 0usize;
            while qi < 63 {
                let n = usize::BITS - (62usize.saturating_sub(qi)).leading_zeros();
                qi += b.read(n)? as usize + 1;
                let index = b.read(index_bits)? as usize;
                if qi > 63 || index >= count {
                    return Err(FormatError("invalid Theora quantization range"));
                }
                range.push((qi, index));
            }
            ranges.push(range);
        }
        let mut quant = vec![[[0u16; 64]; 64]; 6];
        for (table, range) in ranges.iter().enumerate() {
            for qi in 0..64 {
                let pair = range.windows(2).find(|p| qi <= p[1].0).unwrap();
                let [(lo, a), (hi, z)] = [pair[0], pair[1]];
                for ci in 0..64 {
                    let value = (2 * (hi - qi) as u32 * base[a][ci]
                        + 2 * (qi - lo) as u32 * base[z][ci]
                        + (hi - lo) as u32)
                        / (2 * (hi - lo) as u32);
                    let minimum = (if ci == 0 { 16 } else { 8 }) << (table / 3);
                    quant[table][qi][ci] = ((value * scales[usize::from(ci == 0)][qi] / 100 * 4)
                        .clamp(minimum, 4096)) as u16;
                }
            }
        }
        let mut huffman = Vec::with_capacity(80);
        for _ in 0..80 {
            let mut table = Huffman::parse(&mut b)?;
            table.prepare(&super::packet::TOKEN_EXTRA_BITS);
            huffman.push(table);
        }
        Ok(Self {
            limits,
            quant,
            huffman,
        })
    }
}

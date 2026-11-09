use super::{
    bits::{Bits, Result},
    geometry::Geometry,
    headers::Setup,
};
use cmvs_core::FormatError;

pub(super) struct State {
    pub coded: Vec<u8>,
    pub modes: Vec<u8>,
    pub vectors: Vec<[i16; 2]>,
    pub qi: Vec<usize>,
    pub coeffs: Vec<[i16; 64]>,
    pub ncoeffs: Vec<u8>,
    pub coded_blocks: Vec<usize>,
    positions: Vec<u8>,
    active: Vec<usize>,
    partial: Vec<u8>,
    full: Vec<u8>,
    runs: Vec<u8>,
}

/// Tables 7.7 / 7.11. All run writes are bounded before filling the scratch slice.
fn runs(b: &mut Bits<'_>, out: &mut [u8], long: bool) -> Result<()> {
    if out.is_empty() {
        return Ok(());
    }
    let mut value = b.read(1)? as u8;
    let mut position = 0;
    loop {
        let code = unary(b, if long { 6 } else { 5 })?;
        let (base, extra) = if long {
            [(1, 0), (2, 1), (4, 1), (6, 2), (10, 3), (18, 4), (34, 12)][code]
        } else {
            [(1, 1), (3, 1), (5, 1), (7, 2), (11, 2), (15, 4)][code]
        };
        let length = base + b.read(extra)? as usize;
        let end = position + length;
        out.get_mut(position..end)
            .ok_or(FormatError("Theora run exceeds block count"))?
            .fill(value);
        position = end;
        if position == out.len() {
            return Ok(());
        }
        value = if long && length == 4129 {
            b.read(1)? as u8
        } else {
            value ^ 1
        };
    }
}

fn unary(b: &mut Bits<'_>, maximum: usize) -> Result<usize> {
    for i in 0..maximum {
        if b.read(1)? == 0 {
            return Ok(i);
        }
    }
    Ok(maximum)
}

fn component(b: &mut Bits<'_>, fixed: bool) -> Result<i16> {
    let (magnitude, sign) = if fixed {
        (b.read(5)? as i16, b.read(1)?)
    } else {
        match b.read(3)? {
            0 => return Ok(0),
            1 => return Ok(1),
            2 => return Ok(-1),
            3 => (2, b.read(1)?),
            4 => (3, b.read(1)?),
            prefix => {
                let extra = b.read(prefix - 2)?;
                ((1 << (prefix - 3)) + (extra >> 1) as i16, extra & 1)
            }
        }
    };
    Ok(if sign == 0 { magnitude } else { -magnitude })
}

fn vector(b: &mut Bits<'_>, fixed: bool) -> Result<[i16; 2]> {
    Ok([component(b, fixed)?, component(b, fixed)?])
}

pub(super) fn reference(mode: u8) -> usize {
    match mode {
        1 => 0,
        5 | 6 => 2,
        _ => 1,
    }
}

impl State {
    pub fn new(g: &Geometry) -> Self {
        let n = g.blocks.len();
        Self {
            coded: vec![0; n],
            modes: vec![0; g.macroblocks.len()],
            vectors: vec![[0; 2]; n],
            qi: vec![0; n],
            coeffs: vec![[0; 64]; n],
            ncoeffs: vec![0; n],
            coded_blocks: Vec::with_capacity(n),
            positions: vec![0; n],
            active: Vec::with_capacity(n),
            partial: vec![0; g.superblocks],
            full: vec![0; g.superblocks],
            runs: vec![0; n.max(g.superblocks)],
        }
    }

    pub fn decode(
        &mut self,
        b: &mut Bits<'_>,
        intra: bool,
        nqis: usize,
        g: &Geometry,
        setup: &Setup,
    ) -> Result<()> {
        self.coded.fill(u8::from(intra));
        self.modes.fill(u8::from(intra));
        self.vectors.fill([0; 2]);
        self.qi.fill(0);
        if !intra {
            runs(b, &mut self.partial, true)?;
            let n = self.partial.iter().filter(|&&v| v == 0).count();
            runs(b, &mut self.runs[..n], true)?;
            let mut cursor = 0;
            for (p, f) in self.partial.iter().zip(&mut self.full) {
                if *p == 0 {
                    *f = self.runs[cursor];
                    cursor += 1;
                }
            }
            let n = g
                .blocks
                .iter()
                .filter(|block| self.partial[block.sb] != 0)
                .count();
            runs(b, &mut self.runs[..n], false)?;
            cursor = 0;
            for (block, coded) in g.blocks.iter().zip(&mut self.coded) {
                *coded = if self.partial[block.sb] != 0 {
                    let v = self.runs[cursor];
                    cursor += 1;
                    v
                } else {
                    self.full[block.sb]
                };
            }
            self.decode_modes(b, g)?;
            self.decode_vectors(b, g)?;
        }
        for q in 0..nqis - 1 {
            let n = self
                .coded
                .iter()
                .zip(&self.qi)
                .filter(|(c, qi)| **c != 0 && **qi == q)
                .count();
            runs(b, &mut self.runs[..n], true)?;
            let mut cursor = 0;
            for (&coded, qi) in self.coded.iter().zip(&mut self.qi) {
                if coded != 0 && *qi == q {
                    *qi += self.runs[cursor] as usize;
                    cursor += 1;
                }
            }
        }
        self.decode_coeffs(b, g, setup)?;
        self.predict_dc(g);
        Ok(())
    }

    fn decode_modes(&mut self, b: &mut Bits<'_>, g: &Geometry) -> Result<()> {
        const ALPHABETS: [[u8; 8]; 6] = [
            [3, 4, 2, 0, 1, 5, 6, 7],
            [3, 4, 0, 2, 1, 5, 6, 7],
            [3, 2, 4, 0, 1, 5, 6, 7],
            [3, 2, 0, 4, 1, 5, 6, 7],
            [0, 3, 4, 2, 1, 5, 6, 7],
            [0, 5, 3, 4, 2, 1, 6, 7],
        ];
        let scheme = b.read(3)? as usize;
        let mut alphabet = [0; 8];
        if scheme == 0 {
            let mut used = 0u8;
            for mode in 0..8 {
                let i = b.read(3)? as usize;
                if used & (1 << i) != 0 {
                    return Err(FormatError("invalid Theora mode permutation"));
                }
                used |= 1 << i;
                alphabet[i] = mode;
            }
        } else if scheme < 7 {
            alphabet = ALPHABETS[scheme - 1];
        }
        for (mode, blocks) in self.modes.iter_mut().zip(&g.macroblocks) {
            if blocks[..4].iter().any(|&bi| self.coded[bi] != 0) {
                *mode = if scheme == 7 {
                    b.read(3)? as u8
                } else {
                    alphabet[unary(b, 7)?]
                };
            }
        }
        Ok(())
    }

    fn decode_vectors(&mut self, b: &mut Bits<'_>, g: &Geometry) -> Result<()> {
        let fixed = b.read(1)? != 0;
        let (mut last, mut prior) = ([0; 2], [0; 2]);
        for (&mode, blocks) in self.modes.iter().zip(&g.macroblocks) {
            if mode == 7 {
                let mut sum = [0i16; 2];
                let mut newest = last;
                for &bi in &blocks[..4] {
                    if self.coded[bi] != 0 {
                        let mv = vector(b, fixed)?;
                        self.vectors[bi] = mv;
                        newest = mv;
                        sum[0] += mv[0];
                        sum[1] += mv[1];
                    }
                }
                prior = last;
                last = newest;
                let average = sum.map(|v| v.signum() * ((v.abs() + 2) / 4));
                self.vectors[blocks[4]] = average;
                self.vectors[blocks[5]] = average;
                continue;
            }
            let mv = match mode {
                2 => {
                    prior = last;
                    last = vector(b, fixed)?;
                    last
                }
                3 => last,
                4 => {
                    std::mem::swap(&mut last, &mut prior);
                    last
                }
                6 => vector(b, fixed)?,
                _ => [0; 2],
            };
            for &bi in blocks {
                self.vectors[bi] = mv;
            }
        }
        Ok(())
    }

    fn decode_coeffs(&mut self, b: &mut Bits<'_>, g: &Geometry, setup: &Setup) -> Result<()> {
        self.positions.fill(64);
        self.ncoeffs.fill(0);
        self.active.clear();
        self.coded_blocks.clear();
        for (bi, &coded) in self.coded.iter().enumerate() {
            if coded != 0 {
                self.coded_blocks.push(bi);
                self.coeffs[bi].fill(0);
                self.positions[bi] = 0;
            }
        }
        self.active.extend_from_slice(&self.coded_blocks);
        let mut eobs = 0usize;
        let mut tables = [0; 2];
        for ti in 0..64 {
            if ti <= 1 {
                tables = [b.read(4)? as usize, b.read(4)? as usize];
            }
            let group = match ti {
                0 => 0,
                1..=5 => 1,
                6..=14 => 2,
                15..=27 => 3,
                _ => 4,
            };
            let mut remaining = 0;
            for cursor in 0..self.active.len() {
                let bi = self.active[cursor];
                if self.positions[bi] as usize == ti {
                    self.ncoeffs[bi] = ti as u8;
                    if eobs != 0 {
                        eobs -= 1;
                        self.positions[bi] = 64;
                    } else {
                        let table = group * 16 + tables[usize::from(bi >= g.macroblocks.len() * 4)];
                        let (token, extra) = setup.huffman[table].token(b, &TOKEN_EXTRA_BITS)?;
                        if token < 7 {
                            eobs = match token {
                                0..=2 => token as usize + 1,
                                3..=5 => (1 << (token - 1)) + extra as usize,
                                _ => {
                                    let n = extra as usize;
                                    if n == 0 {
                                        self.positions.iter().filter(|&&p| p < 64).count()
                                    } else {
                                        n
                                    }
                                }
                            };
                            eobs -= 1;
                            self.positions[bi] = 64;
                        } else {
                            let (run, value) = coefficient_value(token, extra);
                            let end = ti + run + usize::from(value != 0);
                            if end > 64 {
                                return Err(FormatError("Theora coefficient run exceeds block"));
                            }
                            if value != 0 {
                                self.coeffs[bi][ti + run] = value;
                                self.ncoeffs[bi] = end as u8;
                            }
                            self.positions[bi] = end as u8;
                        }
                    }
                }
                // Compact while decoding instead of scanning the same list a
                // second time. Writes never overtake the unread input cursor.
                if self.positions[bi] < 64 {
                    self.active[remaining] = bi;
                    remaining += 1;
                }
            }
            self.active.truncate(remaining);
        }
        if eobs != 0 || !self.active.is_empty() {
            return Err(FormatError("unfinished Theora coefficient stream"));
        }
        Ok(())
    }

    fn predict_dc(&mut self, g: &Geometry) {
        // Table 7.47, indexed by available left, lower-left, lower, lower-right.
        const WEIGHTS: [[i32; 5]; 16] = [
            [0, 0, 0, 0, 1],
            [1, 0, 0, 0, 1],
            [0, 1, 0, 0, 1],
            [1, 0, 0, 0, 1],
            [0, 0, 1, 0, 1],
            [1, 0, 1, 0, 2],
            [0, 0, 1, 0, 1],
            [29, -26, 29, 0, 32],
            [0, 0, 0, 1, 1],
            [75, 0, 0, 53, 128],
            [0, 1, 0, 1, 2],
            [75, 0, 0, 53, 128],
            [0, 0, 1, 0, 1],
            [75, 0, 0, 53, 128],
            [0, 3, 10, 3, 16],
            [29, -26, 29, 0, 32],
        ];
        for plane in 0..3 {
            let bw = g.sizes[plane].0 / 8;
            let mut last = [0i32; 3];
            for (ri, &bi) in g.raster[plane].iter().enumerate() {
                if self.coded[bi] == 0 {
                    continue;
                }
                let block = &g.blocks[bi];
                let rfi = reference(self.modes[block.mb]);
                let (x, y) = (block.x, block.y);
                let candidates = [
                    (x > 0).then(|| ri - 1),
                    (x > 0 && y > 0).then(|| ri - bw - 1),
                    (y > 0).then(|| ri - bw),
                    (x + 1 < bw && y > 0).then(|| ri - bw + 1),
                ];
                let mut dc = [0i32; 4];
                let mut mask = 0;
                for (k, neighbor) in candidates.into_iter().enumerate() {
                    if let Some(ri) = neighbor {
                        let bj = g.raster[plane][ri];
                        if self.coded[bj] != 0 && reference(self.modes[g.blocks[bj].mb]) == rfi {
                            dc[k] = i32::from(self.coeffs[bj][0]);
                            mask |= 1 << k;
                        }
                    }
                }
                let w = WEIGHTS[mask];
                let mut pred = if mask == 0 {
                    last[rfi]
                } else {
                    dc.iter().zip(w).map(|(d, w)| d * w).sum::<i32>() / w[4]
                };
                if mask & 7 == 7 {
                    for k in [2, 0, 1] {
                        if (pred - dc[k]).abs() > 128 {
                            pred = dc[k];
                            break;
                        }
                    }
                }
                let value = (i32::from(self.coeffs[bi][0]) + pred) as i16;
                self.coeffs[bi][0] = value;
                last[rfi] = i32::from(value);
            }
        }
    }
}

// Table 7.38, tokens 7..31: extra bits, zero-run base/bits, magnitude base/bits.
// Sign, magnitude and run share one read; their expansion is constant data.
const COEFFICIENT_SPECS: [(u8, u8, u8, i16, u8); 25] = [
    (3, 1, 3, 0, 0),
    (6, 1, 6, 0, 0),
    (0, 0, 0, 1, 0),
    (0, 0, 0, -1, 0),
    (0, 0, 0, 2, 0),
    (0, 0, 0, -2, 0),
    (1, 0, 0, 3, 0),
    (1, 0, 0, 4, 0),
    (1, 0, 0, 5, 0),
    (1, 0, 0, 6, 0),
    (2, 0, 0, 7, 1),
    (3, 0, 0, 9, 2),
    (4, 0, 0, 13, 3),
    (5, 0, 0, 21, 4),
    (6, 0, 0, 37, 5),
    (10, 0, 0, 69, 9),
    (1, 1, 0, 1, 0),
    (1, 2, 0, 1, 0),
    (1, 3, 0, 1, 0),
    (1, 4, 0, 1, 0),
    (1, 5, 0, 1, 0),
    (3, 6, 2, 1, 0),
    (4, 10, 3, 1, 0),
    (2, 1, 0, 2, 1),
    (3, 2, 1, 2, 1),
];

const TOKEN_EXTRA_BITS: [u8; 32] = {
    let mut bits = [0; 32];
    bits[3] = 2;
    bits[4] = 3;
    bits[5] = 4;
    bits[6] = 12;
    let mut i = 0;
    while i < COEFFICIENT_SPECS.len() {
        bits[i + 7] = COEFFICIENT_SPECS[i].0;
        i += 1;
    }
    bits
};
#[derive(Clone, Copy)]
struct Coefficient {
    run: u8,
    value: i16,
}

const COEFFICIENT_VALUE_COUNT: usize = {
    let mut total = 0;
    let mut i = 0;
    while i < COEFFICIENT_SPECS.len() {
        total += 1 << COEFFICIENT_SPECS[i].0;
        i += 1;
    }
    total
};

const fn coefficient_tables() -> ([u16; 25], [Coefficient; COEFFICIENT_VALUE_COUNT]) {
    let mut offsets = [0; 25];
    let mut values = [Coefficient { run: 0, value: 0 }; COEFFICIENT_VALUE_COUNT];
    let mut first = 0;
    let mut index = 0;
    while index < 25 {
        let (bits, run, run_bits, magnitude, magnitude_bits) = COEFFICIENT_SPECS[index];
        offsets[index] = first as u16;
        let mut extra = 0u16;
        while extra < 1 << bits {
            let run = run + (extra & ((1 << run_bits) - 1)) as u8;
            let value = magnitude + ((extra >> run_bits) & ((1 << magnitude_bits) - 1)) as i16;
            let value = if extra & (1 << (run_bits + magnitude_bits)) == 0 {
                value
            } else {
                -value
            };
            values[first + extra as usize] = Coefficient { run, value };
            extra += 1;
        }
        first += 1 << bits;
        index += 1;
    }
    assert!(first == values.len());
    (offsets, values)
}

static COEFFICIENT_TABLES: ([u16; 25], [Coefficient; COEFFICIENT_VALUE_COUNT]) =
    coefficient_tables();

#[inline]
#[cfg(test)]
fn coefficient(b: &mut Bits<'_>, token: u8) -> Result<(usize, i16)> {
    let extra = b.read(u32::from(TOKEN_EXTRA_BITS[token as usize]))?;
    Ok(coefficient_value(token, extra))
}

#[inline]
fn coefficient_value(token: u8, extra: u32) -> (usize, i16) {
    let first = COEFFICIENT_TABLES.0[token as usize - 7];
    let value = COEFFICIENT_TABLES.1[usize::from(first) + extra as usize];
    (usize::from(value.run), value.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theora::bits::packed;

    #[test]
    fn every_coefficient_extra_bits_value_matches_recorded_reference() {
        let mut recorded = Vec::new();
        for token in 7..32 {
            let bits = COEFFICIENT_SPECS[token as usize - 7].0;
            for extra in 0..1u16 << bits {
                let payload = (if bits == 0 { 0 } else { extra << (16 - bits) }).to_be_bytes();
                let mut reader = Bits::new(&payload);
                let (run, value) = coefficient(&mut reader, token).unwrap();
                assert_eq!(reader.remaining(), 16 - usize::from(bits));
                recorded.push(u8::try_from(run).unwrap());
                recorded.extend(value.to_le_bytes());
            }
        }
        assert_eq!(
            cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(&recorded)),
            "45cdb90d11b7deeb7e7f5414b2009cc362153f5ce23ffd53cc7bcee9a520287c"
        );
    }

    #[test]
    fn maximum_long_run_reads_a_fresh_value_and_runs_are_bounded() {
        let mut description = vec![1; 19]; // BIT=1, code=111111, offset=4095.
        description.extend([1, 0]); // Repeated BIT=1, run length 1.
        let bytes = packed(&description);
        let mut out = vec![0; 4130];
        runs(&mut Bits::new(&bytes), &mut out, true).unwrap();
        assert!(out.iter().all(|&v| v == 1));
        assert!(runs(&mut Bits::new(&bytes), &mut [0; 1], true).is_err());
        assert!(runs(&mut Bits::new(&[0xff, 0xff]), &mut [0; 1], false).is_err());
        runs(&mut Bits::new(&[]), &mut [], true).unwrap();
    }
}

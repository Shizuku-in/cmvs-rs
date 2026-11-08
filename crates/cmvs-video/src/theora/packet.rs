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
        self.coeffs.fill([0; 64]);
        self.positions.fill(64);
        self.ncoeffs.fill(0);
        self.active.clear();
        for (bi, &coded) in self.coded.iter().enumerate() {
            if coded != 0 {
                self.active.push(bi);
                self.positions[bi] = 0;
            }
        }
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
            for &bi in &self.active {
                if self.positions[bi] as usize != ti {
                    continue;
                }
                self.ncoeffs[bi] = ti as u8;
                if eobs == 0 {
                    let table = group * 16 + tables[usize::from(bi >= g.macroblocks.len() * 4)];
                    let token = setup.huffman[table].token(b)?;
                    if token < 7 {
                        eobs = match token {
                            0..=2 => token as usize + 1,
                            3..=5 => (1 << (token - 1)) + b.read(u32::from(token - 1))? as usize,
                            _ => {
                                let n = b.read(12)? as usize;
                                if n == 0 {
                                    self.positions.iter().filter(|&&p| p < 64).count()
                                } else {
                                    n
                                }
                            }
                        };
                    } else {
                        let (run, value) = coefficient(b, token)?;
                        let end = ti + run + usize::from(value != 0);
                        if end > 64 {
                            return Err(FormatError("Theora coefficient run exceeds block"));
                        }
                        if value != 0 {
                            self.coeffs[bi][ti + run] = value;
                            self.ncoeffs[bi] = end as u8;
                        }
                        self.positions[bi] = end as u8;
                        continue;
                    }
                }
                eobs -= 1;
                self.positions[bi] = 64;
            }
            self.active.retain(|&bi| self.positions[bi] < 64);
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

fn coefficient(b: &mut Bits<'_>, token: u8) -> Result<(usize, i16)> {
    if token <= 8 {
        return Ok((b.read(if token == 7 { 3 } else { 6 })? as usize + 1, 0));
    }
    if token <= 12 {
        return Ok((0, [1, -1, 2, -2][token as usize - 9]));
    }
    let sign = b.read(1)?;
    let (run, mag) = match token {
        13..=16 => (0, i16::from(token - 10)),
        17..=22 => {
            let (base, n) =
                [(7, 1), (9, 2), (13, 3), (21, 4), (37, 5), (69, 9)][token as usize - 17];
            (0, base + b.read(n)? as i16)
        }
        23..=27 => ((token - 22) as usize, 1),
        28 => (6 + b.read(2)? as usize, 1),
        29 => (10 + b.read(3)? as usize, 1),
        30 => (1, 2 + b.read(1)? as i16),
        _ => {
            let mag = 2 + b.read(1)? as i16;
            (2 + b.read(1)? as usize, mag)
        }
    };
    Ok((run, if sign == 0 { mag } else { -mag }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theora::bits::packed;

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

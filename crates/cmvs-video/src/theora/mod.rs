//! Theora 3.2 YUV420 packet decoder, following the Xiph Theora I specification.
//! Geometry, quantizers, entropy tables and frame storage are allocated at open.
mod bits;
mod dsp;
mod geometry;
mod headers;
mod packet;
#[cfg(test)]
mod tests;

use bits::{Bits, Result};
use cmvs_core::FormatError;
use geometry::Geometry;
use headers::Setup;
pub(crate) use headers::{Ident, comments};
use packet::State;

#[derive(Clone)]
pub(crate) struct Frame {
    pub planes: [Vec<u8>; 3],
}

impl Frame {
    fn new(g: &Geometry) -> Self {
        Self {
            planes: std::array::from_fn(|p| vec![0; g.sizes[p].0 * g.sizes[p].1]),
        }
    }
}

pub(crate) struct Decoder {
    geometry: Geometry,
    setup: Setup,
    state: State,
    frames: [Frame; 3],
    previous: usize,
    golden: usize,
    started: bool,
    failed: bool,
}

impl Decoder {
    pub fn new(ident: &Ident, packet: &[u8]) -> Result<Self> {
        let setup = Setup::parse(packet)?;
        let geometry = Geometry::new(ident);
        let state = State::new(&geometry);
        let frames = std::array::from_fn(|_| Frame::new(&geometry));
        Ok(Self {
            geometry,
            setup,
            state,
            frames,
            previous: 0,
            golden: 0,
            started: false,
            failed: false,
        })
    }

    pub fn decode_frame(&mut self, packet: &[u8]) -> Result<&Frame> {
        if self.failed {
            return Err(FormatError(
                "Theora decoder must be reset after a failed packet",
            ));
        }
        if packet.is_empty() {
            if !self.started {
                return Err(FormatError("first Theora frame is empty"));
            }
            return Ok(&self.frames[self.previous]);
        }
        self.failed = true;
        let mut b = Bits::new(packet);
        if b.read(1)? != 0 {
            return Err(FormatError("Theora header in data packet"));
        }
        let intra = b.read(1)? == 0;
        if !self.started && !intra {
            return Err(FormatError("first Theora frame must be intra"));
        }
        let mut qis = [0usize; 3];
        qis[0] = b.read(6)? as usize;
        let mut nqis = 1;
        while nqis < 3 && b.read(1)? != 0 {
            qis[nqis] = b.read(6)? as usize;
            nqis += 1;
        }
        if intra && b.read(3)? != 0 {
            return Err(FormatError("nonzero Theora frame reserved bits"));
        }
        self.state
            .decode(&mut b, intra, nqis, &self.geometry, &self.setup)?;
        let current = (0..3)
            .find(|&i| i != self.previous && i != self.golden)
            .unwrap();
        self.reconstruct(current, &qis, intra);
        self.filter(current, self.setup.limits[qis[0]]);
        self.previous = current;
        if intra {
            self.golden = current;
        }
        self.started = true;
        self.failed = false;
        Ok(&self.frames[current])
    }

    pub fn current_frame(&self) -> &Frame {
        &self.frames[self.previous]
    }

    fn reconstruct(&mut self, current: usize, qis: &[usize; 3], intra: bool) {
        const ZIGZAG: [usize; 64] = [
            0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34,
            27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37,
            44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
        ];
        let (before, after) = self.frames.split_at_mut(current);
        let (frame, after) = after.split_first_mut().unwrap();
        let reference = |index: usize| {
            if index < current {
                &before[index]
            } else {
                &after[index - current - 1]
            }
        };
        let previous = reference(self.previous);
        let golden = reference(self.golden);
        // Uncoded blocks are co-located copies. One plane copy avoids walking
        // every uncoded 8x8 tile, especially on nearly static inter frames.
        let prefilled = !intra && self.state.coded_blocks.len() < self.geometry.blocks.len();
        if prefilled {
            for (dst, src) in frame.planes.iter_mut().zip(&previous.planes) {
                dst.copy_from_slice(src);
            }
        }
        for &bi in &self.state.coded_blocks {
            let bi = bi as usize;
            let block = &self.geometry.blocks[bi];
            let p = usize::from(block.plane);
            let (width, height) = self.geometry.sizes[p];
            let (x, y) = (block.x as usize * 8, block.y as usize * 8);
            let mode = self.state.modes[block.mb as usize];
            let ncoeffs = usize::from(self.state.ncoeffs[bi]);
            let table = usize::from(mode != 1) * 3 + p;
            let dc_q = self.setup.quant[table][qis[0]][0];
            let dst = &mut frame.planes[p][y * width + x..];
            let inter_pred = |mv: [i16; 2]| {
                let divisor = if p == 0 { 2 } else { 4 };
                let first = mv.map(|v| i32::from(v) / divisor);
                let second = mv.map(|v| {
                    i32::from(v.signum()) * ((i32::from(v.abs()) + divisor - 1) / divisor)
                });
                let src = if matches!(mode, 5 | 6) {
                    golden
                } else {
                    previous
                };
                let source = &src.planes[p];
                let first_origin = [x as i32 + first[0], y as i32 + first[1]];
                let second_origin = [x as i32 + second[0], y as i32 + second[1]];
                let inside = |origin: [i32; 2]| {
                    origin[0] >= 0
                        && origin[1] >= 0
                        && origin[0] as usize + 8 <= width
                        && origin[1] as usize + 8 <= height
                };
                let offsets = (inside(first_origin) && inside(second_origin)).then(|| {
                    let a = (first_origin[1] as usize) * width + first_origin[0] as usize;
                    let b = (second_origin[1] as usize) * width + second_origin[0] as usize;
                    (a, (first != second).then_some(b))
                });
                (source, first, second, offsets)
            };
            // `coeffs` starts zeroed and decode_coeffs only writes `[..ncoeffs]`;
            // clearing each block right after reading it avoids per-block fill(0)
            // inside packet decoding while preserving the zero invariant.
            match ncoeffs {
                0 | 1 => {
                    let c0 = self.state.coeffs[bi][0];
                    self.state.coeffs[bi][0] = 0;
                    let dc = ((i32::from(c0) * i32::from(dc_q) + 15) >> 5) as i16;
                    if mode == 1 {
                        dsp::intra_dc(dst, width, dc);
                        continue;
                    }
                    let mv = self.state.vectors[bi];
                    if prefilled && !matches!(mode, 5 | 6) && mv == [0; 2] && dc == 0 {
                        continue;
                    }
                    let (source, first, second, offsets) = inter_pred(mv);
                    if let Some((a, b)) = offsets {
                        dsp::inter_dc(dst, width, &source[a..], b.map(|b| &source[b..]), dc);
                    } else {
                        reconstruct_border(
                            dst,
                            source,
                            (width, height),
                            (x, y),
                            first,
                            second,
                            &[[dc; 8]; 8],
                        );
                    }
                }
                2 | 3 => {
                    let ac = &self.setup.quant[table][qis[usize::from(self.state.qi[bi])]];
                    let coeffs = &mut self.state.coeffs[bi];
                    let c0 = (i32::from(coeffs[0]) * i32::from(dc_q)) as i16;
                    let c1 = (i32::from(coeffs[1]) * i32::from(ac[1])) as i16;
                    let c8 = if ncoeffs > 2 {
                        (i32::from(coeffs[2]) * i32::from(ac[8])) as i16
                    } else {
                        0
                    };
                    coeffs[..ncoeffs].fill(0);
                    if mode == 1 {
                        dsp::intra_sparse2(dst, width, c0, c1, c8);
                    } else {
                        let (source, first, second, offsets) = inter_pred(self.state.vectors[bi]);
                        if let Some((a, b)) = offsets {
                            dsp::inter_sparse2(
                                dst,
                                width,
                                &source[a..],
                                b.map(|b| &source[b..]),
                                c0,
                                c1,
                                c8,
                            );
                        } else {
                            let rows = dsp::idct_sparse2(c0, c1, c8);
                            reconstruct_border(
                                dst,
                                source,
                                (width, height),
                                (x, y),
                                first,
                                second,
                                &rows,
                            );
                        }
                    }
                }
                4..=10 => {
                    let ac = &self.setup.quant[table][qis[usize::from(self.state.qi[bi])]];
                    let coeffs = &mut self.state.coeffs[bi];
                    let mut s = [0i16; 10];
                    s[0] = (i32::from(coeffs[0]) * i32::from(dc_q)) as i16;
                    for zi in 1..ncoeffs {
                        s[zi] = (i32::from(coeffs[zi]) * i32::from(ac[ZIGZAG[zi]])) as i16;
                    }
                    coeffs[..ncoeffs].fill(0);
                    if mode == 1 {
                        dsp::intra_sparse4(dst, width, &s);
                    } else {
                        let (source, first, second, offsets) = inter_pred(self.state.vectors[bi]);
                        if let Some((a, b)) = offsets {
                            dsp::inter_sparse4(
                                dst,
                                width,
                                &source[a..],
                                b.map(|b| &source[b..]),
                                &s,
                            );
                        } else {
                            let rows = dsp::idct_sparse4(&s);
                            reconstruct_border(
                                dst,
                                source,
                                (width, height),
                                (x, y),
                                first,
                                second,
                                &rows,
                            );
                        }
                    }
                }
                _ => {
                    let ac = &self.setup.quant[table][qis[usize::from(self.state.qi[bi])]];
                    let coeffs = &mut self.state.coeffs[bi];
                    let mut dqc = [0i16; 64];
                    dqc[0] = (i32::from(coeffs[0]) * i32::from(dc_q)) as i16;
                    for zi in 1..ncoeffs {
                        let ci = ZIGZAG[zi];
                        dqc[ci] = (i32::from(coeffs[zi]) * i32::from(ac[ci])) as i16;
                    }
                    coeffs[..ncoeffs].fill(0);
                    if mode == 1 {
                        dsp::intra_full(dst, width, &dqc);
                    } else {
                        let (source, first, second, offsets) = inter_pred(self.state.vectors[bi]);
                        if let Some((a, b)) = offsets {
                            dsp::inter_full(
                                dst,
                                width,
                                &source[a..],
                                b.map(|b| &source[b..]),
                                &dqc,
                            );
                        } else {
                            let rows = dsp::idct(&dqc, ncoeffs as u8);
                            reconstruct_border(
                                dst,
                                source,
                                (width, height),
                                (x, y),
                                first,
                                second,
                                &rows,
                            );
                        }
                    }
                }
            }
        }
    }

    fn filter(&mut self, current: usize, limit: u8) {
        if limit == 0 {
            return;
        }
        for p in 0..3 {
            let (width, height) = self.geometry.sizes[p];
            let bw = width / 8;
            let bh = height / 8;
            let raster = &self.geometry.raster[p];
            let pixels = &mut self.frames[current].planes[p];
            for by in 0..bh {
                let y = by * 8;
                let row_ri = by * bw;
                for bx in 0..bw {
                    let bi = raster[row_ri + bx] as usize;
                    if self.state.coded[bi] == 0 {
                        continue;
                    }
                    let x = bx * 8;
                    if bx > 0 {
                        dsp::filter_v(pixels, y * width + x - 2, width, limit);
                    }
                    if by > 0 {
                        dsp::filter_h(pixels, (y - 2) * width + x, width, limit);
                    }
                    if bx + 1 < bw && self.state.coded[raster[row_ri + bx + 1] as usize] == 0 {
                        dsp::filter_v(pixels, y * width + x + 6, width, limit);
                    }
                    if by + 1 < bh && self.state.coded[raster[row_ri + bx + bw] as usize] == 0 {
                        dsp::filter_h(pixels, (y + 6) * width + x, width, limit);
                    }
                }
            }
        }
    }
}

/// Border motion vectors require per-sample edge replication outside the reference plane.
#[cold]
#[inline(never)]
fn reconstruct_border(
    dst: &mut [u8],
    source: &[u8],
    (width, height): (usize, usize),
    (x, y): (usize, usize),
    first: [i32; 2],
    second: [i32; 2],
    rows: &[[i16; 8]; 8],
) {
    for (row, residual) in rows.iter().enumerate() {
        let mut pred = [0; 8];
        let rx = x as i32 + first[0];
        let ry = (y as i32 + row as i32 + first[1]).clamp(0, height as i32 - 1) as usize;
        let ry2 = (y as i32 + row as i32 + second[1]).clamp(0, height as i32 - 1) as usize;
        for (col, pixel) in pred.iter_mut().enumerate() {
            let x1 = (rx + col as i32).clamp(0, width as i32 - 1) as usize;
            let x2 = (x as i32 + col as i32 + second[0]).clamp(0, width as i32 - 1) as usize;
            *pixel = ((u16::from(source[ry * width + x1]) + u16::from(source[ry2 * width + x2]))
                >> 1) as u8;
        }
        dsp::put(
            (&mut dst[row * width..row * width + 8]).try_into().unwrap(),
            &pred,
            residual,
        );
    }
}

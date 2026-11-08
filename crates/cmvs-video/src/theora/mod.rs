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
            let block = &self.geometry.blocks[bi];
            let p = block.plane;
            let (width, height) = self.geometry.sizes[p];
            let (x, y) = (block.x * 8, block.y * 8);
            let mode = self.state.modes[block.mb];
            let residual = {
                let table = usize::from(mode != 1) * 3 + p;
                let dc = &self.setup.quant[table][qis[0]];
                let coeffs = &self.state.coeffs[bi];
                if self.state.ncoeffs[bi] < 2 {
                    let dc = ((i32::from(coeffs[0]) * i32::from(dc[0]) + 15) >> 5) as i16;
                    dsp::Residual::Dc(dc)
                } else {
                    let ac = &self.setup.quant[table][qis[self.state.qi[bi]]];
                    let mut dqc = [0i16; 64];
                    dqc[0] = (i32::from(coeffs[0]) * i32::from(dc[0])) as i16;
                    for zi in 1..usize::from(self.state.ncoeffs[bi]) {
                        let ci = ZIGZAG[zi];
                        dqc[ci] = (i32::from(coeffs[zi]) * i32::from(ac[ci])) as i16;
                    }
                    dsp::Residual::Ac(dsp::idct(&dqc, self.state.ncoeffs[bi]))
                }
            };
            let offset = y * width + x;
            let dst = &mut frame.planes[p][offset..];
            if mode == 1 {
                dsp::intra(dst, width, &residual);
                continue;
            }
            let mv = self.state.vectors[bi];
            if prefilled
                && !matches!(mode, 5 | 6)
                && mv == [0; 2]
                && matches!(&residual, dsp::Residual::Dc(0))
            {
                continue;
            }
            let divisor = if p == 0 { 2 } else { 4 };
            let first = mv.map(|v| i32::from(v) / divisor);
            let second =
                mv.map(|v| i32::from(v.signum()) * ((i32::from(v.abs()) + divisor - 1) / divisor));
            let source = if matches!(mode, 5 | 6) {
                golden
            } else {
                previous
            };
            let source = &source.planes[p];
            let first_origin = [x as i32 + first[0], y as i32 + first[1]];
            let second_origin = [x as i32 + second[0], y as i32 + second[1]];
            let inside = |origin: [i32; 2]| {
                origin[0] >= 0
                    && origin[1] >= 0
                    && origin[0] as usize + 8 <= width
                    && origin[1] as usize + 8 <= height
            };
            let interior = inside(first_origin) && inside(second_origin);
            if interior {
                let a = (first_origin[1] as usize) * width + first_origin[0] as usize;
                let b = (second_origin[1] as usize) * width + second_origin[0] as usize;
                dsp::inter(
                    dst,
                    width,
                    &source[a..],
                    if first == second {
                        None
                    } else {
                        Some(&source[b..])
                    },
                    &residual,
                );
                continue;
            }
            // Border vectors require per-sample edge replication.
            for row in 0..8 {
                let mut pred = [0; 8];
                let rx = x as i32 + first[0];
                let ry = (y as i32 + row as i32 + first[1]).clamp(0, height as i32 - 1) as usize;
                let ry2 = (y as i32 + row as i32 + second[1]).clamp(0, height as i32 - 1) as usize;
                for (col, pixel) in pred.iter_mut().enumerate() {
                    let x1 = (rx + col as i32).clamp(0, width as i32 - 1) as usize;
                    let x2 =
                        (x as i32 + col as i32 + second[0]).clamp(0, width as i32 - 1) as usize;
                    *pixel = ((u16::from(source[ry * width + x1])
                        + u16::from(source[ry2 * width + x2]))
                        >> 1) as u8;
                }
                dsp::put(
                    (&mut dst[row * width..row * width + 8]).try_into().unwrap(),
                    &pred,
                    &residual.row(row),
                );
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
            let raster = &self.geometry.raster[p];
            let pixels = &mut self.frames[current].planes[p];
            for (ri, &bi) in raster.iter().enumerate() {
                if self.state.coded[bi] == 0 {
                    continue;
                }
                let (x, y) = ((ri % bw) * 8, (ri / bw) * 8);
                if x > 0 {
                    edge(pixels, y * width + x - 2, 1, width, limit);
                }
                if y > 0 {
                    edge(pixels, (y - 2) * width + x, width, 1, limit);
                }
                if x + 8 < width && self.state.coded[raster[ri + 1]] == 0 {
                    edge(pixels, y * width + x + 6, 1, width, limit);
                }
                if y + 8 < height && self.state.coded[raster[ri + bw]] == 0 {
                    edge(pixels, (y + 6) * width + x, width, 1, limit);
                }
            }
        }
    }
}

fn edge(pixels: &mut [u8], start: usize, step: usize, advance: usize, limit: u8) {
    let limit = i32::from(limit);
    for i in 0..8 {
        let a = start + advance * i;
        let p = std::array::from_fn::<_, 4, _>(|i| i32::from(pixels[a + step * i]));
        let r = (p[0] - 3 * p[1] + 3 * p[2] - p[3] + 4) >> 3;
        let delta = r.signum() * r.abs().min((2 * limit - r.abs()).max(0));
        pixels[a + step] = (p[1] + delta).clamp(0, 255) as u8;
        pixels[a + step * 2] = (p[2] - delta).clamp(0, 255) as u8;
    }
}

//! Common PB2/PB3 split streams, color planes and tile patches.
use crate::{FormatError, MAX_PAYLOAD, lzss::encode_split, raster::RgbaImage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compression {
    Lzss,
    Literals,
}

impl Compression {
    pub(crate) fn split(self, bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        match self {
            Self::Lzss => encode_split(bytes),
            Self::Literals => (vec![0; bytes.len().div_ceil(8)], bytes.to_vec()),
        }
    }
}

pub(crate) fn put(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
}

/// Visit one plane in 8x8 tile order, clipping both edges to the image.
pub(crate) fn tiles(img: &RgbaImage, lane: usize, block_size: usize, mut write: impl FnMut(&[u8])) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut block = [0; 256];
    for y in (0..h).step_by(block_size) {
        for x in (0..w).step_by(block_size) {
            let mut n = 0;
            for row in y..(y + block_size).min(h) {
                for col in x..(x + block_size).min(w) {
                    block[n] = img.pixels()[(row * w + col) * 4 + lane];
                    n += 1;
                }
            }
            write(&block[..n]);
        }
    }
}

pub(crate) fn blocked_plane(
    img: &RgbaImage,
    lane: usize,
    block: usize,
    mode: Compression,
) -> (Vec<u8>, Vec<u8>) {
    let mut mask = Vec::new();
    let mut solid = Vec::new();
    let mut literals = Vec::new();
    let mut bit = 0;
    tiles(img, lane, block, |tile| {
        if bit == 0 {
            mask.push(0);
        }
        if mode == Compression::Lzss && tile.iter().all(|&v| v == tile[0]) {
            *mask.last_mut().unwrap() |= 128 >> bit;
            solid.push(tile[0]);
        } else {
            literals.extend_from_slice(tile);
        }
        bit = (bit + 1) % 8;
    });
    let (flags, data) = mode.split(&literals);
    let mut metadata = Vec::new();
    for size in [mask.len(), solid.len(), literals.len()] {
        metadata.extend((size as u32).to_le_bytes());
    }
    metadata.extend(mask);
    metadata.extend(solid);
    metadata.extend(flags);
    (metadata, data)
}

pub(crate) fn chunks(planes: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for plane in &planes {
        bytes.extend((plane.len() as u32).to_le_bytes());
    }
    for plane in planes {
        bytes.extend(plane);
    }
    bytes
}

/// Four independently compressed planes, with offsets relative to the end of the table.
pub(crate) fn planes(img: &RgbaImage, xor: bool, mode: Compression) -> Vec<u8> {
    let mut out = vec![0; 32];
    for (i, lane) in [2, 1, 0, 3].into_iter().enumerate() {
        let mut last = 0u8;
        let mut values = Vec::with_capacity(img.pixels().len() / 4);
        for p in img.pixels().as_chunks::<4>().0 {
            let v = p[lane];
            values.push(if xor {
                match i {
                    0 => v ^ p[1],
                    1 => v ^ p[0],
                    2 => v ^ p[3],
                    _ => v,
                }
            } else {
                v.wrapping_sub(last)
            });
            last = v;
        }
        let (flags, data) = mode.split(&values);
        let offset = out.len() - 32;
        put(&mut out, 8 * i, offset);
        put(&mut out, 8 * i + 4, offset + flags.len());
        out.extend(flags);
        out.extend(data);
    }
    out
}

/// An unset mask bit replaces the entire tile. Animation patches store opaque BGR
/// and can only address complete tiles. Named parts and PB3 overlays store clipped BGRA.
pub(crate) fn patch(
    base: &RgbaImage,
    img: &RgbaImage,
    animation: bool,
) -> Result<Vec<u8>, FormatError> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if base.width() != img.width() || base.height() != img.height() {
        return Err(FormatError("overlay dimensions differ from base"));
    }
    let (nx, ny) = if animation {
        (w / 8, h / 8)
    } else {
        (w.div_ceil(8), h.div_ceil(8))
    };
    let mask = (nx * ny).div_ceil(8);
    let mut out = vec![0; 8 + mask];
    put(&mut out, 0, mask);
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            let rows = y..(y + 8).min(h);
            let row = |yy: usize| ((yy * w + x) * 4)..((yy * w + (x + 8).min(w)) * 4);
            let changed = rows
                .clone()
                .any(|yy| img.pixels()[row(yy)] != base.pixels()[row(yy)]);
            if x / 8 >= nx || y / 8 >= ny {
                if changed {
                    return Err(FormatError(
                        "PB2 animation cannot change partial edge tiles",
                    ));
                }
                continue;
            }
            let bit = y / 8 * nx + x / 8;
            if !changed {
                out[8 + bit / 8] |= 128 >> (bit % 8);
                continue;
            }
            for yy in rows {
                for &[r, g, b, a] in img.pixels()[row(yy)].as_chunks::<4>().0 {
                    if animation && a != 255 {
                        return Err(FormatError("changed PB2 animation tile must be opaque"));
                    }
                    out.extend([b, g, r]);
                    if !animation {
                        out.push(a);
                    }
                }
            }
        }
    }
    let payload_len = out.len() - 8 - mask;
    put(&mut out, 4, payload_len);
    if out.len() > MAX_PAYLOAD {
        return Err(FormatError("PB patch exceeds allocation limit"));
    }
    Ok(out)
}

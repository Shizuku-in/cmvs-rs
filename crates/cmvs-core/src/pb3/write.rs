//! Self-contained images and named PB3 overlays.
use super::{Compression, EncodeDepth, NAME_KEY};
use crate::{
    FormatError, MAX_PAYLOAD,
    pb_write::{blocked_plane, chunks, patch, planes, put},
    raster::RgbaImage,
};

pub enum ImageType<'a> {
    Blocked(EncodeDepth),
    Delta,
    Interleaved,
    Overlay {
        kind: u16,
        stem: &'a [u8],
        base: &'a RgbaImage,
    },
}

pub fn encode_image(
    img: &RgbaImage,
    format: ImageType<'_>,
    mode: Compression,
) -> Result<Vec<u8>, FormatError> {
    if let ImageType::Blocked(depth) = format
        && mode == Compression::Literals
    {
        return super::encode(img, depth);
    }
    let w = u16::try_from(img.width()).map_err(|_| FormatError("PB3 width exceeds 16 bits"))?;
    let h = u16::try_from(img.height()).map_err(|_| FormatError("PB3 height exceeds 16 bits"))?;
    let mut raw = vec![0; 52];
    raw[..4].copy_from_slice(b"PB3B");
    let (kind, bpp) = match format {
        ImageType::Blocked(depth) => {
            if depth == EncodeDepth::Rgb24
                && img.pixels().iter().skip(3).step_by(4).any(|&a| a != 255)
            {
                return Err(FormatError("24-bit PB3 requires opaque pixels"));
            }
            let lanes = &[2, 1, 0, 3][..usize::from(depth.bpp() / 8)];
            let (flags, data) = lanes
                .iter()
                .map(|&lane| blocked_plane(img, lane, 16, mode))
                .unzip();
            put(&mut raw, 8, 1);
            put(&mut raw, 24, 16);
            put(&mut raw, 44, 52);
            raw.extend(chunks(flags));
            let at = raw.len();
            put(&mut raw, 48, at);
            raw.extend(chunks(data));
            (1, depth.bpp())
        }
        ImageType::Delta => {
            put(&mut raw, 12, 52);
            raw.extend(planes(img, false, mode));
            (5, 32)
        }
        ImageType::Interleaved => {
            put(&mut raw, 12, 52);
            let mut bgra = img.pixels().to_vec();
            for p in bgra.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            let (flags, data) = mode.split(&bgra);
            raw.extend(0u32.to_le_bytes());
            raw.extend((flags.len() as u32).to_le_bytes());
            raw.extend(flags);
            raw.extend(data);
            (7, 32)
        }
        ImageType::Overlay { kind, stem, base } => {
            if !matches!(kind, 6 | 8)
                || stem.is_empty()
                || stem.len() > 31
                || stem.contains(&0)
                || stem.to_ascii_lowercase().ends_with(b".pb3")
            {
                return Err(FormatError(
                    "expected type 6/8 and a 1..31-byte PB3 reference stem",
                ));
            }
            let mut name = stem.to_vec();
            name.extend(b".pb3");
            crate::cpz::lookup_key(&name)?;
            let patch = patch(base, img, false)?;
            let (flags, data) = mode.split(&patch);
            put(&mut raw, 12, 52);
            put(&mut raw, 24, patch.len());
            put(&mut raw, 44, flags.len());
            put(&mut raw, 48, data.len());
            raw.extend((0..32).map(|i| stem.get(i).copied().unwrap_or(0) ^ NAME_KEY[i % 16]));
            raw.extend(flags);
            raw.extend(data);
            (kind, 32)
        }
    };
    for (i, v) in [kind, w, h, bpp].into_iter().enumerate() {
        raw[28 + i * 2..30 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    let size = raw.len() + 47;
    if size > MAX_PAYLOAD {
        return Err(FormatError("encoded PB3 exceeds allocation limit"));
    }
    put(&mut raw, 4, size);
    raw.resize(size, 0);
    Ok(raw)
}

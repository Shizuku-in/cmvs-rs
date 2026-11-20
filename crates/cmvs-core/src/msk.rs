//! MSK0 8-bit transition masks and glyph atlases used by CVNS.

// Format reference: cvns.exe SHA-256 8e6044466b928d77f0a94a0d0f32fe48190596bf628115598881c2aa3a4885aa,
// sub_43D090 (base 0x400000).
use crate::{FormatError, MAX_PAYLOAD, pb3::u32_at, raster::RgbaImage};

/// Parsed MSK0 mask header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub data_offset: u32,
    pub width: u32,
    pub height: u32,
}

impl Header {
    /// Parses and validates an MSK0 mask header against research bounds.
    pub fn parse(raw: &[u8]) -> Result<Self, FormatError> {
        if raw.len() > MAX_PAYLOAD || raw.get(..4) != Some(b"MSK0") {
            return Err(FormatError("expected bounded MSK0 mask"));
        }
        let h = Self {
            data_offset: u32_at(raw, 4)?,
            width: u32_at(raw, 8)?,
            height: u32_at(raw, 12)?,
        };
        let pixels = pixel_count(h.width, h.height)?;
        let offset = h.data_offset as usize;
        let end = offset
            .checked_add(pixels)
            .ok_or(FormatError("MSK0 payload end overflow"))?;
        if offset < 16 || end > raw.len() {
            return Err(FormatError("MSK0 mask pixels lie outside file"));
        }
        Ok(h)
    }
}

/// Number of mask samples within the decoder's RGBA allocation limit.
pub fn pixel_count(width: u32, height: u32) -> Result<usize, FormatError> {
    let count = u64::from(width) * u64::from(height);
    if count == 0 || count > (MAX_PAYLOAD / 4) as u64 {
        return Err(FormatError("MSK0 dimensions exceed research bounds"));
    }
    Ok(count as usize)
}

/// Write an MSK0 with contiguous rows of unmodified, single-channel samples.
#[cfg(feature = "encode")]
pub fn encode(width: u32, height: u32, samples: &[u8]) -> Result<Vec<u8>, FormatError> {
    if samples.len() != pixel_count(width, height)? {
        return Err(FormatError("MSK0 sample count differs from dimensions"));
    }
    let mut out = Vec::with_capacity(16 + samples.len());
    out.extend_from_slice(b"MSK0");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(samples);
    Ok(out)
}

/// Decodes raw 8-bit mask intensities along with `(width, height)`.
pub fn decode_bytes(raw: &[u8]) -> Result<(u32, u32, Vec<u8>), FormatError> {
    let h = Header::parse(raw)?;
    let offset = h.data_offset as usize;
    let count = (h.width as usize) * (h.height as usize);
    Ok((h.width, h.height, raw[offset..offset + count].to_vec()))
}

/// Decodes an MSK0 mask into opaque grayscale RGBA (`[v, v, v, 255]`) for compositor masks.
pub fn decode(raw: &[u8]) -> Result<RgbaImage, FormatError> {
    let (width, height, mask) = decode_bytes(raw)?;
    let mut pixels = vec![255u8; mask.len() * 4];
    for (pixel, value) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
        *pixel = [value, value, value, 255];
    }
    RgbaImage::new(width, height, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_mask(data_offset: u32, width: u32, height: u32, payload: &[u8]) -> Vec<u8> {
        let mut raw = b"MSK0".to_vec();
        raw.extend(data_offset.to_le_bytes());
        raw.extend(width.to_le_bytes());
        raw.extend(height.to_le_bytes());
        if (data_offset as usize) > raw.len() {
            raw.resize(data_offset as usize, 0xaa);
        }
        raw.extend_from_slice(payload);
        raw
    }

    #[test]
    fn decodes_valid_mask_with_header_offset_to_grayscale_rgba() {
        let raw = synthetic_mask(24, 2, 2, &[0, 64, 128, 255]);
        let (width, height, bytes) = decode_bytes(&raw).unwrap();
        assert_eq!((width, height, bytes), (2, 2, vec![0, 64, 128, 255]));
        let image = decode(&raw).unwrap();
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
        assert_eq!(
            image.pixels(),
            &[
                0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 255, 255, 255, 255
            ]
        );
    }

    #[test]
    fn rejects_invalid_magic_offset_dimensions_and_truncation() {
        assert!(decode(b"MSK1\x10\0\0\0\x01\0\0\0\x01\0\0\0\x00").is_err());
        assert!(decode(&synthetic_mask(12, 1, 1, &[1])).is_err());
        assert!(decode(&synthetic_mask(16, 0, 2, &[])).is_err());
        assert!(decode(&synthetic_mask(16, 2, 2, &[1, 2, 3])).is_err());
    }
}

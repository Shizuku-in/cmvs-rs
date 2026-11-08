//! Literal PB3B type-1 writer shared by the offline fan patch converters.
use cmvs_core::pb3;
use std::error::Error;

pub(crate) fn encode_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let count = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > 65535
        || height > 65535
        || count * 4 > cmvs_core::MAX_PAYLOAD as u64
        || count * 4 != rgba.len() as u64
    {
        return Err("invalid bounded RGBA dimensions".into());
    }
    let count = count as usize;
    // Store all four channels, including alpha, in one literal tile.
    let mut flags = Vec::new();
    for value in [1u32, 0, count as u32] {
        flags.extend(value.to_le_bytes());
    }
    flags.resize(13 + count.div_ceil(8), 0);
    let data_at = 52 + 16 + flags.len() * 4;
    let mut out = vec![0; 52];
    out[..4].copy_from_slice(b"PB3B");
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[24..28].copy_from_slice(&width.max(height).to_le_bytes());
    for (i, value) in [1, width as u16, height as u16, 32].iter().enumerate() {
        out[28 + i * 2..30 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
    out[44..48].copy_from_slice(&52u32.to_le_bytes());
    out[48..52].copy_from_slice(&(data_at as u32).to_le_bytes());
    for _ in 0..4 {
        out.extend((flags.len() as u32).to_le_bytes());
    }
    for _ in 0..4 {
        out.extend(&flags);
    }
    for _ in 0..4 {
        out.extend((count as u32).to_le_bytes());
    }
    for channel in [2, 1, 0, 3] {
        out.extend(rgba.iter().skip(channel).step_by(4));
    }
    out.extend([0; 47]);
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    let decoded = pb3::decode(&out)?;
    if decoded.width() != width || decoded.height() != height || decoded.pixels() != rgba {
        return Err("PB3B full RGBA roundtrip differs".into());
    }
    Ok(out)
}

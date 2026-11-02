//! 0x749: Aoitori 0x140042cf0/0x140042d50; ROOT 0x14003e5a0/0x14003e600.
use super::{Reader, words};
use crate::FormatError;

pub(super) fn validate(vertices: &[[u32; 3]]) -> Result<(), FormatError> {
    if vertices.len() > 128 {
        return Err(FormatError("saved collision vertex count exceeds 128"));
    }
    if vertices
        .iter()
        .flatten()
        .any(|&v| !f32::from_bits(v).is_finite())
    {
        return Err(FormatError("saved collision vertex must be finite"));
    }
    Ok(())
}

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<[u32; 3]>, FormatError> {
    let mut r = Reader::new(bytes);
    // Both constructors initialize the first word to zero; no other value is modeled.
    if r.u32()? != 0 {
        return Err(FormatError("unsupported saved collision header"));
    }
    let count = r.u32()? as usize;
    if count > 128 || bytes.len() != 8 + count * 12 {
        return Err(FormatError(
            "invalid saved collision vertex count or length",
        ));
    }
    let vertices = (0..count)
        .map(|_| r.words())
        .collect::<Result<Vec<_>, _>>()?;
    r.finish()?;
    validate(&vertices)?;
    Ok(vertices)
}

pub(super) fn encode(vertices: &[[u32; 3]]) -> Result<Vec<u8>, FormatError> {
    validate(vertices)?;
    let mut out = Vec::with_capacity(8 + vertices.len() * 12);
    words(&mut out, &[0, vertices.len() as u32]);
    for vertex in vertices {
        words(&mut out, vertex);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_layout_and_invalid_records() {
        let vertex = [1.0f32.to_bits(), (-0.0f32).to_bits(), (-3.0f32).to_bits()];
        let mut expected = vec![0, 0, 0, 0, 1, 0, 0, 0];
        for value in vertex {
            expected.extend(value.to_le_bytes());
        }
        assert_eq!(encode(&[vertex]).unwrap(), expected);
        assert_eq!(decode(&expected).unwrap(), [vertex]);
        for end in 0..expected.len() {
            assert!(decode(&expected[..end]).is_err());
        }
        let mut trailing = expected.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        for (offset, value) in [(0, 1), (4, 129), (4, u32::MAX), (8, f32::NAN.to_bits())] {
            let mut invalid = expected.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&invalid).is_err());
        }
        assert!(encode(&vec![vertex; 129]).is_err());
        assert!(encode(&[[f32::INFINITY.to_bits(), 0, 0]]).is_err());
    }
}

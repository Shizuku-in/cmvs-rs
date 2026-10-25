//! CMVS 2048-byte ring LZSS. See docs/references.md for provenance.
use crate::{FormatError, MAX_PAYLOAD};

pub fn decode(input: &[u8], expected: usize) -> Result<Vec<u8>, FormatError> {
    if expected > MAX_PAYLOAD {
        return Err(FormatError(
            "decoded size exceeds research allocation limit",
        ));
    }
    let mut ring = [0u8; 2048];
    let mut write = 0x7df;
    let mut cursor = 0;
    let mut flags = 0u16;
    let mut output = Vec::with_capacity(expected);
    while cursor < input.len() {
        flags >>= 1;
        if flags & 0x100 == 0 {
            flags = u16::from(input[cursor]) | 0xff00;
            cursor += 1;
        }
        if flags & 1 != 0 {
            let value = *input
                .get(cursor)
                .ok_or(FormatError("truncated LZSS literal"))?;
            cursor += 1;
            if output.len() == expected {
                return Err(FormatError("LZSS output exceeds declared size"));
            }
            output.push(value);
            ring[write] = value;
            write = (write + 1) & 0x7ff;
        } else {
            let pair = input
                .get(cursor..cursor + 2)
                .ok_or(FormatError("truncated LZSS reference"))?;
            cursor += 2;
            let offset = usize::from(pair[0]) | (usize::from(pair[1] & 0xe0) << 3);
            let length = usize::from(pair[1] & 31) + 2;
            if length > expected - output.len() {
                return Err(FormatError("LZSS reference exceeds declared size"));
            }
            for i in 0..length {
                let value = ring[(offset + i) & 0x7ff];
                output.push(value);
                ring[write] = value;
                write = (write + 1) & 0x7ff;
            }
        }
    }
    if output.len() != expected {
        return Err(FormatError("LZSS decoded length differs from header"));
    }
    Ok(output)
}

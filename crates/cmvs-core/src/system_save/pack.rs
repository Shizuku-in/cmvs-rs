//! CSS1 encoder 0x14008db90: 1024-byte dictionary, 48-byte lookahead (start 976),
//! 10-bit positions and 6-bit lengths; insertion 0x14008d870 and removal 0x14008da30 are the
//! tree operations shared with the CSV2 encoder in `lzss`.
pub(super) fn compress(input: &[u8]) -> Vec<u8> {
    crate::lzss::encode_tree(input, 1024, 48, |position| ((position >> 2) & 0xc0) as u8)
}

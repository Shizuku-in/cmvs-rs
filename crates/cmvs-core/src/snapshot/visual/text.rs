use super::Reader;
use crate::{
    FormatError,
    message::{LineBreakRules, TextObject},
};

pub(super) fn decode(bytes: &[u8]) -> Result<TextObject, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 3 {
        return Err(FormatError("unsupported snapshot text version"));
    }
    if !matches!(r.u32()?, 0 | 196) {
        return Err(FormatError("unsupported snapshot text atlas capacity"));
    }
    if !r.string(64)?.is_empty() {
        return Err(FormatError("snapshot named font face is not implemented"));
    }
    let logical_size = r.words::<2>()?.map(|v| v as i32);
    if logical_size.iter().any(|&v| v <= 0) {
        return Err(FormatError("snapshot text logical size must be positive"));
    }
    let a = r.take(156)?;
    let b = r.take(148)?;
    let c = r.take(208)?;
    let word = |block: &[u8], at: usize| u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
    // A begins at native +8, B at +176, C at +328.
    let aw = |offset: usize| word(a, offset - 8);
    let bw = |offset: usize| word(b, offset - 176);
    let cw = |offset: usize| word(c, offset - 328);
    if a[36..100].iter().any(|&v| v != 0) || aw(108) != 0 || aw(112) != 1 || bw(180) != 148 {
        return Err(FormatError("unsupported snapshot text font or layout mode"));
    }
    let line_break = if cw(328) == 0 {
        None
    } else {
        Some(LineBreakRules {
            max_overflow: cw(332) as i32,
            hanging: Reader::new(&c[8..72]).string(64)?.to_vec(),
            inseparable: Reader::new(&c[72..200]).string(128)?.to_vec(),
        })
    };
    if r.u16()? != 0 {
        return Err(FormatError(
            "snapshot displayed glyph reconstruction is not implemented",
        ));
    }
    r.finish()?;
    let area = [aw(16), aw(20), aw(24), aw(28)].map(|v| v as i32);
    Ok(TextObject {
        scale_bits: 1.0f32.to_bits(),
        logical_size,
        position: [aw(8) as i32, aw(12) as i32],
        area,
        // The native reader resets +156/+160 before regenerating glyphs.
        cursor: [area[0], area[1]],
        ruby_lines: std::array::from_fn(|i| bw(196 + i * 4)),
        layout_scratch: [cw(528), cw(532)],
        glyphs: Vec::new(),
        glyph_size: aw(32) as i32,
        spacing: [aw(36) as i32, aw(40) as i32],
        color: aw(116),
        decoration_color: aw(120),
        outline_mode: aw(124),
        glyph_parameter: aw(128),
        decoration: aw(132) as i32,
        reveal: [aw(136) as i32, aw(140) as i32],
        char_delay: aw(144) as i32,
        layer: aw(152) as i32,
        visible: aw(148) != 0,
        ruby: bw(176) != 0,
        ruby_every_line: bw(184) != 0,
        ruby_size: bw(188) as i32,
        ruby_spacing: bw(192) as i32,
        line_break,
    })
}

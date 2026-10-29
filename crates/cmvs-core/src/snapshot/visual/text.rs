//! Text object record 0x380: writer 0x14005c790, reader 0x14005c8f0.
//! Layout: u16 version 3, u32 +324 capacity, NUL font name (+44), +552/+556 logical size,
//! blocks +8..+164, +176..+324 and +328..+536, u16 glyph count (then 44-byte glyphs).
use super::Reader;
use crate::{
    FormatError,
    message::{LineBreakRules, TextObject},
};

/// Decodes the saved fields as written; the loader's pen reset is applied by callers.
pub(crate) fn decode(bytes: &[u8]) -> Result<TextObject, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 3 {
        return Err(FormatError("unsupported snapshot text version"));
    }
    let glyph_capacity = r.u32()?;
    if !matches!(glyph_capacity, 0 | 196) {
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
    if cw(328) > 1 {
        return Err(FormatError("unsupported snapshot line-break flag"));
    }
    let line_break = if cw(328) == 0 {
        if c[4..200].iter().any(|&v| v != 0) {
            return Err(FormatError(
                "disabled snapshot line-break rules keep residual bytes",
            ));
        }
        None
    } else {
        let field = |range: std::ops::Range<usize>, limit| {
            let field = &c[range];
            let text = Reader::new(field).string(limit)?;
            if field[text.len()..].iter().any(|&v| v != 0) {
                return Err(FormatError(
                    "snapshot line-break field keeps residual bytes",
                ));
            }
            Ok(text.to_vec())
        };
        Some(LineBreakRules {
            max_overflow: cw(332) as i32,
            hanging: field(8..72, 64)?,
            inseparable: field(72..200, 128)?,
        })
    };
    if r.u16()? != 0 {
        return Err(FormatError(
            "snapshot displayed glyph reconstruction is not implemented",
        ));
    }
    r.finish()?;
    Ok(TextObject {
        scale_bits: 1.0f32.to_bits(),
        logical_size,
        position: [aw(8) as i32, aw(12) as i32],
        area: [aw(16), aw(20), aw(24), aw(28)].map(|v| v as i32),
        cursor: [aw(156) as i32, aw(160) as i32],
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
        glyph_capacity,
    })
}

/// Writer 0x14005c790 for the supported subset: default face, no displayed glyphs.
pub(crate) fn encode(text: &TextObject) -> Result<Vec<u8>, FormatError> {
    if !text.glyphs.is_empty() {
        return Err(FormatError(
            "saving displayed text glyphs is not implemented",
        ));
    }
    let mut a = [0u8; 156];
    let mut b = [0u8; 148];
    let mut c = [0u8; 208];
    fn put(block: &mut [u8], base: usize, offset: usize, value: u32) {
        block[offset - base..offset - base + 4].copy_from_slice(&value.to_le_bytes());
    }
    let words = [
        (8, text.position[0] as u32),
        (12, text.position[1] as u32),
        (16, text.area[0] as u32),
        (20, text.area[1] as u32),
        (24, text.area[2] as u32),
        (28, text.area[3] as u32),
        (32, text.glyph_size as u32),
        (36, text.spacing[0] as u32),
        (40, text.spacing[1] as u32),
        (112, 1),
        (116, text.color),
        (120, text.decoration_color),
        (124, text.outline_mode),
        (128, text.glyph_parameter),
        (132, text.decoration as u32),
        (136, text.reveal[0] as u32),
        (140, text.reveal[1] as u32),
        (144, text.char_delay as u32),
        (148, u32::from(text.visible)),
        (152, text.layer as u32),
        (156, text.cursor[0] as u32),
        (160, text.cursor[1] as u32),
    ];
    for (offset, value) in words {
        put(&mut a, 8, offset, value);
    }
    put(&mut b, 176, 176, u32::from(text.ruby));
    put(&mut b, 176, 180, 148);
    put(&mut b, 176, 184, u32::from(text.ruby_every_line));
    put(&mut b, 176, 188, text.ruby_size as u32);
    put(&mut b, 176, 192, text.ruby_spacing as u32);
    for (i, &line) in text.ruby_lines.iter().enumerate() {
        put(&mut b, 176, 196 + i * 4, line);
    }
    if let Some(rules) = &text.line_break {
        if rules.hanging.len() >= 64 || rules.inseparable.len() >= 128 {
            return Err(FormatError("line-break rules exceed their snapshot fields"));
        }
        put(&mut c, 328, 328, 1);
        put(&mut c, 328, 332, rules.max_overflow as u32);
        c[8..8 + rules.hanging.len()].copy_from_slice(&rules.hanging);
        c[72..72 + rules.inseparable.len()].copy_from_slice(&rules.inseparable);
    }
    put(&mut c, 328, 528, text.layout_scratch[0]);
    put(&mut c, 328, 532, text.layout_scratch[1]);
    let mut out = 3u16.to_le_bytes().to_vec();
    out.extend(text.glyph_capacity.to_le_bytes());
    out.push(0);
    for size in text.logical_size {
        out.extend((size as u32).to_le_bytes());
    }
    out.extend(a);
    out.extend(b);
    out.extend(c);
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

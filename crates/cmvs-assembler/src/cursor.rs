//! Editable CUR rows. Hex bytes preserve indexed colors, padding and legacy inversion.
use crate::Result;
use cmvs_core::format::cursor::{CursorBitmap, CursorDib};
use std::{
    fmt::Write as _,
    io::{BufRead, Write},
};

pub fn export(raw: &[u8], mut out: impl Write) -> Result<()> {
    CursorBitmap::decode(raw)?;
    let d = CursorDib::parse(raw)?;
    writeln!(
        out,
        "cmvs-cur-v1\t{}\t{}\t{}\t{}\t{}",
        d.width, d.height, d.bits, d.hotspot.0, d.hotspot.1
    )?;
    let hex = |b: &[u8]| {
        let mut s = String::with_capacity(2 * b.len());
        for v in b {
            write!(s, "{v:02x}").unwrap();
        }
        s
    };
    writeln!(out, "palette\t{}", hex(d.palette))?;
    for (x, a) in d
        .xor
        .chunks_exact(d.xor.len() / d.height)
        .rev()
        .zip(d.and.chunks_exact(d.and.len() / d.height).rev())
    {
        writeln!(out, "row\t{}\t{}", hex(x), hex(a))?;
    }
    Ok(())
}
pub fn assemble(input: impl BufRead) -> Result<Vec<u8>> {
    fn hex(s: &str, max: usize) -> Result<Vec<u8>> {
        if s.len() > 2 * max
            || !s.len().is_multiple_of(2)
            || !s.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid CUR hex bytes".into());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
            .collect()
    }
    let mut lines = input.lines();
    let header = lines.next().ok_or("missing CUR header")??;
    let f: Vec<_> = header.split('\t').collect();
    let ["cmvs-cur-v1", w, h, bits, x, y] = f.as_slice() else {
        return Err("invalid CUR project header".into());
    };
    let (width, height, bits) = (
        w.parse::<usize>()?,
        h.parse::<usize>()?,
        bits.parse::<u16>()?,
    );
    if !(1..=256).contains(&width)
        || !(1..=256).contains(&height)
        || !matches!(bits, 4 | 8 | 24 | 32)
    {
        return Err("invalid CUR dimensions or bit depth".into());
    }
    let palette = lines.next().ok_or("missing CUR palette")??;
    let palette = hex(
        palette
            .strip_prefix("palette\t")
            .ok_or("expected CUR palette")?,
        1024,
    )?;
    let xs = (width * usize::from(bits)).div_ceil(32) * 4;
    let ms = width.div_ceil(32) * 4;
    let mut xor = vec![0; xs * height];
    let mut and = vec![0; ms * height];
    for row in (0..height).rev() {
        let line = lines.next().ok_or("missing CUR row")??;
        let fields: Vec<_> = line.split('\t').collect();
        let ["row", x, a] = fields.as_slice() else {
            return Err("expected XOR and AND row".into());
        };
        let xb = hex(x, xs)?;
        let ab = hex(a, ms)?;
        if xb.len() != xs || ab.len() != ms {
            return Err("wrong CUR row stride".into());
        }
        xor[row * xs..][..xs].copy_from_slice(&xb);
        and[row * ms..][..ms].copy_from_slice(&ab);
    }
    if lines.next().is_some() {
        return Err("unexpected extra CUR row".into());
    }
    Ok(CursorDib {
        width,
        height,
        bits,
        hotspot: (x.parse()?, y.parse()?),
        palette: &palette,
        xor: &xor,
        and: &and,
    }
    .encode()?)
}

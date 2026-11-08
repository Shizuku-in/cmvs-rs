//! Bounded byte-oriented counterpart of 0x14005ca80 (see vm-semantics.md).
use super::{LineBreakRules, TextObject};
use crate::TextEncoding;

/// 0x14005b050: 1 for a character of the hanging set (+336), 2 for the inseparable set
/// (+400), else 0. Shared by measurement and output. Under UTF-8 the sets hold UTF-8
/// characters, matched by their glyph codes.
pub(super) fn line_break_kind(
    encoding: TextEncoding,
    rules: &LineBreakRules,
    code: u16,
) -> Result<u8, &'static str> {
    if encoding == TextEncoding::Utf8 {
        for (kind, bytes) in [(1, &rules.hanging), (2, &rules.inseparable)] {
            let set =
                core::str::from_utf8(bytes).map_err(|_| "line-break set is not valid UTF-8")?;
            if set.chars().any(|ch| u32::from(ch) == u32::from(code)) {
                return Ok(kind);
            }
        }
        return Ok(0);
    }
    for (kind, bytes) in [(1, &rules.hanging), (2, &rules.inseparable)] {
        // 0x14005b050 walks two bytes at a time; an odd final byte has the NUL
        // terminator as its second byte. Its next iteration would read beyond it.
        if bytes
            .as_chunks::<2>()
            .0
            .iter()
            .any(|b| u16::from_be_bytes([b[0], b[1]]) == code)
        {
            return Ok(kind);
        }
        if bytes.len() % 2 != 0 {
            return Err("odd-length line-break set has an unbounded original scan");
        }
    }
    Ok(0)
}

pub(super) struct Layout<'a> {
    pub(super) encoding: TextEncoding,
    pub(super) text: &'a TextObject,
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) line: u32,
    pub(super) count: u32,
    pub(super) max: u32,
    pub(super) overflow: u32,
    pub(super) previous: u16,
}

impl Layout<'_> {
    pub(super) fn limit(&self) -> u32 {
        (self.text.area[0] as u32)
            .wrapping_add(self.text.area[2] as u32)
            .wrapping_sub(self.text.spacing[0] as u32)
            .wrapping_sub(self.text.glyph_size as u32)
    }

    pub(super) fn newline(&mut self, reset: bool, ruby: bool) -> Result<(), &'static str> {
        self.line += 1;
        // The original indexes the 32-word ruby-line array even when every-line is set.
        if ruby {
            let marked = self
                .text
                .ruby_lines
                .get(self.line as usize)
                .ok_or("text measurement exceeds the 32 ruby-line flags")?;
            if *marked != 0 || self.text.ruby_every_line {
                self.y = self.y.wrapping_add(self.text.ruby_size as u32);
            }
        }
        self.x = self.text.area[0] as u32;
        self.y = self
            .y
            .wrapping_add(self.text.glyph_size as u32)
            .wrapping_add(self.text.spacing[1] as u32);
        if reset {
            self.count = 0;
            self.overflow = 0;
            self.previous = 0;
        }
        Ok(())
    }

    fn classify(&self, code: u16) -> Result<u8, &'static str> {
        match &self.text.line_break {
            Some(rules) => line_break_kind(self.encoding, rules, code),
            None => Ok(0),
        }
    }

    pub(super) fn wide_wrap(&mut self, code: u16) -> Result<(), &'static str> {
        if self.text.line_break.is_some() {
            if self.x < self.limit() {
                self.previous = code;
            } else {
                let kind = self.classify(code)?;
                let allowed = match kind {
                    1 => true,
                    2 => self.previous == code,
                    _ => false,
                };
                if allowed {
                    self.overflow = self.overflow.wrapping_add(1);
                    if self.text.line_break.as_ref().expect("present").max_overflow as u32
                        >= self.overflow
                    {
                        return Ok(());
                    }
                }
                self.newline(true, true)?;
            }
        } else if self.x >= self.limit() && !self.encoding.no_break_codes().contains(&code) {
            // The exempt codes are fixed punctuation of the encoding (see
            // TextEncoding::no_break_codes). This branch increments the line number
            // without resetting the byte count.
            self.newline(false, true)?;
        }
        Ok(())
    }

    fn add(&mut self, wide: bool) {
        self.count = self.count.wrapping_add(if wide { 2 } else { 1 });
        let size = self.text.glyph_size as u32;
        self.x = self
            .x
            .wrapping_add(if wide { size } else { size >> 1 })
            .wrapping_add(self.text.spacing[0] as u32);
        self.max = self.max.max(self.count);
    }

    pub(super) fn ruby_wrap(&mut self) -> Result<(), &'static str> {
        if self.x < self.limit() {
            if self.text.line_break.is_some() {
                self.previous = 0;
            }
        } else if self.text.line_break.is_some() {
            let kind = self.classify(0)?;
            if kind == 1 || (kind == 2 && self.previous == 0) {
                self.overflow = self.overflow.wrapping_add(1);
                if self.text.line_break.as_ref().expect("present").max_overflow as u32
                    >= self.overflow
                {
                    return Ok(());
                }
            }
            self.newline(true, true)?;
        } else {
            self.newline(false, true)?;
        }
        Ok(())
    }
}

/// The glyph code of the wide character at `i`. The bytes have already been split at the
/// terminator, so only the length is checked, as in the original.
fn wide(
    encoding: TextEncoding,
    bytes: &[u8],
    i: usize,
    len: usize,
    truncated: &'static str,
) -> Result<u16, &'static str> {
    encoding.wide_code(bytes.get(i..i + len).ok_or(truncated)?)
}

pub(super) fn measure(
    encoding: TextEncoding,
    text: &TextObject,
    bytes: &[u8],
) -> Result<([u32; 3], [u32; 2]), &'static str> {
    let bytes = bytes
        .split(|byte| *byte == 0)
        .next()
        .expect("split has first item");
    let mut state = Layout {
        encoding,
        text,
        x: text.cursor[0] as u32,
        y: text.cursor[1] as u32,
        line: 0,
        count: 0,
        max: 0,
        overflow: 0,
        previous: 0,
    };
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let control = *bytes.get(i + 1).ok_or("truncated text escape")?;
                match control {
                    b'n' => state.newline(true, false)?,
                    b'c' | b't' => {}
                    b'w' => {
                        bytes.get(i + 2).ok_or("truncated text wait escape")?;
                    }
                    _ => return Err("unknown text escape would leave the original scan stuck"),
                }
                i += if control == b'w' { 3 } else { 2 };
            }
            b'{' => {
                state.ruby_wrap()?;
                i += 1;
                // Only double-byte characters in the base advance the original counter.
                loop {
                    let byte = *bytes.get(i).ok_or("ruby base is missing its slash")?;
                    if let Some(len) = encoding.wide_len(byte) {
                        wide(
                            encoding,
                            bytes,
                            i,
                            len,
                            "truncated double-byte ruby character",
                        )?;
                        i += len;
                        state.add(true);
                        continue;
                    }
                    i += 1;
                    if byte == b'/' {
                        break;
                    }
                }
                loop {
                    let byte = *bytes
                        .get(i)
                        .ok_or("ruby annotation is missing its closing brace")?;
                    if let Some(len) = encoding.wide_len(byte) {
                        wide(
                            encoding,
                            bytes,
                            i,
                            len,
                            "truncated double-byte ruby annotation",
                        )?;
                        i += len;
                        continue;
                    }
                    i += 1;
                    if byte == b'}' {
                        break;
                    }
                }
            }
            byte if encoding.is_lead_byte(byte) => {
                let len = encoding.wide_len(byte).expect("lead byte");
                let code = wide(
                    encoding,
                    bytes,
                    i,
                    len,
                    "truncated double-byte text character",
                )?;
                state.wide_wrap(code)?;
                state.add(true);
                i += len;
            }
            byte => {
                if state.x >= state.limit() {
                    state.newline(text.line_break.is_some(), true)?;
                } else if text.line_break.is_some() {
                    // The original sign-extends single-byte characters to a 16-bit word.
                    state.previous = byte as i8 as i16 as u16;
                }
                state.add(false);
                i += 1;
            }
        }
    }
    Ok((
        [state.max, state.line + 1, state.count],
        [state.overflow, u32::from(state.previous)],
    ))
}

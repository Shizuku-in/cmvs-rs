//! Bounded byte-oriented counterpart of 0x14005ca80 (see vm-semantics.md).
use super::{LineBreakRules, TextObject};

/// 0x14005b050: 1 for a character of the hanging set (+336), 2 for the inseparable set
/// (+400), else 0. Shared by measurement and output.
pub(super) fn line_break_kind(rules: &LineBreakRules, code: u16) -> Result<u8, &'static str> {
    for (kind, bytes) in [(1, &rules.hanging), (2, &rules.inseparable)] {
        // 0x14005b050 walks two bytes at a time; an odd final byte has the NUL
        // terminator as its second byte. Its next iteration would read beyond it.
        if bytes
            .chunks_exact(2)
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

struct Layout<'a> {
    text: &'a TextObject,
    x: u32,
    y: u32,
    line: u32,
    count: u32,
    max: u32,
    overflow: u32,
    previous: u16,
}

/// 0x140008080: Shift-JIS lead bytes, without checking the trail byte.
pub(super) fn lead(byte: u8) -> bool {
    (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte)
}

impl Layout<'_> {
    fn limit(&self) -> u32 {
        (self.text.area[0] as u32)
            .wrapping_add(self.text.area[2] as u32)
            .wrapping_sub(self.text.spacing[0] as u32)
            .wrapping_sub(self.text.glyph_size as u32)
    }

    fn newline(&mut self, reset: bool, ruby: bool) -> Result<(), &'static str> {
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
            Some(rules) => line_break_kind(rules, code),
            None => Ok(0),
        }
    }

    fn wide_wrap(&mut self, code: u16) -> Result<(), &'static str> {
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
        } else if self.x >= self.limit() && !matches!(code, 0x8141 | 0x8142 | 0x8176 | 0x8178) {
            // This branch increments the line number without resetting the byte count.
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

    fn ruby_wrap(&mut self) -> Result<(), &'static str> {
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

pub(super) fn measure(
    text: &TextObject,
    bytes: &[u8],
) -> Result<([u32; 3], [u32; 2]), &'static str> {
    let bytes = bytes
        .split(|byte| *byte == 0)
        .next()
        .expect("split has first item");
    let mut state = Layout {
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
                    i += 1;
                    if lead(byte) {
                        bytes.get(i).ok_or("truncated double-byte ruby character")?;
                        i += 1;
                        state.add(true);
                    } else if byte == b'/' {
                        break;
                    }
                }
                loop {
                    let byte = *bytes
                        .get(i)
                        .ok_or("ruby annotation is missing its closing brace")?;
                    i += 1;
                    if lead(byte) {
                        bytes
                            .get(i)
                            .ok_or("truncated double-byte ruby annotation")?;
                        i += 1;
                    } else if byte == b'}' {
                        break;
                    }
                }
            }
            byte if lead(byte) => {
                let second = *bytes
                    .get(i + 1)
                    .ok_or("truncated double-byte text character")?;
                state.wide_wrap(u16::from_be_bytes([byte, second]))?;
                state.add(true);
                i += 2;
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

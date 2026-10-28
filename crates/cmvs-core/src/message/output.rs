//! Explicit recording of 0x2152 output for plain characters, ruby groups and newlines,
//! including automatic wrapping. These records are not glyph textures and must not imply successful
//! font rendering.
use super::{
    MessageHost, TextObject,
    measure::{lead, line_break_kind},
    window_slot,
};
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    input::ActionInput,
    machine::{Machine, Step},
    message_flags::MessageFlags,
    vm::SystemResult,
};

/// Logical glyph descriptor and reveal state; no device objects or font pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedGlyph {
    pub code: u16,
    pub position: [i32; 2],
    pub size: u32,
    pub colors: [u32; 2],
    /// Descriptor word 2, mapped from text +132 (0..3; other values map to zero).
    pub gradient: u32,
    /// Descriptor words 8/9: settings +1528 and text +128, respectively.
    pub outline: [u32; 2],
    pub delay_ms: u32,
    pub elapsed_ms: u32,
    pub duration_ms: u32,
    pub active: bool,
    pub shown: bool,
    pub opacity: u32,
}
impl RecordedGlyph {
    pub(super) fn advance(&mut self, delta: u32) {
        if !self.active {
            return;
        }
        let available = delta.saturating_sub(self.delay_ms);
        self.delay_ms = self.delay_ms.saturating_sub(delta);
        if available == 0 {
            return;
        }
        self.shown = true;
        self.elapsed_ms = self.elapsed_ms.wrapping_add(available);
        if self.elapsed_ms >= self.duration_ms {
            self.finish();
        } else {
            self.opacity = 255u32.wrapping_mul(self.elapsed_ms) / self.duration_ms;
        }
    }
    fn finish(&mut self) {
        self.active = false;
        self.delay_ms = 0;
        self.shown = true;
        self.opacity = 255;
    }
}

/// Line feed of 0x14005bc90 and 0x14005b120: the new line reserves the ruby height when it is
/// marked or every line reserves it, then the pen returns to the area's left edge. The
/// original reads past the 32 ruby-line flags from line 32 on.
fn new_line(
    text: &TextObject,
    line: &mut u32,
    x: &mut u32,
    y: &mut u32,
) -> Result<(), &'static str> {
    *line += 1;
    let marked = *text
        .ruby_lines
        .get(*line as usize)
        .ok_or("message output exceeds the 32 ruby-line flags")?;
    if marked != 0 || text.ruby_every_line {
        *y = y.wrapping_add(text.ruby_size as u32);
    }
    *x = text.area[0] as u32;
    *y = y
        .wrapping_add(text.glyph_size as u32)
        .wrapping_add(text.spacing[1] as u32);
    Ok(())
}

const ESCAPES: &str = "recorded message output does not yet support this text escape";

/// Line end shared by the layout functions: area x + width − spacing x − glyph size.
fn line_end(text: &TextObject) -> u32 {
    (text.area[0] as u32)
        .wrapping_add(text.area[2] as u32)
        .wrapping_sub(text.spacing[0] as u32)
        .wrapping_sub(text.glyph_size as u32)
}

/// One `{base/ruby}` group as 0x14005bc90 walks it: each loop iteration is a double-byte
/// character (Some) or a single byte (None), the closing `/` or `}` included.
struct Group {
    base: Vec<Option<u16>>,
    ruby: Vec<Option<u16>>,
    /// Index just after the closing brace.
    end: usize,
}

/// Parses the group whose `{` precedes `start`. The original reads past a terminator
/// instead of stopping, so a missing `/` or `}` is an error.
fn parse_group(bytes: &[u8], start: usize) -> Result<Group, &'static str> {
    fn part(
        bytes: &[u8],
        i: &mut usize,
        close: u8,
        missing: &'static str,
    ) -> Result<Vec<Option<u16>>, &'static str> {
        let mut items = Vec::new();
        loop {
            let byte = *bytes.get(*i).filter(|&&b| b != 0).ok_or(missing)?;
            if lead(byte) {
                let second = *bytes
                    .get(*i + 1)
                    .filter(|&&b| b != 0)
                    .ok_or("truncated double-byte message character")?;
                *i += 2;
                items.push(Some(u16::from_be_bytes([byte, second])));
            } else {
                *i += 1;
                items.push(None);
                if byte == close {
                    return Ok(items);
                }
            }
        }
    }
    let mut i = start;
    let base = part(bytes, &mut i, b'/', "ruby base is missing its slash")?;
    let ruby = part(
        bytes,
        &mut i,
        b'}',
        "ruby annotation is missing its closing brace",
    )?;
    Ok(Group { base, ruby, end: i })
}

/// 0x14005b360 (ruby layout only): from the pen, marks each line on which a group starts.
/// Its own wrapping ignores 0x211e rules but keeps the +112 punctuation exception, so the
/// marks can differ from the actual layout, as in the original.
fn ruby_lines(text: &TextObject, bytes: &[u8]) -> Result<[u32; 32], &'static str> {
    let limit = line_end(text);
    let size = text.glyph_size as u32;
    let spacing = text.spacing[0] as u32;
    let mut lines = [0; 32];
    let (mut x, mut line) = (text.cursor[0] as u32, 0usize);
    let mut i = 0;
    while i < bytes.len() && bytes[i] != 0 {
        let byte = bytes[i];
        if byte == b'\\' {
            if bytes.get(i + 1) != Some(&b'n') {
                return Err(ESCAPES);
            }
            // 0x14005b547..0x14005b552: explicit newline, without a glyph advance.
            x = text.area[0] as u32;
            line += 1;
            i += 2;
            continue;
        }
        let (advance, wrap) = if byte == b'{' {
            if x >= limit {
                (x, line) = (text.area[0] as u32, line + 1);
            }
            // The original writes past the 32 flags from line 32 on.
            *lines
                .get_mut(line)
                .ok_or("ruby group beyond the 32 ruby-line flags")? = 1;
            let group = parse_group(bytes, i + 1)?;
            for _ in group.base.iter().flatten() {
                x = x.wrapping_add(size).wrapping_add(spacing);
            }
            i = group.end;
            continue;
        } else if lead(byte) {
            let second = *bytes
                .get(i + 1)
                .filter(|&&b| b != 0)
                .ok_or("truncated double-byte message character")?;
            i += 2;
            let code = u16::from_be_bytes([byte, second]);
            (size, !matches!(code, 0x8141 | 0x8142 | 0x8176 | 0x8178))
        } else {
            i += 1;
            (size >> 1, true)
        };
        if wrap && x >= limit {
            (x, line) = (text.area[0] as u32, line + 1);
        }
        x = x.wrapping_add(advance).wrapping_add(spacing);
    }
    Ok(lines)
}

/// Wrap decision for a double-byte character, or for a ruby group as code 0: inline in
/// 0x14005bc90 without rules (the +112 punctuation exception), 0x14005b120 (a2 = 0) with
/// them. Past the line end, +528 counts hanging characters and repeats of an inseparable
/// one; past the unsigned limit they wrap as well.
fn wraps(text: &mut TextObject, code: u16, x: u32, limit: u32) -> Result<bool, &'static str> {
    Ok(match &text.line_break {
        None => x >= limit && !matches!(code, 0x8141 | 0x8142 | 0x8176 | 0x8178),
        Some(_) if x < limit => {
            text.layout_scratch[1] = u32::from(code);
            false
        }
        Some(rules) => {
            let overflow = match line_break_kind(rules, code)? {
                1 => true,
                2 => text.layout_scratch[1] == u32::from(code),
                _ => false,
            };
            if overflow {
                text.layout_scratch[0] = text.layout_scratch[0].wrapping_add(1);
            }
            !overflow || (rules.max_overflow as u32) < text.layout_scratch[0]
        }
    })
}

/// 0x14005a830: a glyph shown at once; 0x14005b5a0 may then delay its reveal.
fn push(
    text: &mut TextObject,
    code: u16,
    [x, y]: [u32; 2],
    size: u32,
) -> Result<usize, &'static str> {
    if text.glyphs.len() >= 4096 {
        return Err("recorded message glyph limit exceeded");
    }
    text.glyphs.push(RecordedGlyph {
        code,
        position: [x as i32, y as i32],
        size,
        colors: [text.color, text.decoration_color],
        gradient: match text.decoration {
            1..=3 => text.decoration as u32,
            _ => 0,
        },
        outline: [1, text.glyph_parameter],
        delay_ms: 0,
        elapsed_ms: 0,
        duration_ms: 350,
        active: false,
        shown: true,
        opacity: 255,
    });
    Ok(text.glyphs.len() - 1)
}

/// 0x14005b5a0 with the constructor settings (alpha reveal, 350 ms) and no `\wc`: a nonzero
/// interval hides the glyph until `delay` has passed and returns delay + interval.
fn retime(
    text: &mut TextObject,
    glyph: Option<usize>,
    delay: u32,
    interval: u32,
) -> Result<u32, &'static str> {
    if interval == 0 {
        return Ok(delay);
    }
    let glyph = glyph.and_then(|index| text.glyphs.get_mut(index)).ok_or(
        "ruby markup before any glyph re-times the original's uninitialized glyph pointer",
    )?;
    glyph.delay_ms = delay;
    glyph.elapsed_ms = 0;
    glyph.active = true;
    glyph.shown = false;
    glyph.opacity = 0;
    Ok(delay.wrapping_add(interval))
}

/// Settings block constructor: +1544=4, +1536=1 (alpha reveal), +1540=2 (350ms),
/// +1528=1. READ_SKIP and script skip initially zero. Other modes remain barriers.
/// Wrapping follows 0x14005bc90 with +112 = 1 (constructor and 0x14005d3b0; only the
/// unimplemented 0x2108 clears it). Without line-break rules a double-byte 、。」』 never
/// starts a line; with rules, double-byte characters go through 0x14005b120.
///
/// A `{base/ruby}` group wraps as a whole (as code 0). Its double-byte base characters
/// advance the pen; with ruby enabled, its double-byte annotations use the ruby size on the
/// pen line minus the ruby size, spread in unsigned 22.10 fixed point: with the annotation
/// no wider than the base, from base start + gap/(2n) by ruby size + 2·gap/(2n) + ruby
/// spacing (added unscaled), otherwise centred at the ruby size. Single bytes draw nothing.
/// Every group loop iteration re-times the last glyph through 0x14005b5a0, so `/`, `}` and
/// single bytes delay the previous glyph again and add an interval; annotations start at
/// the group's delay and leave the following text's delay unchanged.
fn append(text: &mut TextObject, bytes: &[u8], skip: bool) -> Result<(), &'static str> {
    let mut x = text.cursor[0] as u32;
    let mut y = text.cursor[1] as u32;
    let mut line = 0u32;
    let size = text.glyph_size as u32;
    let limit = line_end(text);
    text.ruby_lines = if text.ruby {
        ruby_lines(text, bytes)?
    } else {
        [0; 32]
    };
    if text.ruby && (text.ruby_lines[0] != 0 || text.ruby_every_line) {
        y = y.wrapping_add(text.ruby_size as u32);
    }
    text.layout_scratch = [0; 2];
    let interval = if skip {
        0
    } else {
        4u32.wrapping_mul(text.char_delay as u32) / 10
    };
    let mut delay = 0u32;
    // 0x14005bc90's last-glyph pointer is uninitialized at the start of every call.
    let mut last = None;
    let mut i = 0;
    while i < bytes.len() && bytes[i] != 0 {
        let byte = bytes[i];
        if byte == b'\\' {
            if bytes.get(i + 1) != Some(&b'n') {
                return Err(ESCAPES);
            }
            // 0x14005c46a..0x14005c4b8: no glyph or reveal interval for the newline.
            new_line(text, &mut line, &mut x, &mut y)?;
            text.layout_scratch = [0; 2];
            i += 2;
            continue;
        }
        if byte == b'{' {
            let group = parse_group(bytes, i + 1)?;
            i = group.end;
            if wraps(text, 0, x, limit)? {
                new_line(text, &mut line, &mut x, &mut y)?;
                if text.line_break.is_some() {
                    text.layout_scratch = [0; 2];
                }
            }
            let (start, ruby_y, group_delay) = (x, y.wrapping_sub(text.ruby_size as u32), delay);
            for item in &group.base {
                if let Some(code) = *item {
                    last = Some(push(text, code, [x, y], size)?);
                    x = x.wrapping_add(size).wrapping_add(text.spacing[0] as u32);
                }
                delay = retime(text, last, delay, interval)?;
            }
            if !text.ruby {
                continue;
            }
            let ruby_size = text.ruby_size as u32;
            let count = group.ruby.iter().flatten().count() as u32;
            let width = x.wrapping_sub(start) << 10;
            let total = count.wrapping_mul(ruby_size) << 10;
            let (mut ruby_x, step) = if total <= width {
                let gap = (width - total).checked_div(count.wrapping_mul(2)).ok_or(
                    "ruby annotation without double-byte characters divides by zero in the original",
                )?;
                let step = (ruby_size << 9).wrapping_add(gap).wrapping_mul(2);
                (
                    (start << 10).wrapping_add(gap),
                    step.wrapping_add(text.ruby_spacing as u32),
                )
            } else {
                (
                    (start << 10).wrapping_sub((total - width) >> 1),
                    ruby_size << 10,
                )
            };
            let mut ruby_delay = group_delay;
            for item in &group.ruby {
                if let Some(code) = *item {
                    last = Some(push(text, code, [ruby_x >> 10, ruby_y], ruby_size)?);
                    ruby_x = ruby_x.wrapping_add(step);
                }
                ruby_delay = retime(text, last, ruby_delay, interval)?;
            }
            continue;
        }
        let wide = lead(byte);
        let (code, wrap) = if wide {
            let second = *bytes
                .get(i + 1)
                .filter(|&&b| b != 0)
                .ok_or("truncated double-byte message character")?;
            i += 2;
            let code = u16::from_be_bytes([byte, second]);
            (code, wraps(text, code, x, limit)?)
        } else {
            i += 1;
            // Sign-extended to the 16-bit character word.
            let code = byte as i8 as i16 as u16;
            if text.line_break.is_some() && x < limit {
                text.layout_scratch[1] = u32::from(code);
            }
            (code, x >= limit)
        };
        if wrap {
            new_line(text, &mut line, &mut x, &mut y)?;
            if text.line_break.is_some() {
                text.layout_scratch = [0; 2];
            }
        }
        let gx = if code == 0x814a {
            x.wrapping_sub(size / 5)
        } else {
            x
        };
        last = Some(push(text, code, [gx, y], size)?);
        delay = retime(text, last, delay, interval)?;
        let advance = if code == 0x814a {
            (size >> 1).wrapping_sub(size / 5)
        } else if wide {
            size
        } else {
            size >> 1
        };
        x = x.wrapping_add(advance).wrapping_add(text.spacing[0] as u32);
    }
    text.cursor = [x as i32, y as i32];
    Ok(())
}

impl MessageHost {
    /// 0x140076700 / 0x14005d900: repeat=0 creates glyphs once; hidden windows keep
    /// yielding. Waiting returns 0xa000, completion 0xc00c (both yield). Research mode
    /// explicitly selects the constructor settings block. Only the +1520 skip flag shortens
    /// reveal timing; it, held actions 7/8 and the +12244 bypass finish without consuming.
    /// A release of action 0 or 20 finishes and consumes both (0x140054d90, 0x140055160).
    pub fn try_record_output(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x152)) = vm.pending_system() else {
            return Ok(None);
        };
        if !self.record_output {
            return Ok(None);
        }
        let slot = window_slot(vm, pc, 0x2152)?;
        // The read ID is resolved even with READ_SKIP=0, to validate all 12 argument bytes.
        let _read_id = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x2152, e.0))? as i32;
        let raw = vm
            .stack()
            .peek_u32(12)
            .map_err(|e| error(pc, 0x2152, e.0))?;
        let bytes = vm.resolve_string(raw)?;
        let window = self
            .window(slot)
            .ok_or_else(|| error(pc, 0x2152, "missing message window sets an unmodeled flag"))?;
        let index = window.text;
        let mut candidate = self
            .text(index)
            .ok_or_else(|| error(pc, 0x2152, "message window without text"))?
            .clone();
        if vm.repeat_count() == 0 {
            append(&mut candidate, &bytes, flags.skip()).map_err(|why| error(pc, 0x2152, why))?;
        }
        let active = candidate.glyphs.iter().any(|g| g.active);
        let skip = flags.skip() || input.skip_held() || flags.bypass();
        let release = input.released(0) || input.released(20);
        let done = !window.hidden && (!active || skip || release);
        let consume = done && active && !skip && release;
        let step = if done {
            vm.complete_system(SystemResult(0xc00c))?
        } else {
            vm.yield_pending_system()?
        };
        if done && active {
            for glyph in &mut candidate.glyphs {
                glyph.finish();
            }
        }
        self.texts[index] = Some(candidate);
        if consume {
            input.consume(0);
            input.consume(20);
        }
        Ok(Some(step))
    }
}

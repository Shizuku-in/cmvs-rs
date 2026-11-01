//! Font-cell text semantics, independently selected by the dispatch table.
//! Evidence is bound to cmvs-20171114 in docs/games/aoitori/vm-semantics.md.
use super::{
    MessageHost, TextObject,
    auxiliary::{AuxiliaryGlyph, AuxiliaryText},
    auxiliary_layout::TextLayout,
    measure::{Layout, lead},
    output::{escape_w, line_end, new_line, parse_group, push, retime, ruby_layout, wraps},
};
use crate::bytecode::DecodeError;
#[derive(Debug, Default)]
pub(super) struct State {
    proportional: bool,
}

const NO_BREAK: [u16; 4] = [0x8141, 0x8142, 0x8176, 0x8178];
const VOICED_MARK: u16 = 0x814a;
const ESCAPES: &str = "text escape that does not advance in the original";
fn append(
    text: &mut TextObject,
    bytes: &[u8],
    skip: bool,
    settings: crate::settings::TextOutputSettings,
    proportional: bool,
    metrics: Option<&dyn super::GlyphMetrics>,
) -> Result<(), &'static str> {
    let mut x = text.cursor[0] as u32;
    let mut y = text.cursor[1] as u32;
    let mut line = 0u32;
    let size = text.glyph_size as u32;
    let limit = line_end(text);
    text.ruby_lines = if text.ruby {
        ruby_layout(text, bytes)?.0
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
        settings.speed.wrapping_mul(text.char_delay as u32) / 10
    };
    let mut delay = 0u32;
    // `\wc` / `\ws` (v83): reveal glyphs at once until switched back; starts off per call.
    let mut instant = false;
    // 0x14005bc90's last-glyph pointer is uninitialized at the start of every call.
    let mut last = None;
    let mut i = 0;
    while i < bytes.len() && bytes[i] != 0 {
        let byte = bytes[i];
        if byte == b'\\' {
            match bytes.get(i + 1) {
                // 0x14005c46a..0x14005c4b8: no glyph or reveal interval for the newline.
                Some(b'n') => {
                    new_line(text, &mut line, &mut x, &mut y)?;
                    text.layout_scratch = [0; 2];
                    i += 2;
                }
                // `\c` (as 0x14005ad50): drop the glyphs and return the pen to the area
                // origin; the line count and ruby marks stay. The original's last-glyph
                // pointer then refers to a freed glyph, so it is forgotten here.
                Some(b'c') => {
                    text.glyphs.clear();
                    last = None;
                    x = text.area[0] as u32;
                    y = text.area[1] as u32;
                    i += 2;
                }
                // `\t` restarts the reveal delay (v104).
                Some(b't') => {
                    delay = 0;
                    i += 2;
                }
                Some(b'w') => {
                    let (length, selector) = escape_w(bytes, i)?;
                    if let Some(value) = selector {
                        instant = value;
                        if value {
                            delay = 0;
                        }
                    }
                    i += length;
                }
                _ => return Err(ESCAPES),
            }
            continue;
        }
        if byte == b'{' {
            if proportional && text.ruby {
                return Err("proportional ruby annotation layout is not implemented");
            }
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
                    last = Some(push(text, code, [x, y], size, settings.outline)?);
                    let width = output_width(text, code, size, proportional, metrics)?;
                    x = x.wrapping_add(width).wrapping_add(text.spacing[0] as u32);
                }
                delay = retime(text, last, delay, interval, instant, settings)?;
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
                    last = Some(push(
                        text,
                        code,
                        [ruby_x >> 10, ruby_y],
                        ruby_size,
                        settings.outline,
                    )?);
                    ruby_x = ruby_x.wrapping_add(step);
                }
                ruby_delay = retime(text, last, ruby_delay, interval, instant, settings)?;
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
        last = Some(push(text, code, [gx, y], size, settings.outline)?);
        delay = retime(text, last, delay, interval, instant, settings)?;
        let advance = if code == 0x814a {
            (size >> 1).wrapping_sub(size / 5)
        } else if proportional {
            output_width(text, code, size, true, metrics)?
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

fn output_width(
    text: &TextObject,
    code: u16,
    size: u32,
    proportional: bool,
    metrics: Option<&dyn super::GlyphMetrics>,
) -> Result<u32, &'static str> {
    if proportional {
        metrics
            .ok_or("proportional text requires a font metrics backend")?
            .advance_width(&text.face, code, size, text.bold)
    } else {
        Ok(size)
    }
}

fn place(
    text: &mut TextObject,
    bytes: &[u8],
    metrics: Option<&dyn super::GlyphMetrics>,
    outline: Option<u32>,
    proportional: bool,
) -> Result<(), &'static str> {
    text.ruby_lines = [0; 32];
    text.layout_scratch = [0; 2];
    let size = text.glyph_size as u32;
    let limit = line_end(text);
    let mode = outline.unwrap_or(text.outline_mode);
    let (mut x, mut y) = (text.cursor[0] as u32, text.cursor[1] as u32);
    let mut line = 0u32;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i).filter(|&&b| b != 0) {
        if byte == b'\\' {
            match bytes.get(i + 1) {
                Some(b'c' | b't') => i += 2,
                Some(b'n') => {
                    x = text.area[0] as u32;
                    y = y.wrapping_add(size).wrapping_add(text.spacing[1] as u32);
                    i += 2;
                }
                Some(b'w') if bytes.get(i + 2).is_some_and(|&b| b != 0) => i += 3,
                _ => return Err("direct text output escape would not advance in the original"),
            }
            continue;
        }
        if byte == b'{' {
            if text.line_break.is_some() {
                return Err("direct ruby groups under line-break rules are not modeled");
            }
            let group = parse_group(bytes, i + 1)?;
            if proportional && x >= limit {
                new_line(text, &mut line, &mut x, &mut y)?;
            }
            for &code in group.base.iter().flatten() {
                if !proportional && wraps(text, code, x, limit)? {
                    new_line(text, &mut line, &mut x, &mut y)?;
                }
                push(text, code, [x, y], size, mode)?;
                let width = output_width(text, code, size, proportional, metrics)?;
                x = x.wrapping_add(width).wrapping_add(text.spacing[0] as u32);
            }
            i = group.end;
            continue;
        }
        if !lead(byte) {
            let metrics =
                metrics.ok_or("direct single-byte text requires a font metrics backend")?;
            let code = (byte as i8 as i16) as u16;
            let width = if proportional {
                metrics.advance_width(&text.face, code, size, text.bold)?
            } else {
                metrics
                    .styled_ink_width(&text.face, code, size, text.bold)?
                    .max(size >> 1)
            };
            if x >= limit {
                new_line(text, &mut line, &mut x, &mut y)?;
                text.layout_scratch = [0; 2];
            } else if text.line_break.is_some() {
                text.layout_scratch[1] = u32::from(code);
            }
            push(text, code, [x, y], size, mode)?;
            x = x.wrapping_add(width).wrapping_add(text.spacing[0] as u32);
            i += 1;
            continue;
        }
        let second = *bytes
            .get(i + 1)
            .filter(|&&b| b != 0)
            .ok_or("truncated double-byte message character")?;
        i += 2;
        let code = u16::from_be_bytes([byte, second]);
        if wraps(text, code, x, limit)? {
            new_line(text, &mut line, &mut x, &mut y)?;
            if text.line_break.is_some() {
                text.layout_scratch = [0; 2];
            }
        }
        let (gx, advance) = if code == 0x814a {
            (x.wrapping_sub(size / 5), (size >> 1).wrapping_sub(size / 5))
        } else {
            (x, output_width(text, code, size, proportional, metrics)?)
        };
        push(text, code, [gx, y], size, mode)?;
        x = x.wrapping_add(advance).wrapping_add(text.spacing[0] as u32);
    }
    text.cursor = [x as i32, y as i32];
    Ok(())
}

/// Aoitori 0x140062aa0: pixel-width query, retaining the original wrap counters
/// and scratch effects while restoring the caller's pen. Ruby annotations are skipped.
pub(super) fn measure_pixels(
    text: &TextObject,
    bytes: &[u8],
    proportional: bool,
    metrics: Option<&dyn super::GlyphMetrics>,
) -> Result<([u32; 3], [u32; 2]), &'static str> {
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
    let add = |state: &mut Layout<'_>, code: u16| -> Result<(), &'static str> {
        let size = text.glyph_size as u32;
        let width = if proportional {
            metrics
                .ok_or("proportional text requires a font metrics backend")?
                .advance_width(&text.face, code, size, text.bold)?
        } else if code > 0xff && code < 0xff80 {
            size
        } else {
            metrics
                .ok_or("pixel text measurement requires a font metrics backend")?
                .styled_ink_width(&text.face, code, size, text.bold)?
        };
        let advance = width.wrapping_add(text.spacing[0] as u32);
        state.x = state.x.wrapping_add(advance);
        state.count = state.count.wrapping_add(advance);
        state.max = state.max.max(state.count);
        Ok(())
    };
    let mut i = 0;
    while let Some(&byte) = bytes.get(i).filter(|&&b| b != 0) {
        if byte == b'\\' {
            match bytes.get(i + 1) {
                Some(b'n') => state.newline(true, false)?,
                Some(b'c' | b't') => {}
                Some(b'w') if bytes.get(i + 2).is_some_and(|&b| b != 0) => {
                    i += 3;
                    continue;
                }
                Some(&code @ (b'"' | b'%' | b'@' | b'{' | b'}')) => {
                    if state.x >= state.limit() {
                        state.newline(text.line_break.is_some(), true)?;
                    } else if text.line_break.is_some() {
                        state.previous = u16::from(if code == b'@' { b'\\' } else { code });
                    }
                    add(
                        &mut state,
                        u16::from(if code == b'@' { b'\\' } else { code }),
                    )?;
                }
                _ => return Err("pixel text measurement escape would not advance in the original"),
            }
            i += 2;
        } else if byte == b'{' {
            state.ruby_wrap()?;
            i += 1;
            for close in *b"/}" {
                loop {
                    let first = *bytes
                        .get(i)
                        .filter(|&&b| b != 0)
                        .ok_or("unterminated measured ruby group")?;
                    i += 1;
                    if lead(first) {
                        let second = *bytes
                            .get(i)
                            .filter(|&&b| b != 0)
                            .ok_or("truncated measured ruby character")?;
                        i += 1;
                        if close == b'/' {
                            add(&mut state, u16::from_be_bytes([first, second]))?;
                        }
                    } else if first == close {
                        break;
                    }
                }
            }
        } else if lead(byte) {
            let second = *bytes
                .get(i + 1)
                .filter(|&&b| b != 0)
                .ok_or("truncated measured text character")?;
            let code = u16::from_be_bytes([byte, second]);
            state.wide_wrap(code)?;
            add(&mut state, code)?;
            i += 2;
        } else {
            let code = byte as i8 as i16 as u16;
            if state.x >= state.limit() {
                state.newline(text.line_break.is_some(), true)?;
            } else if text.line_break.is_some() {
                state.previous = code;
            }
            add(&mut state, code)?;
            i += 1;
        }
    }
    Ok((
        [state.max, state.line + 1, state.count],
        [state.overflow, u32::from(state.previous)],
    ))
}
impl AuxiliaryText {
    fn float_font_cells(&self, at: u16) -> f32 {
        f32::from_bits(self.word(at).expect("modeled field"))
    }
    fn at_edge_font_cells(&self, x: f32) -> bool {
        x >= (self.float_font_cells(16) + self.float_font_cells(24))
            - (self.word(48).unwrap() as i32 as f32 + self.float_font_cells(52))
    }
    pub(super) fn layout_font_cells(
        &self,
        text: &[u8],
        proportional: bool,
        metrics: Option<&dyn super::GlyphMetrics>,
    ) -> Result<TextLayout, &'static str> {
        let byte = |i: usize| text.get(i).copied().unwrap_or(0);
        let size = self.word(48).expect("modeled field");
        let size_f = size as i32 as f32;
        let width = |code| -> Result<f32, &'static str> {
            if proportional {
                Ok(metrics
                    .ok_or("proportional text requires a font metrics backend")?
                    .advance_width(&self.face, code, size, false)? as f32)
            } else {
                Ok(size_f)
            }
        };
        let gradient = match self.word(148).expect("modeled field") {
            value @ 1..=3 => value,
            _ => 0,
        };
        let template = AuxiliaryGlyph {
            face: self.face.clone(),
            code: 0,
            position: [0, 0],
            size,
            colors: [
                self.word(132).expect("modeled field"),
                self.word(136).expect("modeled field"),
            ],
            gradient,
            outline: [
                self.word(140).expect("modeled field"),
                self.word(144).expect("modeled field"),
            ],
            cell: 0,
            layer: 0,
        };
        let no_break = self.word(128).expect("modeled field") != 0;
        let (mut x, mut y) = (self.float_font_cells(176), self.float_font_cells(180));
        let mut line = 0u32;
        let mut glyphs = Vec::new();
        // 0x14002f581 / 0x14002f6c9 / 0x14002f877: a line feed after the wrap test.
        let wrap = |x: &mut f32, y: &mut f32, line: &mut u32| -> Result<(), &'static str> {
            *line += 1;
            if *line >= 32 {
                return Err("auxiliary text exceeds the 32 cleared ruby-line flags");
            }
            if self.word(200).expect("modeled field") != 0 {
                *y += self.word(204).expect("modeled field") as i32 as f32;
            }
            *x = self.float_font_cells(16);
            *y += size_f + self.float_font_cells(56);
            Ok(())
        };
        let truncate = |v: f32| {
            if (-2147483648.0..2147483648.0).contains(&v) {
                v as i32
            } else {
                i32::MIN
            }
        };
        let mut i = 0;
        while byte(i) != 0 {
            let first = byte(i);
            if first == b'\\' {
                match byte(i + 1) {
                    b'c' | b't' => i += 2,
                    b'n' => {
                        // Raw +16 copy; neither the line counter nor the ruby height moves.
                        x = self.float_font_cells(16);
                        y += size_f + self.float_font_cells(56);
                        i += 2;
                    }
                    b'w' => i += 3,
                    _ => return Err("unknown auxiliary text escape loops forever"),
                }
                if i > text.len() {
                    return Err("auxiliary text escape reads past the terminator");
                }
                continue;
            }
            if first == b'{' {
                // The newer proportional path wraps the group as a whole, unlike
                // the root build's per-character auxiliary ruby-base layout.
                if proportional && self.at_edge_font_cells(x) {
                    wrap(&mut x, &mut y, &mut line)?;
                }
                i += 1;
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        let code = u16::from_be_bytes([b, byte(i + 1)]);
                        i += 2;
                        if !proportional
                            && self.at_edge_font_cells(x)
                            && !(no_break && NO_BREAK.contains(&code))
                        {
                            wrap(&mut x, &mut y, &mut line)?;
                        }
                        glyphs.push(AuxiliaryGlyph {
                            code,
                            position: [truncate(x), truncate(y)],
                            ..template.clone()
                        });
                        x += width(code)? + self.float_font_cells(52);
                    } else {
                        i += 1;
                        if b == b'/' {
                            break;
                        }
                    }
                }
                loop {
                    let b = byte(i);
                    if b == 0 {
                        return Err("unterminated auxiliary ruby group");
                    }
                    if lead(b) {
                        i += 2;
                    } else {
                        i += 1;
                        if b == b'}' {
                            break;
                        }
                    }
                }
                continue;
            }
            if !lead(first) {
                if proportional {
                    if self.at_edge_font_cells(x) {
                        wrap(&mut x, &mut y, &mut line)?;
                    }
                    let code = first as i8 as i16 as u16;
                    glyphs.push(AuxiliaryGlyph {
                        code,
                        position: [truncate(x), truncate(y)],
                        ..template.clone()
                    });
                    x += width(code)? + self.float_font_cells(52);
                    i += 1;
                    continue;
                }
                // 0x14002f7a2 compares with a word the caller never stored (r8's home slot).
                return Err("half-width auxiliary text advance reads an uninitialized stack word");
            }
            let code = u16::from_be_bytes([first, byte(i + 1)]);
            i += 2;
            if self.at_edge_font_cells(x) && !(no_break && NO_BREAK.contains(&code)) {
                wrap(&mut x, &mut y, &mut line)?;
            }
            let mut position = [truncate(x), truncate(y)];
            if code == VOICED_MARK {
                position[0] = (position[0] as u32).wrapping_sub(size / 5) as i32;
                x += (size_f * 0.5 - size_f / 5.0) + self.float_font_cells(52);
            } else {
                x += width(code)? + self.float_font_cells(52);
            }
            glyphs.push(AuxiliaryGlyph {
                code,
                position,
                ..template.clone()
            });
        }
        Ok(TextLayout {
            glyphs,
            pen: [x, y],
        })
    }
}

fn append_layout(
    host: &MessageHost,
    text: &mut TextObject,
    bytes: &[u8],
    skip: bool,
) -> Result<(), &'static str> {
    append(
        text,
        bytes,
        skip,
        host.output_settings,
        host.font_cells.proportional,
        host.glyph_metrics.as_deref(),
    )
}
fn place_layout(
    host: &MessageHost,
    text: &mut TextObject,
    bytes: &[u8],
    outline: Option<u32>,
) -> Result<(), &'static str> {
    place(
        text,
        bytes,
        host.glyph_metrics.as_deref(),
        outline,
        host.font_cells.proportional,
    )
}
use crate::dispatch::{Call, Flow, TextHosts};
pub(crate) fn set_mode(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    let raw = call.argument(4, "enabled")?;
    call.complete(0x4004)?;
    hosts.messages.font_cells.proportional = raw != 0;
    Ok(Flow::Done)
}
pub(crate) fn measure_text(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    let index = call.argument(4, "text slot")? as usize;
    let raw = call.argument(8, "text")?;
    let bytes = call.vm.resolve_string(raw)?;
    let host = &mut hosts.messages;
    let mut text = host
        .text(index)
        .ok_or_else(|| {
            call.error("pixel measurement needs an existing text object in slots 0..11")
        })?
        .clone();
    text.ruby_lines = [0; 32];
    let (counts, scratch) = measure_pixels(
        &text,
        &bytes,
        host.font_cells.proportional,
        host.glyph_metrics.as_deref(),
    )
    .map_err(|why| call.error(why))?;
    call.complete(0x4008)?;
    let target = host.texts[index].as_mut().expect("validated text");
    target.ruby_lines = [0; 32];
    target.layout_scratch = scratch;
    for (slot, value) in [(0, 1), (1, counts[0]), (2, counts[1]), (4, counts[2])] {
        call.vm
            .state_mut()
            .set_system_integer(slot, value)
            .expect("known result slot");
    }
    Ok(Flow::Done)
}
pub(crate) fn direct_text(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    if !hosts.messages.record_output {
        return Ok(Flow::Unhandled);
    }
    hosts
        .messages
        .place_text_with_layout(call.vm, call.pc, place_layout)?;
    Ok(Flow::Done)
}
pub(crate) fn direct_window(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    if !hosts.messages.record_output {
        return Ok(Flow::Unhandled);
    }
    hosts
        .messages
        .place_window_text_with_layout(call.vm, call.pc, place_layout)?;
    Ok(Flow::Done)
}
pub(crate) fn append_window(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    Ok(
        match hosts.messages.try_record_output_with_layout(
            call.vm,
            hosts.input,
            hosts.flags,
            append_layout,
        )? {
            Some(step) => Flow::from_step(step, call.vm.pending_system().is_some()),
            None => Flow::Unhandled,
        },
    )
}
pub(crate) fn poll_window(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    Ok(
        if hosts
            .messages
            .try_record_poll_with_layout(call.vm, hosts.flags, append_layout)?
        {
            Flow::Done
        } else {
            Flow::Unhandled
        },
    )
}
pub(crate) fn auxiliary_text(
    hosts: &mut TextHosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, DecodeError> {
    if !hosts.messages.record_output {
        return Ok(Flow::Unhandled);
    }
    let slot = call.argument(4, "auxiliary slot")? as usize;
    let raw = call.argument(8, "text")?;
    let text = call.vm.resolve_string(raw)?;
    let host = &mut hosts.messages;
    let group = host
        .auxiliary_texts
        .get(slot)
        .and_then(Option::as_ref)
        .ok_or_else(|| call.error("auxiliary text group does not exist or outside 0..7"))?;
    let layout = group
        .layout_font_cells(
            &text,
            host.font_cells.proportional,
            host.glyph_metrics.as_deref(),
        )
        .map_err(|why| call.error(why))?;
    call.complete(0x4008)?;
    host.auxiliary_texts[slot]
        .as_mut()
        .expect("validated group")
        .append(layout);
    Ok(Flow::Done)
}

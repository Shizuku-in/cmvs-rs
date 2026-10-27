//! Explicit recording of the first, non-wrapping 0x2152 output branch.
//! These records are not glyph textures and must not imply successful font rendering.
use super::{MessageHost, TextObject, window_slot};
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

/// Settings block constructor: +1544=4, +1536=1 (alpha reveal), +1540=2 (350ms),
/// +1528=1. READ_SKIP and script skip initially zero. Other modes remain barriers.
fn append(text: &mut TextObject, bytes: &[u8], skip: bool) -> Result<(), &'static str> {
    let mut x = text.cursor[0] as u32;
    let mut y = text.cursor[1] as u32;
    let size = text.glyph_size as u32;
    let limit = (text.area[0] as u32)
        .wrapping_add(text.area[2] as u32)
        .wrapping_sub(text.spacing[0] as u32)
        .wrapping_sub(size);
    text.ruby_lines.fill(0);
    if text.ruby && text.ruby_every_line {
        y = y.wrapping_add(text.ruby_size as u32);
    }
    text.layout_scratch = [0; 2];
    let interval = if skip {
        0
    } else {
        4u32.wrapping_mul(text.char_delay as u32) / 10
    };
    let mut delay = 0u32;
    let mut i = 0;
    while i < bytes.len() && bytes[i] != 0 {
        let byte = bytes[i];
        if matches!(byte, b'\\' | b'{') {
            return Err("recorded message output does not yet support escapes or ruby groups");
        }
        if x >= limit {
            return Err("recorded message output does not yet support line wrapping");
        }
        if text.glyphs.len() >= 4096 {
            return Err("recorded message glyph limit exceeded");
        }
        let wide = (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte);
        let code = if wide {
            let second = *bytes
                .get(i + 1)
                .filter(|&&b| b != 0)
                .ok_or("truncated double-byte message character")?;
            i += 2;
            u16::from_be_bytes([byte, second])
        } else {
            i += 1;
            byte as i8 as i16 as u16
        };
        let gx = if code == 0x814a {
            x.wrapping_sub(size / 5)
        } else {
            x
        };
        text.glyphs.push(RecordedGlyph {
            code,
            position: [gx as i32, y as i32],
            size,
            colors: [text.color, text.decoration_color],
            gradient: match text.decoration {
                1..=3 => text.decoration as u32,
                _ => 0,
            },
            outline: [1, text.glyph_parameter],
            delay_ms: delay,
            elapsed_ms: 0,
            duration_ms: 350,
            active: interval != 0,
            shown: interval == 0,
            opacity: if interval == 0 { 255 } else { 0 },
        });
        if text.line_break.is_some() {
            text.layout_scratch[1] = u32::from(code);
        }
        let advance = if code == 0x814a {
            (size >> 1).wrapping_sub(size / 5)
        } else if wide {
            size
        } else {
            size >> 1
        };
        x = x.wrapping_add(advance).wrapping_add(text.spacing[0] as u32);
        delay = delay.wrapping_add(interval);
    }
    text.cursor = [x as i32, y as i32];
    Ok(())
}

impl MessageHost {
    /// 0x140076700 / 0x14005d900: repeat=0 creates glyphs once; hidden windows keep
    /// yielding. Waiting returns 0xa000, completion 0xc00c (both yield). Research mode
    /// explicitly selects the constructor settings block. Only the +1520 skip flag shortens
    /// reveal timing; it, held actions 7/8 and the +12244 bypass finish without consuming.
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
        let done = !window.hidden && (!active || skip || input.released(0));
        let consume = done && active && !skip && input.released(0);
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
        }
        Ok(Some(step))
    }
}

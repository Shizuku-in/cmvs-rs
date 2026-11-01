//! Aoitori 0x23b9/0x23bb: byte/pixel counts, with a temporary pen and committed scratch.
//! Executable-bound evidence: docs/games/aoitori/vm-semantics.md.
use super::{GlyphMetrics, auxiliary::AuxiliaryText, measure::lead, output::parse_group};

struct Cursor<'a> {
    group: &'a AuxiliaryText,
    x: f32,
    lines: u32,
    count: f32,
    widest: f32,
    scratch: [u32; 2],
}

impl Cursor<'_> {
    fn wrap(&mut self, code: u16, simple: bool) -> Result<(), &'static str> {
        if self
            .group
            .wraps_font_cells(self.x, code, simple, &mut self.scratch)?
        {
            self.lines += 1;
            if self.lines >= 32 {
                return Err("auxiliary measurement exceeds the 32 cleared ruby-line flags");
            }
            self.x = self.group.float_font_cells(16);
            // Without the extended rules, automatic wrapping retains the old count.
            if self.group.line_break.is_some() {
                self.count = 0.0;
                self.scratch = [0; 2];
            }
        }
        Ok(())
    }

    fn add(&mut self, advance: f32, count: f32) {
        self.x += advance;
        self.count += count;
        self.widest = self.widest.max(self.count);
    }
}

impl AuxiliaryText {
    pub(super) fn measure_font_cells(
        &self,
        bytes: &[u8],
        pixels: bool,
        proportional: bool,
        metrics: Option<&dyn GlyphMetrics>,
    ) -> Result<([u32; 3], [u32; 2]), &'static str> {
        let size = self.word(48).expect("modeled field");
        let spacing = self.float_font_cells(52);
        let width = |code: u16| -> Result<f32, &'static str> {
            let display = self.word(352).expect("modeled field");
            let logical = self.word(360).expect("modeled field");
            if display == 0 || logical == 0 || display > i32::MAX as u32 {
                return Err("invalid auxiliary measurement scale");
            }
            let raster_size = display.wrapping_mul(size) / logical;
            let measured = if proportional {
                metrics
                    .ok_or("proportional text requires a font metrics backend")?
                    .advance_width(&self.face, code, raster_size, false)?
            } else if code > 0xff && code < 0xff80 {
                raster_size
            } else {
                metrics
                    .ok_or("pixel text measurement requires a font metrics backend")?
                    .ink_width(&self.face, code, raster_size)?
            };
            if measured > i16::MAX as u32 {
                return Err("auxiliary font width exceeds the native signed word");
            }
            let width = if display == logical {
                measured as i32
            } else {
                let product = (logical as i32).wrapping_mul(measured as i32);
                let divisor = display as i32;
                product / divisor + i32::from(product % divisor != 0)
            };
            Ok(width as f32)
        };
        let mut state = Cursor {
            group: self,
            x: self.float_font_cells(176),
            lines: 0,
            count: 0.0,
            widest: 0.0,
            scratch: [0; 2],
        };
        let add = |state: &mut Cursor<'_>, code: u16, wide: bool| -> Result<(), &'static str> {
            if pixels {
                let advance = width(code)? + spacing;
                state.add(advance, advance);
            } else {
                let advance = if wide {
                    size as i32 as f32
                } else {
                    (size >> 1) as f32
                };
                state.add(advance + spacing, if wide { 2.0 } else { 1.0 });
            }
            Ok(())
        };
        let mut i = 0;
        while let Some(&first) = bytes.get(i).filter(|&&b| b != 0) {
            if first == b'\\' {
                match bytes.get(i + 1) {
                    Some(b'"' | b'%' | b'@' | b'{' | b'}') => {
                        let code = if bytes[i + 1] == b'@' {
                            b'\\'
                        } else {
                            bytes[i + 1]
                        };
                        state.wrap(u16::from(code), true)?;
                        add(&mut state, u16::from(code), false)?;
                        i += 2;
                    }
                    Some(b'n') => {
                        state.x = self.float_font_cells(16);
                        state.lines += 1;
                        state.count = 0.0;
                        state.scratch = [0; 2];
                        i += 2;
                    }
                    Some(b'c' | b't') => i += 2,
                    Some(b'w') if bytes.get(i + 2).is_some_and(|&b| b != 0) => i += 3,
                    _ => return Err("unknown or truncated auxiliary measurement escape"),
                }
            } else if first == b'{' {
                let group = parse_group(bytes, i + 1)?;
                state.wrap(0, true)?;
                for code in group.base.into_iter().flatten() {
                    add(&mut state, code, true)?;
                }
                i = group.end;
            } else {
                let wide = lead(first);
                let code = if wide {
                    let second = *bytes
                        .get(i + 1)
                        .filter(|&&b| b != 0)
                        .ok_or("truncated double-byte auxiliary character")?;
                    i += 2;
                    u16::from_be_bytes([first, second])
                } else {
                    i += 1;
                    first as i8 as i16 as u16
                };
                state.wrap(code, !wide)?;
                add(&mut state, code, wide)?;
            }
        }
        let truncate = |value: f32| {
            if (-2147483648.0..2147483648.0).contains(&value) {
                value as i32 as u32
            } else {
                i32::MIN as u32
            }
        };
        Ok((
            [
                truncate(state.widest),
                state.lines + 1,
                truncate(state.count),
            ],
            state.scratch,
        ))
    }
}

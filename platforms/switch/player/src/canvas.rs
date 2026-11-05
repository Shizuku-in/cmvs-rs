//! Full-screen host pages (game list, errors) drawn on the CPU with the menu font.
use crate::SCREEN;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use cmvs_core::raster::RgbaImage;

/// Left and right page margin.
pub const MARGIN: i32 = 64;

pub const BACKGROUND: [u8; 3] = [24, 30, 42];
pub const HIGHLIGHT: [u8; 3] = [46, 104, 196];
pub const TEXT: [u8; 3] = [200, 208, 224];
pub const BRIGHT: [u8; 3] = [255, 255, 255];
pub const DIM: [u8; 3] = [120, 132, 152];

pub struct Canvas {
    pixels: Vec<u8>,
}
impl Canvas {
    pub fn new() -> Self {
        let mut pixels = Vec::with_capacity((SCREEN[0] * SCREEN[1] * 4) as usize);
        for _ in 0..SCREEN[0] * SCREEN[1] {
            pixels.extend_from_slice(&[BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]);
        }
        Self { pixels }
    }
    fn blend(&mut self, x: i32, y: i32, color: [u8; 3], coverage: f32) {
        if x < 0 || y < 0 || x >= SCREEN[0] as i32 || y >= SCREEN[1] as i32 {
            return;
        }
        let at = (y as usize * SCREEN[0] as usize + x as usize) * 4;
        let c = coverage.clamp(0.0, 1.0);
        for (i, channel) in color.iter().enumerate() {
            let under = f32::from(self.pixels[at + i]);
            self.pixels[at + i] = (under + (f32::from(*channel) - under) * c).round() as u8;
        }
    }
    pub fn fill(&mut self, [x, y, w, h]: [i32; 4], color: [u8; 3]) {
        for row in y..y + h {
            for column in x..x + w {
                self.blend(column, row, color, 1.0);
            }
        }
    }
    pub fn text_width(font: &FontVec, px: f32, text: &str) -> f32 {
        let font = font.as_scaled(PxScale::from(px));
        let mut width = 0.0;
        let mut previous = None;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            if let Some(previous) = previous {
                width += font.kern(previous, id);
            }
            width += font.h_advance(id);
            previous = Some(id);
        }
        width
    }
    /// Draws one line with its baseline at `y`, shortened with an ellipsis to fit.
    pub fn text(
        &mut self,
        font: &FontVec,
        px: f32,
        [x, y]: [f32; 2],
        max: f32,
        text: &str,
        color: [u8; 3],
    ) {
        let mut line = text.to_owned();
        if Self::text_width(font, px, &line) > max {
            let ellipsis = if font.glyph_id('…').0 != 0 {
                "…"
            } else {
                "..."
            };
            let mut chars: Vec<char> = text.chars().collect();
            while !chars.is_empty() {
                chars.pop();
                line = chars.iter().collect::<String>() + ellipsis;
                if Self::text_width(font, px, &line) <= max {
                    break;
                }
            }
        }
        let scaled = font.as_scaled(PxScale::from(px));
        let mut caret = x;
        let mut previous = None;
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(previous) = previous {
                caret += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(px, point(caret, y));
            caret += scaled.h_advance(id);
            previous = Some(id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    self.blend(
                        bounds.min.x as i32 + gx as i32,
                        bounds.min.y as i32 + gy as i32,
                        color,
                        coverage,
                    );
                });
            }
        }
    }
    pub fn finish(self) -> RgbaImage {
        RgbaImage::new(SCREEN[0], SCREEN[1], self.pixels).expect("screen-sized image")
    }
}

/// Splits `text` into lines no wider than `max`, breaking at spaces where possible and
/// between any characters otherwise (Japanese has no spaces). Newlines are kept.
pub fn wrap(text: &str, max: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end();
        let mut line = String::new();
        for ch in paragraph.chars() {
            line.push(ch);
            if width(&line) <= max || line.chars().count() == 1 {
                continue;
            }
            line.pop();
            match line.rfind(' ').filter(|&at| at > 0) {
                Some(at) => {
                    let rest = line[at + 1..].to_owned();
                    line.truncate(at);
                    lines.push(std::mem::take(&mut line));
                    line = rest;
                }
                None => lines.push(std::mem::take(&mut line)),
            }
            line.push(ch);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wraps_at_spaces_then_anywhere_and_keeps_newlines() {
        let width = |s: &str| s.chars().count() as f32;
        assert_eq!(
            wrap("invalid scene surface\nat main / 0x1f", 10.0, width),
            ["invalid", "scene", "surface", "at main /", "0x1f"]
        );
        assert_eq!(
            wrap("ゲームが見つかりません", 4.0, width),
            ["ゲームが", "見つかり", "ません"]
        );
        assert_eq!(wrap("", 4.0, width), [""]);
    }
}

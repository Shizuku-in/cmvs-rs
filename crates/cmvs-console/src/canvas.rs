//! Full-screen host pages (game list, errors) drawn on the CPU with the menu font.
//! Pages are laid out on a 720-unit-high grid scaled to the screen, so they keep their
//! proportions on any console; all positions and sizes here are in those units.
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use cmvs_core::raster::RgbaImage;

/// Left and right page margin.
pub const MARGIN: i32 = 64;
/// Page height in layout units.
pub const HEIGHT: f32 = 720.0;

pub const BACKGROUND: [u8; 3] = [24, 30, 42];
pub const HIGHLIGHT: [u8; 3] = [46, 104, 196];
pub const TEXT: [u8; 3] = [200, 208, 224];
pub const BRIGHT: [u8; 3] = [255, 255, 255];
pub const DIM: [u8; 3] = [120, 132, 152];

/// Screen pixels per layout unit.
pub fn scale(screen: [u32; 2]) -> f32 {
    screen[1] as f32 / HEIGHT
}

/// Page width in layout units.
pub fn width(screen: [u32; 2]) -> f32 {
    screen[0] as f32 / scale(screen)
}

pub struct Canvas {
    size: [u32; 2],
    scale: f32,
    pixels: Vec<u8>,
}
impl Canvas {
    /// A background-filled page for a `screen`-sized display.
    pub fn new(screen: [u32; 2]) -> Self {
        let mut pixels = Vec::with_capacity((screen[0] * screen[1] * 4) as usize);
        for _ in 0..screen[0] * screen[1] {
            pixels.extend_from_slice(&[BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]);
        }
        Self {
            size: screen,
            scale: scale(screen),
            pixels,
        }
    }
    /// Page width in layout units.
    pub fn width(&self) -> f32 {
        width(self.size)
    }
    /// Blends one screen pixel.
    fn blend(&mut self, x: i32, y: i32, color: [u8; 3], coverage: f32) {
        if x < 0 || y < 0 || x >= self.size[0] as i32 || y >= self.size[1] as i32 {
            return;
        }
        let at = (y as usize * self.size[0] as usize + x as usize) * 4;
        let c = coverage.clamp(0.0, 1.0);
        for (i, channel) in color.iter().enumerate() {
            let under = f32::from(self.pixels[at + i]);
            self.pixels[at + i] = (under + (f32::from(*channel) - under) * c).round() as u8;
        }
    }
    pub fn fill(&mut self, [x, y, w, h]: [i32; 4], color: [u8; 3]) {
        let to_screen = |v: i32| (v as f32 * self.scale).round() as i32;
        let (left, top) = (to_screen(x), to_screen(y));
        let (right, bottom) = (to_screen(x + w), to_screen(y + h));
        for row in top..bottom {
            for column in left..right {
                self.blend(column, row, color, 1.0);
            }
        }
    }
    /// Advance width of `text` at `px`, in the units of `px`.
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
        // Rasterized at screen resolution; advances scale linearly with the size.
        let (px, x, y) = (px * self.scale, x * self.scale, y * self.scale);
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
        RgbaImage::new(self.size[0], self.size[1], self.pixels).expect("screen-sized image")
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
    fn layout_units_scale_to_the_screen() {
        assert_eq!(scale([1280, 720]), 1.0);
        assert!((width([960, 544]) - 1270.6).abs() < 0.1);
        let mut canvas = Canvas::new([960, 544]);
        canvas.fill([0, 0, 4, 720], [255, 0, 0]);
        let image = canvas.finish();
        let red = |x: usize, y: usize| image.pixels()[(y * 960 + x) * 4] == 255;
        assert!(red(2, 543) && !red(3, 0));
    }

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

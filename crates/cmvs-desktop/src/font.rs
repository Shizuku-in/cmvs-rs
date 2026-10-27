//! Explicit OpenType font preview. Script layout stays in core; this backend replaces
//! the original GDI mask rasterizer, so its pixels are not claimed to match GDI.
use ab_glyph::{Font, FontVec, ScaleFont, point};
use cmvs_core::{
    FormatError,
    message::{MESSAGE_WINDOWS, MessageHost, RecordedGlyph, TEXT_OBJECTS},
    raster::RgbaImage,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
    scene_render,
};
use std::{collections::BTreeMap, error::Error, fs::File, io::Read, path::Path};

const MAX_FONT: usize = 32 * 1024 * 1024;

/// No automatic font discovery or redistribution. TTC face selection is explicit.
pub struct FontRenderer {
    font: FontVec,
    cache: BTreeMap<[u32; 8], RgbaImage>,
}

fn character(code: u16) -> Result<char, FormatError> {
    let pair = code.to_be_bytes();
    let bytes = if code <= 0x7f || code >= 0xff80 {
        &pair[1..]
    } else {
        &pair[..]
    };
    let text = encoding_rs::SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or(FormatError("invalid Shift-JIS glyph code"))?;
    let mut chars = text.chars();
    let ch = chars.next().ok_or(FormatError("empty glyph code"))?;
    if chars.next().is_some() || ch == '\0' {
        return Err(FormatError(
            "glyph code must decode to one non-NUL character",
        ));
    }
    Ok(ch)
}

fn rgb(color: u32) -> [u8; 3] {
    let [b, g, r, _] = color.to_le_bytes();
    [r, g, b]
}

/// Straight-alpha foreground over a dilation outline; only equal-endpoint gradients
/// are supported. Glyph shapes/baselines follow ab_glyph, not Windows GDI hinting.
fn paint(
    mask: &[u8],
    side: usize,
    color: u32,
    outline: [u32; 2],
) -> Result<RgbaImage, FormatError> {
    let radius = match outline[0] {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => return Err(FormatError("unsupported glyph outline mode")),
    };
    let mut dilation = vec![0u8; mask.len()];
    if radius != 0 {
        for y in 0..side {
            for x in 0..side {
                let value = mask[y * side + x];
                for yy in y.saturating_sub(radius)..=(y + radius).min(side - 1) {
                    for xx in x.saturating_sub(radius)..=(x + radius).min(side - 1) {
                        dilation[yy * side + xx] = dilation[yy * side + xx].max(value);
                    }
                }
            }
        }
    }
    let fg = rgb(color);
    let bg = rgb(outline[1]);
    let mut pixels = Vec::with_capacity(mask.len() * 4);
    for (a, b) in mask.iter().zip(dilation) {
        let a = u32::from(*a);
        let b = u32::from(b);
        let behind = (b * (255 - a) + 127) / 255;
        let alpha = a + behind;
        for c in 0..3 {
            pixels.push(
                (u32::from(fg[c]) * a + u32::from(bg[c]) * behind + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0) as u8,
            );
        }
        pixels.push(alpha as u8);
    }
    RgbaImage::new(side as u32, side as u32, pixels)
}

impl FontRenderer {
    pub fn read(path: &Path, face: u32) -> Result<Self, Box<dyn Error>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_FONT as u64 + 1)
            .read_to_end(&mut bytes)?;
        Ok(Self::from_bytes(bytes, face)?)
    }
    pub fn from_bytes(bytes: Vec<u8>, face: u32) -> Result<Self, FormatError> {
        if bytes.len() > MAX_FONT {
            return Err(FormatError("font exceeds 32 MiB research limit"));
        }
        let font = FontVec::try_from_vec_and_index(bytes, face)
            .map_err(|_| FormatError("invalid OpenType font or collection face index"))?;
        Ok(Self {
            font,
            cache: BTreeMap::new(),
        })
    }
    fn glyph(&mut self, glyph: &RecordedGlyph) -> Result<RgbaImage, FormatError> {
        if !(1..=128).contains(&glyph.size) {
            return Err(FormatError("glyph size outside 1..128 preview range"));
        }
        if glyph.gradient > 3 || (glyph.gradient != 0 && glyph.colors[0] != glyph.colors[1]) {
            return Err(FormatError(
                "non-flat glyph gradients are not implemented by the font preview",
            ));
        }
        if glyph.outline[0] > 2 {
            return Err(FormatError("unsupported glyph outline mode"));
        }
        let key = [
            u32::from(glyph.code),
            glyph.size,
            glyph.colors[0],
            glyph.colors[1],
            glyph.gradient,
            glyph.outline[0],
            glyph.outline[1],
            0,
        ];
        if let Some(image) = self.cache.get(&key) {
            return Ok(image.clone());
        }
        let ch = character(glyph.code)?;
        let id = self.font.glyph_id(ch);
        if id.0 == 0 {
            return Err(FormatError(
                "selected font has no glyph for a requested character",
            ));
        }
        let scale = glyph.size as f32;
        let baseline = self.font.as_scaled(scale).ascent();
        let side = glyph.size as usize + 4;
        let mut mask = vec![0u8; side * side];
        if let Some(outline) = self
            .font
            .outline_glyph(id.with_scale_and_position(scale, point(0.0, baseline)))
        {
            let bounds = outline.px_bounds();
            let cap = scale * 4.0 + 16.0;
            if ![bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= cap)
            {
                return Err(FormatError("glyph outline exceeds preview bounds"));
            }
            outline.draw(|x, y, coverage| {
                let x = x as i32 + bounds.min.x as i32 + 2;
                let y = y as i32 + bounds.min.y as i32 + 2;
                if x >= 0 && y >= 0 && x < side as i32 && y < side as i32 {
                    mask[y as usize * side + x as usize] =
                        (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            });
        } else if !ch.is_whitespace() {
            return Err(FormatError(
                "selected font lacks a supported outline for a character",
            ));
        }
        let image = paint(&mask, side, glyph.colors[0], glyph.outline)?;
        if self.cache.len() >= 128 {
            self.cache.clear();
        }
        self.cache.insert(key, image.clone());
        Ok(image)
    }

    /// Adds glyph images and visible message-window objects to the core's layer queue.
    /// Current layout is the 1280x720 research viewport, unit text scale, flat gradients.
    pub fn render(
        &mut self,
        scene: &SceneHost,
        messages: &MessageHost,
    ) -> Result<RgbaImage, FormatError> {
        let mut objects = Vec::new();
        for text in (0..TEXT_OBJECTS)
            .filter_map(|i| messages.text(i))
            .filter(|t| t.visible && !t.glyphs.is_empty())
        {
            if text.scale_bits != 1.0f32.to_bits() || text.logical_size != [1280, 720] {
                return Err(FormatError(
                    "font preview requires unit scale and 1280x720 text coordinates",
                ));
            }
            for glyph in text.glyphs.iter().filter(|g| g.shown && g.opacity != 0) {
                if objects.len() >= 512 {
                    return Err(FormatError("font preview exceeds 512 visible glyphs"));
                }
                let pixels = self.glyph(glyph)?;
                let mut draw = DrawState::default();
                draw.words[4] = pixels.width();
                draw.words[5] = pixels.height();
                draw.words[8] = (text.position[0] as u32)
                    .wrapping_add(glyph.position[0] as u32)
                    .wrapping_sub(2);
                draw.words[9] = (text.position[1] as u32)
                    .wrapping_add(glyph.position[1] as u32)
                    .wrapping_sub(2);
                draw.words[10] = text.layer as u32;
                draw.words[17] = glyph.opacity;
                objects.push(SceneObject {
                    image: Some(ObjectImage {
                        name: Vec::new(),
                        pixels,
                        alpha: true,
                    }),
                    draw: Some(draw),
                    ..SceneObject::default()
                });
            }
        }
        let mut additional: Vec<_> = objects.iter().collect();
        additional.extend(
            (0..MESSAGE_WINDOWS)
                .filter_map(|i| messages.window(i))
                .filter(|w| w.visible && !w.hidden)
                .map(|w| &w.object),
        );
        scene_render::render_with_objects(scene, &additional, 1280, 720)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glyph_codes_decode_strictly_including_sign_extended_halfwidth() {
        assert_eq!(character(0x82a0).unwrap(), 'あ');
        assert_eq!(character(0xffb1).unwrap(), 'ｱ');
        assert_eq!(character(65).unwrap(), 'A');
        assert!(character(0).is_err());
        assert!(character(0x817f).is_err());
    }
    #[test]
    fn synthetic_mask_has_transparent_padding_and_a_one_pixel_outline() {
        let mut mask = vec![0; 25];
        mask[12] = 255;
        let image = paint(&mask, 5, 0xffffff, [1, 0x123456]).unwrap();
        assert_eq!(&image.pixels()[48..52], &[255, 255, 255, 255]);
        assert_eq!(&image.pixels()[24..28], &[0x12, 0x34, 0x56, 255]);
        assert_eq!(&image.pixels()[..4], &[0, 0, 0, 0]);
        assert!(paint(&mask, 5, 0, [3, 0]).is_err());
    }
    #[test]
    fn invalid_font_data_or_collection_face_is_an_error() {
        assert!(FontRenderer::from_bytes(vec![], 0).is_err());
        assert!(FontRenderer::from_bytes(b"ttcf".to_vec(), 1).is_err());
    }
}

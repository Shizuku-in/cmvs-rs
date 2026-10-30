//! Explicit OpenType font preview. Script layout stays in core; this backend replaces
//! the original GDI mask rasterizer, so its pixels are not claimed to match GDI.
use crate::font_library::FontLibrary;
use ab_glyph::{Font, FontVec, ScaleFont, point};
use cmvs_core::{
    FormatError,
    camera::CameraHost,
    message::{AUXILIARY_TEXTS, MESSAGE_WINDOWS, MessageHost, RecordedGlyph, TEXT_OBJECTS},
    raster::RgbaImage,
    scene::{DrawState, ObjectImage, SceneHost, SceneObject},
    scene_render,
};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};

const MAX_FONT: usize = 32 * 1024 * 1024;

/// The explicit font file (TTC face selected explicitly) serves the built-in default
/// face; faces named by glyphs are resolved through `FontLibrary`.
pub struct FontRenderer {
    library: Arc<Mutex<FontLibrary>>,
    cache: BTreeMap<(Arc<[u8]>, [u32; 8]), RgbaImage>,
}

struct PreviewMetrics(Arc<Mutex<FontLibrary>>);
impl PreviewMetrics {
    fn font(&self, face: &[u8]) -> Result<Arc<FontVec>, &'static str> {
        self.0
            .lock()
            .map_err(|_| "font library lock poisoned")?
            .face(face)
            .map_err(|e| e.0)
    }
}
impl std::fmt::Debug for PreviewMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenType preview metrics")
    }
}
impl cmvs_core::message::GlyphMetrics for PreviewMetrics {
    fn styled_ink_width(
        &self,
        face: &[u8],
        code: u16,
        size: u32,
        bold: bool,
    ) -> Result<u32, &'static str> {
        let width = self.ink_width(face, code, size)?;
        Ok(if bold && width != 0 {
            (width + 1).min(size)
        } else {
            width
        })
    }
    fn ink_width(&self, face: &[u8], code: u16, size: u32) -> Result<u32, &'static str> {
        if !(1..=128).contains(&size) {
            return Err("glyph size outside 1..128 preview range");
        }
        let font = self.font(face)?;
        let ch = character(code).map_err(|e| e.0)?;
        let id = font.glyph_id(ch);
        if id.0 == 0 {
            return Err("selected font has no glyph for a requested character");
        }
        let scale = size as f32;
        let baseline = font.as_scaled(scale).ascent();
        let Some(outline) =
            font.outline_glyph(id.with_scale_and_position(scale, point(0.0, baseline)))
        else {
            return if ch.is_whitespace() {
                Ok(0)
            } else {
                Err("selected font lacks a supported outline")
            };
        };
        let bounds = outline.px_bounds();
        if !bounds.min.x.is_finite() || !bounds.max.x.is_finite() {
            return Err("nonfinite font metrics");
        }
        let left = bounds.min.x.max(0.0);
        let width = (bounds.max.x - bounds.min.x)
            .max(0.0)
            .min((scale - left).max(0.0));
        Ok((left + width) as u32)
    }
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

/// Preview weight synthesis: extend ink by one pixel to the right before painting
/// gradients and outlines. This is not GDI's weight-600 rasterizer.
fn embolden(mask: &mut [u8], side: usize) {
    for row in mask.chunks_exact_mut(side) {
        for x in (1..side).rev() {
            row[x] = row[x].max(row[x - 1]);
        }
    }
}

/// 0x140059d20: signed 10-bit fixed-point color steps over the glyph's size.
/// Mode 1 is vertical, 2 horizontal, 3 diagonal (halve each axis step separately).
/// The final row/column stops one step before the second endpoint.
fn gradient_color(colors: [u32; 2], mode: u32, size: usize, x: usize, y: usize) -> [u8; 3] {
    let first = rgb(colors[0]);
    let last = rgb(colors[1]);
    std::array::from_fn(|c| {
        let base = i32::from(first[c]) << 10;
        let step = ((i32::from(last[c]) << 10) - base) / size as i32;
        let offset = match mode {
            1 => step * y as i32,
            2 => step * x as i32,
            3 => (step / 2) * (x + y) as i32,
            _ => 0,
        };
        ((base + offset) >> 10) as u8
    })
}

/// Straight-alpha foreground over a dilation outline. Glyph shapes/baselines and
/// outline compositing follow this preview backend, not Windows GDI hinting.
fn paint(
    mask: &[u8],
    side: usize,
    colors: [u32; 2],
    gradient: u32,
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
    let size = side - 4;
    let bg = rgb(outline[1]);
    let mut pixels = Vec::with_capacity(mask.len() * 4);
    for (index, (a, b)) in mask.iter().zip(dilation).enumerate() {
        // The mask has two padding pixels on every side. Clamp any overhanging
        // ab_glyph bearings to the nominal GDI color grid.
        let x = (index % side).saturating_sub(2).min(size - 1);
        let y = (index / side).saturating_sub(2).min(size - 1);
        let fg = gradient_color(colors, gradient, size, x, y);
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
            library: Arc::new(Mutex::new(FontLibrary::new(Arc::new(font)))),
            cache: BTreeMap::new(),
        })
    }
    /// Uses `library` (sharing its explicit default font) for every face.
    pub fn with_library(library: FontLibrary) -> Self {
        Self {
            library: Arc::new(Mutex::new(library)),
            cache: BTreeMap::new(),
        }
    }
    /// Matches this renderer's fonts; OpenType ink bounds replace GDI's
    /// device metrics. Whitespace deterministically has zero ink extent.
    pub fn metrics(&self) -> Box<dyn cmvs_core::message::GlyphMetrics> {
        Box::new(PreviewMetrics(Arc::clone(&self.library)))
    }
    /// Whether a face in the game encoding resolves to a font (startup check).
    pub fn has_face(&self, face: &[u8]) -> bool {
        self.library
            .lock()
            .is_ok_and(|mut library| library.contains(face))
    }
    fn font(&self, face: &[u8]) -> Result<Arc<FontVec>, FormatError> {
        self.library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?
            .face(face)
    }
    fn glyph(&mut self, glyph: &RecordedGlyph) -> Result<RgbaImage, FormatError> {
        if !(1..=128).contains(&glyph.size) {
            return Err(FormatError("glyph size outside 1..128 preview range"));
        }
        if glyph.gradient > 3 {
            return Err(FormatError("unsupported glyph gradient mode"));
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
            u32::from(glyph.bold),
        ];
        let key = (glyph.face.clone(), key);
        if let Some(image) = self.cache.get(&key) {
            return Ok(image.clone());
        }
        let font = self.font(&glyph.face)?;
        let ch = character(glyph.code)?;
        let id = font.glyph_id(ch);
        if id.0 == 0 {
            return Err(FormatError(
                "selected font has no glyph for a requested character",
            ));
        }
        let scale = glyph.size as f32;
        let baseline = font.as_scaled(scale).ascent();
        let side = glyph.size as usize + 4;
        let mut mask = vec![0u8; side * side];
        if let Some(outline) =
            font.outline_glyph(id.with_scale_and_position(scale, point(0.0, baseline)))
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
        if glyph.bold {
            embolden(&mut mask, side);
        }
        let image = paint(&mask, side, glyph.colors, glyph.gradient, glyph.outline)?;
        if self.cache.len() >= 128 {
            self.cache.clear();
        }
        self.cache.insert(key, image.clone());
        Ok(image)
    }

    /// Adds glyph images and visible message-window objects to the core's layer queue.
    /// Current layout uses the 1280x720 research viewport and finite nonnegative text scale.
    pub fn render(
        &mut self,
        scene: &SceneHost,
        cameras: &CameraHost,
        messages: &MessageHost,
    ) -> Result<RgbaImage, FormatError> {
        let mut objects = Vec::new();
        let mut visible_glyphs = 0;
        for text in (0..TEXT_OBJECTS)
            .filter_map(|i| messages.text(i))
            .filter(|t| t.visible && !t.glyphs.is_empty())
        {
            if text.logical_size != [1280, 720] {
                return Err(FormatError(
                    "font preview requires 1280x720 text coordinates",
                ));
            }
            let scale = f32::from_bits(text.scale_bits);
            if !scale.is_finite() || scale < 0.0 {
                return Err(FormatError(
                    "font preview requires finite nonnegative scale",
                ));
            }
            let mut parent_draw = DrawState::default();
            parent_draw.words[8..10].copy_from_slice(&text.position.map(|v| v as u32));
            parent_draw.words[10] = text.layer as u32;
            parent_draw.words[18..20].fill(text.scale_bits);
            // The original text scene object owns attached glyph children. Scaling
            // acts on their relative positions and padded cells, not the text anchor.
            let mut parent = SceneObject {
                draw: Some(parent_draw),
                flags: 0xffff_ffff,
                ..SceneObject::default()
            };
            for glyph in text.glyphs.iter().filter(|g| g.shown && g.opacity != 0) {
                if visible_glyphs >= 512 {
                    return Err(FormatError("font preview exceeds 512 visible glyphs"));
                }
                visible_glyphs += 1;
                let pixels = self.glyph(glyph)?;
                let mut draw = DrawState::default();
                draw.words[4] = glyph.reveal_width.min(pixels.width());
                draw.words[5] = pixels.height();
                draw.words[8] = (glyph.position[0] as u32).wrapping_sub(2);
                draw.words[9] = (glyph.position[1] as u32).wrapping_sub(2);
                draw.words[10] = text.layer as u32;
                draw.words[17] = glyph.opacity;
                parent.children.insert(
                    parent.children.len() as u16,
                    SceneObject {
                        image: Some(ObjectImage {
                            filter: None,
                            name: Vec::new(),
                            pixels,
                            alpha: true,
                            surface: false,
                        }),
                        draw: Some(draw),
                        ..SceneObject::default()
                    },
                );
            }
            objects.push(parent);
        }
        // 0x140015da0 queues the +4328 auxiliary atlas objects after the text objects and
        // before the windows. Attached glyphs (0x14002eb10) inherit the atlas position,
        // opacity and mode-4 second texture, keep the layer they were attached with, and
        // queue independently.
        for group in (0..AUXILIARY_TEXTS)
            .filter_map(|i| messages.auxiliary_text(i))
            .filter(|g| g.attached_glyphs().next().is_some())
        {
            let mut parent_draw = DrawState::default();
            parent_draw.words[8..10].copy_from_slice(&group.glyph_origin()?.map(|v| v as u32));
            parent_draw.texture_mode = group
                .object
                .draw
                .as_ref()
                .map_or(0, |draw| draw.texture_mode);
            let mut parent = SceneObject {
                draw: Some(parent_draw),
                secondary_image: group.secondary_image.clone(),
                flags: 0x8000_0000,
                ..SceneObject::default()
            };
            for glyph in group.attached_glyphs() {
                let pixels = self.glyph(&RecordedGlyph {
                    face: glyph.face.clone(),
                    bold: false,
                    code: glyph.code,
                    position: glyph.position,
                    size: glyph.size,
                    colors: glyph.colors,
                    gradient: glyph.gradient,
                    outline: glyph.outline,
                    delay_ms: 0,
                    elapsed_ms: 0,
                    duration_ms: 0,
                    active: false,
                    shown: true,
                    opacity: 255,
                    reveal_mode: 0,
                    reveal_width: glyph.size + 4,
                })?;
                let mut draw = DrawState::default();
                draw.words[4] = pixels.width();
                draw.words[5] = pixels.height();
                draw.words[8] = (glyph.position[0] as u32).wrapping_sub(2);
                draw.words[9] = (glyph.position[1] as u32).wrapping_sub(2);
                draw.words[10] = glyph.layer;
                parent.children.insert(
                    parent.children.len() as u16,
                    SceneObject {
                        image: Some(ObjectImage {
                            filter: None,
                            name: Vec::new(),
                            pixels,
                            alpha: true,
                            surface: false,
                        }),
                        draw: Some(draw),
                        flags: 1,
                        ..SceneObject::default()
                    },
                );
            }
            objects.push(parent);
        }
        let mut additional: Vec<_> = objects.iter().collect();
        additional.extend(
            (0..MESSAGE_WINDOWS)
                .filter_map(|i| messages.window(i))
                .filter(|w| w.visible && !w.hidden)
                .map(|w| &w.object),
        );
        scene_render::render_scene(scene, Some(cameras), &additional, 1280, 720)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires CMVS_FONT_PATH; synthetic glyphs and external font"]
    fn bold_glyph_uses_a_distinct_cache_entry_and_increases_ink() {
        let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
        let mut font = FontRenderer::read(Path::new(&path), 0).unwrap();
        let mut glyph = RecordedGlyph {
            face: Arc::from(cmvs_core::settings::DEFAULT_FONT_FACE),
            bold: false,
            code: 0x82a0,
            position: [0, 0],
            size: 27,
            colors: [0xffffff; 2],
            gradient: 0,
            outline: [0, 0],
            delay_ms: 0,
            elapsed_ms: 0,
            duration_ms: 0,
            active: false,
            shown: true,
            opacity: 255,
            reveal_mode: 0,
            reveal_width: 31,
        };
        let normal = font.glyph(&glyph).unwrap();
        glyph.bold = true;
        let bold = font.glyph(&glyph).unwrap();
        let ink = |image: &RgbaImage| {
            image
                .pixels()
                .chunks_exact(4)
                .map(|p| u64::from(p[3]))
                .sum::<u64>()
        };
        assert!(ink(&bold) > ink(&normal));
        for (before, after) in normal
            .pixels()
            .chunks_exact(4)
            .zip(bold.pixels().chunks_exact(4))
        {
            assert!(after[3] >= before[3]);
        }
        glyph.bold = false;
        assert_eq!(font.glyph(&glyph).unwrap(), normal);
        assert_eq!(font.cache.len(), 2);
        let metrics = font.metrics();
        let face = cmvs_core::settings::DEFAULT_FONT_FACE;
        let width = metrics.ink_width(face, 65, 27).unwrap();
        assert_eq!(
            metrics.styled_ink_width(face, 65, 27, true).unwrap(),
            (width + 1).min(27)
        );
        assert_eq!(metrics.styled_ink_width(face, 32, 27, true).unwrap(), 0);
    }
    #[test]
    fn bold_mask_extends_ink_once_without_crossing_rows() {
        let mut mask = vec![0, 20, 200, 0, 0, 0, 0, 0, 255];
        embolden(&mut mask, 3);
        assert_eq!(mask, [0, 20, 200, 0, 0, 0, 0, 0, 255]);
        let mut mask = vec![0, 180, 0, 0, 0, 0, 0, 0, 0];
        embolden(&mut mask, 3);
        assert_eq!(mask, [0, 180, 180, 0, 0, 0, 0, 0, 0]);
    }
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
        let image = paint(&mask, 5, [0xffffff; 2], 0, [1, 0x123456]).unwrap();
        assert_eq!(&image.pixels()[48..52], &[255, 255, 255, 255]);
        assert_eq!(&image.pixels()[24..28], &[0x12, 0x34, 0x56, 255]);
        assert_eq!(&image.pixels()[..4], &[0, 0, 0, 0]);
        assert!(paint(&mask, 5, [0; 2], 0, [3, 0]).is_err());
    }
    #[test]
    fn invalid_font_data_or_collection_face_is_an_error() {
        assert!(FontRenderer::from_bytes(vec![], 0).is_err());
        assert!(FontRenderer::from_bytes(b"ttcf".to_vec(), 1).is_err());
    }
}

#[cfg(test)]
mod gradient_tests {
    use super::*;

    #[test]
    fn gradients_follow_fixed_point_axis_steps_and_do_not_reach_the_endpoint() {
        let colors = [0xffffff, 0];
        assert_eq!(gradient_color(colors, 0, 4, 3, 3), [255; 3]);
        assert_eq!(gradient_color(colors, 1, 4, 3, 0), [255; 3]);
        assert_eq!(gradient_color(colors, 1, 4, 0, 3), [63; 3]);
        assert_eq!(gradient_color(colors, 2, 4, 0, 3), [255; 3]);
        assert_eq!(gradient_color(colors, 2, 4, 3, 0), [63; 3]);
        assert_eq!(gradient_color(colors, 3, 4, 1, 0), [223; 3]);
        assert_eq!(gradient_color(colors, 3, 4, 0, 1), [223; 3]);
        // Signed division truncates toward zero before the diagonal step is halved.
        assert_eq!(gradient_color([0, 0x010101], 3, 3, 2, 2), [0; 3]);
    }

    #[test]
    fn gradient_foreground_keeps_constant_outline_color_and_transparent_padding() {
        let mut mask = vec![0; 64];
        mask[2 * 8 + 2] = 255;
        mask[5 * 8 + 2] = 255;
        let image = paint(&mask, 8, [0xff0000, 0x0000ff], 1, [1, 0x123456]).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels()[(y * 8 + x) * 4..(y * 8 + x + 1) * 4];
        assert_eq!(pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(pixel(2, 5), [63, 0, 191, 255]);
        assert_eq!(pixel(1, 2), [0x12, 0x34, 0x56, 255]);
        assert_eq!(pixel(0, 0), [0, 0, 0, 0]);
    }
}

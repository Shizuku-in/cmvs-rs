//! Explicit OpenType font preview. Script layout stays in core; this backend replaces
//! the original GDI mask rasterizer, so its pixels are not claimed to match GDI.
use crate::composition::SceneComposition;
use crate::font_library::FontLibrary;
use crate::glyph_cache::GlyphCache;
use ab_glyph::{Font, FontVec, ScaleFont, point};
use cmvs_core::{
    FormatError, TextEncoding,
    render::camera::CameraHost,
    render::message::{AUXILIARY_TEXTS, GlyphRasterizer, MessageHost, RecordedGlyph, TEXT_OBJECTS},
    render::raster::RgbaImage,
    render::scene::{SceneHost, SceneObject},
};
use std::{
    error::Error,
    fs::File,
    hash::{Hash, Hasher},
    io::Read,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_FONT: usize = 32 * 1024 * 1024;

#[derive(Clone)]
struct CachedGlyphScene {
    fingerprint: u128,
    objects: Vec<SceneObject>,
}

/// Deterministic two-lane 128-bit streaming hasher for visible glyph-scene state.
struct GlyphSceneHasher {
    lo: u64,
    hi: u64,
}

impl GlyphSceneHasher {
    fn new() -> Self {
        Self {
            lo: 0xcbf2_9ce4_8422_2325,
            hi: 0x8422_2325_cbf2_9ce4,
        }
    }

    fn mix64(mut x: u64) -> u64 {
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }

    fn finish128(self) -> u128 {
        let lo = Self::mix64(self.lo);
        let hi = Self::mix64(self.hi ^ lo.rotate_left(17));
        (u128::from(hi) << 64) | u128::from(lo)
    }
}

impl Hasher for GlyphSceneHasher {
    fn finish(&self) -> u64 {
        Self::mix64(self.lo ^ self.hi)
    }

    fn write(&mut self, bytes: &[u8]) {
        let (chunks, tail) = bytes.as_chunks::<8>();
        for chunk in chunks {
            let word = u64::from_le_bytes(*chunk);
            self.lo = (self.lo ^ word)
                .rotate_left(23)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                .wrapping_add(0x517c_c1b7_2722_0a95);
            self.hi = (self.hi.wrapping_add(word ^ 0xa076_1d64_78bd_642f))
                .rotate_right(19)
                .wrapping_mul(0xe703_7ed1_a0b4_28db);
        }
        if !tail.is_empty() {
            let mut last = 0u64;
            for (i, &b) in tail.iter().enumerate() {
                last |= u64::from(b) << (i * 8);
            }
            last |= (tail.len() as u64) << 56;
            self.lo = (self.lo ^ last)
                .rotate_left(11)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            self.hi = (self.hi ^ last.rotate_left(32)).wrapping_mul(0xe703_7ed1_a0b4_28db);
        }
    }
}

/// The explicit font file (TTC face selected explicitly) serves the built-in default
/// face; faces named by glyphs are resolved through `FontLibrary`.
#[derive(Clone)]
pub struct FontRenderer {
    encoding: TextEncoding,
    library: Arc<Mutex<FontLibrary>>,
    library_generation: Arc<AtomicU64>,
    cache: GlyphCache,
    scene_cache: Option<CachedGlyphScene>,
    #[cfg(test)]
    scene_cache_hits: u64,
    #[cfg(test)]
    glyph_build_calls: u64,
}

struct PreviewMetrics(Arc<Mutex<FontLibrary>>, TextEncoding);
impl PreviewMetrics {
    fn font(&self, face: &[u8], ch: char) -> Result<Arc<FontVec>, &'static str> {
        self.0
            .lock()
            .map_err(|_| "font library lock poisoned")
            .map(|mut library| library.for_character(face, self.1, ch))
    }
}
impl std::fmt::Debug for PreviewMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenType preview metrics")
    }
}
impl cmvs_core::render::message::GlyphMetrics for PreviewMetrics {
    fn display_face_name(&self, face: &[u8]) -> Option<Vec<u8>> {
        if !face.starts_with(b"CMVS-U-") {
            return None;
        }
        // Missing mappings and names outside the game's character set still get
        // a readable label, without exposing a storage handle to the player.
        let name = self
            .0
            .lock()
            .ok()
            .and_then(|mut library| library.family_name(face, self.1).ok());
        Some(
            name.and_then(|name| {
                if name.chars().any(|ch| ch as u32 > 0xffff) {
                    return None;
                }
                let (bytes, _, bad) = codec(self.1).ok()?.encode(&name);
                (!bad).then(|| bytes.into_owned())
            })
            .unwrap_or_else(|| b"Selected font".to_vec()),
        )
    }

    fn advance_width(
        &self,
        face: &[u8],
        code: u16,
        size: u32,
        _bold: bool,
    ) -> Result<u32, &'static str> {
        if !(1..=128).contains(&size) {
            return Err("glyph size outside 1..128 preview range");
        }
        let ch = character(self.1, code).map_err(|e| e.0)?;
        let font = self.font(face, ch)?;
        let advance = font.as_scaled(size as f32).h_advance(font.glyph_id(ch));
        if !advance.is_finite() || advance < 0.0 || advance > u16::MAX as f32 {
            return Err("invalid proportional font advance");
        }
        Ok(advance.round() as u32)
    }
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
        let ch = character(self.1, code).map_err(|e| e.0)?;
        let font = self.font(face, ch)?;
        let id = font.glyph_id(ch);
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

// Keep backend codec selection here, outside core's dependency-free byte scanners.
// New core encodings must be explicitly supported before rendering their glyphs.
// Glyph codes and legacy font face names share this codec.
pub(crate) fn codec(encoding: TextEncoding) -> Result<&'static encoding_rs::Encoding, FormatError> {
    match encoding {
        TextEncoding::ShiftJis => Ok(encoding_rs::SHIFT_JIS),
        TextEncoding::Gbk => Ok(encoding_rs::GBK),
        TextEncoding::Utf8 => Ok(encoding_rs::UTF_8),
        _ => Err(FormatError("unsupported font text encoding")),
    }
}

fn character(encoding: TextEncoding, code: u16) -> Result<char, FormatError> {
    if encoding == TextEncoding::Utf8 {
        // UTF-8 glyph codes are Unicode scalar values (see TextEncoding).
        return char::from_u32(u32::from(code))
            .filter(|&ch| ch != '\0')
            .ok_or(FormatError("invalid UTF-8 glyph code"));
    }
    let pair = code.to_be_bytes();
    let bytes = if code <= 0x7f || (encoding == TextEncoding::ShiftJis && code >= 0xff80) {
        &pair[1..]
    } else {
        &pair[..]
    };
    let text = codec(encoding)?
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or(FormatError(match encoding {
            TextEncoding::ShiftJis => "invalid Shift-JIS glyph code",
            TextEncoding::Gbk => "invalid GBK glyph code",
            _ => "invalid encoded glyph code",
        }))?;
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

// GGO_GRAY4_BITMAP coverage curve from 0x14006b4e0. Portable coverage
// is quantized to the native table's 17 levels; raster shapes remain backend-specific.
fn revise_alpha(alpha: u8) -> u8 {
    const TABLE: [u8; 17] = [
        0, 31, 60, 87, 112, 135, 156, 175, 192, 207, 220, 231, 240, 247, 252, 254, 255,
    ];
    TABLE[(usize::from(alpha) * 16 + 127) / 255]
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
    pub fn read(path: &Path, face: u32) -> Result<Self, Box<dyn Error + Send + Sync>> {
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
            encoding: TextEncoding::default(),
            library: Arc::new(Mutex::new(FontLibrary::new(Arc::new(font)))),
            library_generation: Arc::new(AtomicU64::new(0)),
            cache: GlyphCache::default(),
            scene_cache: None,
            #[cfg(test)]
            scene_cache_hits: 0,
            #[cfg(test)]
            glyph_build_calls: 0,
        })
    }
    /// Adds a font for characters the selected face and the default font lack.
    pub fn add_fallback(&self, bytes: Vec<u8>, face: u32) -> Result<(), FormatError> {
        if bytes.len() > MAX_FONT {
            return Err(FormatError("font exceeds 32 MiB research limit"));
        }
        let font = FontVec::try_from_vec_and_index(bytes, face)
            .map_err(|_| FormatError("invalid OpenType font or collection face index"))?;
        self.library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?
            .add_fallback(Arc::new(font));
        self.library_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    /// Uses `library` (sharing its explicit default font) for every face.
    pub fn with_library(library: FontLibrary) -> Self {
        Self {
            encoding: TextEncoding::default(),
            library: Arc::new(Mutex::new(library)),
            library_generation: Arc::new(AtomicU64::new(0)),
            cache: GlyphCache::default(),
            scene_cache: None,
            #[cfg(test)]
            scene_cache_hits: 0,
            #[cfg(test)]
            glyph_build_calls: 0,
        }
    }
    /// Select the game text encoding before installing this renderer's metrics.
    /// It applies to glyph codes and to legacy font face names of this renderer
    /// only; clones sharing the font library keep their own encoding. Changing it
    /// discards cached glyph images; existing metrics retain their encoding.
    pub fn with_text_encoding(mut self, encoding: TextEncoding) -> Self {
        self.encoding = encoding;
        self.cache = GlyphCache::default();
        self.scene_cache = None;
        self
    }
    /// Matches this renderer's fonts; OpenType ink bounds replace GDI's
    /// device metrics. Whitespace deterministically has zero ink extent.
    pub fn metrics(&self) -> Box<dyn cmvs_core::render::message::GlyphMetrics> {
        Box::new(PreviewMetrics(Arc::clone(&self.library), self.encoding))
    }
    /// Whether a face in the game encoding resolves to a font (startup check).
    pub fn has_face(&self, face: &[u8]) -> bool {
        self.library
            .lock()
            .is_ok_and(|mut library| library.contains(face, self.encoding))
    }
    pub fn set_user_directory(&self, directory: &Path) -> Result<(), FormatError> {
        self.library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?
            .set_user_directory(directory);
        self.library_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    pub fn family_name(&self, face: &[u8]) -> Result<String, FormatError> {
        self.library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?
            .family_name(face, self.encoding)
    }
    pub fn select_unicode_face(&self, name: &str) -> Result<Vec<u8>, FormatError> {
        let mut library = self
            .library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?;
        let face = library.register_unicode(name)?;
        library.face(&face, self.encoding)?;
        self.library_generation.fetch_add(1, Ordering::Relaxed);
        Ok(face)
    }
    pub fn import_font(
        &self,
        path: &Path,
        index: u32,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_FONT as u64 + 1)
            .read_to_end(&mut bytes)?;
        // An imported font is named in the game's face field only when it fits there.
        let encoder = codec(self.encoding)?;
        let name = crate::font_library::family_names(path)?
            .into_iter()
            .find(|(face, _)| *face == index)
            .and_then(|(_, names)| {
                names.into_iter().find_map(|name| {
                    if name.chars().any(char::is_control) {
                        return None;
                    }
                    let (encoded, _, unmappable) = encoder.encode(&name);
                    (!unmappable
                        && !encoded.is_empty()
                        && encoded.len() <= 31
                        && !encoded.contains(&0)
                        && !encoded.starts_with(b"CMVS-F-")
                        && !encoded.starts_with(b"CMVS-U-"))
                    .then(|| encoded.into_owned())
                })
            });
        let mut library = self
            .library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))?;
        let key = library.import_file(bytes, index)?;
        let out = if let Some(name) = name {
            library.name_imported_font(&name, &key)?;
            name
        } else {
            key
        };
        self.library_generation.fetch_add(1, Ordering::Relaxed);
        Ok(out)
    }
    fn font(&self, face: &[u8], ch: char) -> Result<Arc<FontVec>, FormatError> {
        self.library
            .lock()
            .map_err(|_| FormatError("font library lock poisoned"))
            .map(|mut library| library.for_character(face, self.encoding, ch))
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
            u32::from(glyph.revision_alpha),
        ];
        let key = (glyph.face.clone(), key);
        if let Some(image) = self.cache.get(&key) {
            return Ok(image.clone());
        }
        let ch = character(self.encoding, glyph.code)?;
        let font = self.font(&glyph.face, ch)?;
        let id = font.glyph_id(ch);
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
        if glyph.revision_alpha {
            for alpha in &mut mask {
                *alpha = revise_alpha(*alpha);
            }
        }
        if glyph.bold {
            embolden(&mut mask, side);
        }
        let image = paint(&mask, side, glyph.colors, glyph.gradient, glyph.outline)?;
        self.cache.insert(key, image.clone());
        Ok(image)
    }

    /// Adds glyph images and visible message-window objects to the core's layer queue.
    /// Text coordinates match the logical device canvas; scales must be finite and nonnegative.
    pub fn render(
        &mut self,
        scene: &SceneHost,
        cameras: &CameraHost,
        messages: &MessageHost,
    ) -> Result<RgbaImage, FormatError> {
        self.compose(scene, cameras, messages)?
            .prepare()?
            .render_cpu()
    }

    /// Combine scene state, rasterized glyphs and visible message windows.
    /// Prepare the returned composition for either CPU or GPU rendering.
    pub fn compose<'a>(
        &mut self,
        scene: &'a SceneHost,
        cameras: &'a CameraHost,
        messages: &'a MessageHost,
    ) -> Result<SceneComposition<'a>, FormatError> {
        Ok(SceneComposition {
            scene,
            cameras,
            messages,
            glyphs: self.prepare_objects(messages)?,
        })
    }

    /// Builds glyph objects without compositing a frame.
    /// Visible message windows must also be passed to the shared scene queue.
    /// Prefer [`Self::compose`] when preparing a complete game scene.
    pub fn prepare_objects(
        &mut self,
        messages: &MessageHost,
    ) -> Result<Vec<SceneObject>, FormatError> {
        let fingerprint = self.glyph_scene_fingerprint(messages);
        if let (Some(fp), Some(cached)) = (fingerprint, self.scene_cache.as_ref())
            && cached.fingerprint == fp
        {
            #[cfg(test)]
            {
                self.scene_cache_hits += 1;
            }
            return Ok(cached.objects.clone());
        }
        #[cfg(test)]
        {
            self.glyph_build_calls += 1;
        }
        let objects = messages.build_glyph_objects(self)?;
        if let Some(fingerprint) = fingerprint {
            self.scene_cache = Some(CachedGlyphScene {
                fingerprint,
                objects: objects.clone(),
            });
        }
        Ok(objects)
    }

    fn glyph_scene_fingerprint(&self, messages: &MessageHost) -> Option<u128> {
        let mut h = GlyphSceneHasher::new();
        (
            self.encoding,
            self.library_generation.load(Ordering::Relaxed),
            messages.screen_size(),
            messages.allows_logical_text_scale(),
        )
            .hash(&mut h);

        let mut visible_glyphs = 0usize;
        for slot in 0..TEXT_OBJECTS {
            let Some(text) = messages
                .text(slot)
                .filter(|t| t.visible && !t.glyphs.is_empty())
            else {
                continue;
            };
            let scale = f32::from_bits(text.scale_bits);
            if !scale.is_finite() || scale < 0.0 {
                return None;
            }
            (
                0x11u8,
                slot as u8,
                text.scale_bits,
                text.logical_size,
                text.position,
                text.layer,
            )
                .hash(&mut h);
            for glyph in text.glyphs.iter().filter(|g| g.shown && g.opacity != 0) {
                visible_glyphs += 1;
                if visible_glyphs > 512 {
                    return None;
                }
                (
                    0x12u8,
                    glyph.face.as_ref(),
                    glyph.bold,
                    glyph.revision_alpha,
                    glyph.code,
                    glyph.position,
                    glyph.size,
                    glyph.colors,
                    glyph.gradient,
                    glyph.outline,
                    glyph.opacity,
                    glyph.reveal_width,
                )
                    .hash(&mut h);
            }
            0x1fu8.hash(&mut h);
        }

        for slot in 0..AUXILIARY_TEXTS {
            let Some(group) = messages.auxiliary_text(slot) else {
                continue;
            };
            let mut attached = group.attached_glyphs().peekable();
            if attached.peek().is_none() {
                continue;
            }
            let Ok(origin) = group.glyph_origin() else {
                return None;
            };
            let texture_mode = group
                .object
                .draw
                .as_ref()
                .map_or(0, |draw| draw.texture_mode);
            (0x21u8, slot as u8, origin, texture_mode).hash(&mut h);
            match &group.secondary_image {
                None => 0u8.hash(&mut h),
                Some(img) => (
                    1u8,
                    img.filter,
                    img.name.as_slice(),
                    img.pixels.cache_key(),
                    img.alpha,
                    img.surface,
                )
                    .hash(&mut h),
            }
            for glyph in attached {
                (
                    0x22u8,
                    glyph.face.as_ref(),
                    glyph.revision_alpha,
                    glyph.code,
                    glyph.position,
                    glyph.size,
                    glyph.colors,
                    glyph.gradient,
                    glyph.outline,
                    glyph.layer,
                )
                    .hash(&mut h);
            }
            0x2fu8.hash(&mut h);
        }

        Some(h.finish128())
    }
}

impl GlyphRasterizer for FontRenderer {
    fn rasterize_glyph(&mut self, glyph: &RecordedGlyph) -> Result<RgbaImage, FormatError> {
        self.glyph(glyph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires CMVS_FONT_PATH; external font"]
    fn display_names_resolve_handles_in_the_game_encoding() {
        let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
        let default = FontRenderer::read(Path::new(&path), 0).unwrap();
        let shared = default.library.clone();
        let mut library = shared.lock().unwrap();
        let handle = library.register_unicode("Noto Sans CJK JP").unwrap();
        let chinese = library.register_unicode("黑体").unwrap();
        let unsupported = library.register_unicode("😀").unwrap();
        drop(library);
        let library = shared;
        use cmvs_core::render::message::GlyphMetrics;
        let metrics = PreviewMetrics(library.clone(), TextEncoding::Gbk);
        assert_eq!(
            metrics.display_face_name(&handle).unwrap(),
            b"Noto Sans CJK JP"
        );
        assert_eq!(
            metrics.display_face_name(&chinese).unwrap(),
            encoding_rs::GBK.encode("黑体").0.as_ref()
        );
        assert_eq!(
            metrics.display_face_name(&unsupported).unwrap(),
            b"Selected font"
        );
        assert!(metrics.display_face_name(b"SimHei").is_none());
        let utf8 = PreviewMetrics(library, TextEncoding::Utf8);
        assert_eq!(utf8.display_face_name(&chinese).unwrap(), "黑体".as_bytes());
    }

    fn recorded_glyph(code: u16) -> RecordedGlyph {
        RecordedGlyph {
            snapshot_indices: None,
            face: Arc::from(cmvs_core::host::settings::DEFAULT_FONT_FACE),
            bold: false,
            revision_alpha: false,
            code,
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
        }
    }

    #[test]
    #[ignore = "requires CMVS_FONT_PATH; external font"]
    fn clones_keep_their_own_face_encoding() {
        let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
        let sjis = FontRenderer::read(Path::new(&path), 0).unwrap();
        let gbk = sjis.clone().with_text_encoding(TextEncoding::Gbk);
        let name = encoding_rs::GBK.encode("黑体").0;
        assert_eq!(gbk.family_name(&name).unwrap(), "黑体");
        assert_ne!(sjis.family_name(&name).ok().as_deref(), Some("黑体"));
        let meiryo = encoding_rs::SHIFT_JIS.encode("メイリオ").0;
        assert_eq!(sjis.family_name(&meiryo).unwrap(), "メイリオ");
    }

    #[test]
    #[ignore = "requires CMVS_FONT_PATH; synthetic glyphs and external font"]
    fn bold_glyph_uses_a_distinct_cache_entry_and_increases_ink() {
        let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
        let mut font = FontRenderer::read(Path::new(&path), 0).unwrap();
        let mut glyph = recorded_glyph(0x82a0);
        let normal = font.glyph(&glyph).unwrap();
        glyph.bold = true;
        let bold = font.glyph(&glyph).unwrap();
        let ink = |image: &RgbaImage| {
            image
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| u64::from(p[3]))
                .sum::<u64>()
        };
        assert!(ink(&bold) > ink(&normal));
        for (before, after) in normal
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(bold.pixels().as_chunks::<4>().0.iter())
        {
            assert!(after[3] >= before[3]);
        }
        glyph.revision_alpha = true;
        let revised = font.glyph(&glyph).unwrap();
        assert!(ink(&revised) > ink(&bold));
        glyph.revision_alpha = false;
        assert_eq!(font.glyph(&glyph).unwrap(), bold);
        glyph.bold = false;
        assert_eq!(font.glyph(&glyph).unwrap(), normal);
        assert_eq!(font.cache.len(), 3);
        let metrics = font.metrics();
        let face = cmvs_core::host::settings::DEFAULT_FONT_FACE;
        let width = metrics.ink_width(face, 65, 27).unwrap();
        assert_eq!(
            metrics.styled_ink_width(face, 65, 27, true).unwrap(),
            (width + 1).min(27)
        );
        assert_eq!(metrics.styled_ink_width(face, 32, 27, true).unwrap(), 0);
        // An unavailable selected face must use the same fallback for measuring
        // and painting, including glyphs already cached under the default face.
        glyph.face = Arc::from(&b"CMVS unavailable regression font"[..]);
        assert_eq!(font.glyph(&glyph).unwrap(), normal);
        assert_eq!(
            metrics.ink_width(&glyph.face, glyph.code, glyph.size),
            metrics.ink_width(face, glyph.code, glyph.size)
        );
        // Keep one style hot while more than a cacheful of other colors is painted.
        // Sharing its original pixel allocation proves it was not rasterized again.
        let hot = font.glyph(&glyph).unwrap();
        for color in 0..1024 {
            glyph.colors = [color; 2];
            font.glyph(&glyph).unwrap();
            glyph.colors = [0xffffff; 2];
            let hit = font.glyph(&glyph).unwrap();
            assert!(std::ptr::eq(hit.pixels(), hot.pixels()));
        }
        assert_eq!(font.cache.len(), 512);
    }

    #[test]
    #[ignore = "requires CMVS_FONT_PATH; synthetic full page and external font"]
    fn repeated_full_page_reuses_styled_glyph_pixels() {
        let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
        let mut font = FontRenderer::read(Path::new(&path), 0).unwrap();
        let mut glyph = recorded_glyph(u16::from(b'A'));
        let originals: Vec<_> = (0..512)
            .map(|color| {
                glyph.colors = [color; 2];
                font.glyph(&glyph).unwrap()
            })
            .collect();
        for _ in 0..3 {
            for (color, original) in originals.iter().enumerate() {
                glyph.colors = [color as u32; 2];
                let hit = font.glyph(&glyph).unwrap();
                assert!(std::ptr::eq(hit.pixels(), original.pixels()));
            }
        }
    }

    #[test]
    fn alpha_revision_preserves_endpoints_and_increases_partial_coverage() {
        assert_eq!(revise_alpha(0), 0);
        assert_eq!(revise_alpha(255), 255);
        assert_eq!(revise_alpha(128), 192);
        for level in 1..16 {
            let alpha = (level * 255 / 16) as u8;
            assert!(revise_alpha(alpha) > alpha);
        }
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
        assert_eq!(character(TextEncoding::ShiftJis, 0x82a0).unwrap(), 'あ');
        assert_eq!(character(TextEncoding::ShiftJis, 0xffb1).unwrap(), 'ｱ');
        assert_eq!(character(TextEncoding::ShiftJis, 65).unwrap(), 'A');
        assert!(character(TextEncoding::ShiftJis, 0).is_err());
        assert!(character(TextEncoding::ShiftJis, 0x817f).is_err());
    }
    #[test]
    fn gbk_glyph_codes_decode_without_changing_the_u16_model() {
        assert_eq!(character(TextEncoding::Gbk, 0xd6d0).unwrap(), '中');
        assert_eq!(character(TextEncoding::Gbk, 0xcec4).unwrap(), '文');
        assert_eq!(character(TextEncoding::Gbk, 65).unwrap(), 'A');
        assert!(character(TextEncoding::Gbk, 0).is_err());
        assert!(character(TextEncoding::Gbk, 0x817f).is_err());
        // A GBK pair remains two bytes even in the sign-extended SJIS range.
        assert!(character(TextEncoding::Gbk, 0xffb1).is_err());
    }

    #[test]
    fn utf8_glyph_codes_are_unicode_scalars() {
        assert_eq!(character(TextEncoding::Utf8, 0x3042).unwrap(), 'あ');
        assert_eq!(character(TextEncoding::Utf8, 0x00e9).unwrap(), 'é');
        assert_eq!(character(TextEncoding::Utf8, 65).unwrap(), 'A');
        assert!(character(TextEncoding::Utf8, 0).is_err());
        assert!(character(TextEncoding::Utf8, 0xd800).is_err());
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

    fn minimal_test_ttf() -> Vec<u8> {
        let mut cmap = Vec::new();
        for v in [0u16, 1, 3, 1] {
            cmap.extend(v.to_be_bytes());
        }
        cmap.extend(12u32.to_be_bytes());
        for v in [
            4u16,
            32,
            0,
            4,
            4,
            1,
            0,
            0x0041,
            0xffff,
            0,
            0x0041,
            0xffff,
            (-64i16) as u16,
            1,
            0,
            0,
        ] {
            cmap.extend(v.to_be_bytes());
        }
        let mut glyf = Vec::new();
        for v in [1i16, 0, 0, 500, 700] {
            glyf.extend(v.to_be_bytes());
        }
        glyf.extend(3u16.to_be_bytes());
        glyf.extend(0u16.to_be_bytes());
        glyf.extend([0x01u8; 4]);
        for v in [0i16, 500, 0, -500, 0, 0, 700, -700] {
            glyf.extend(v.to_be_bytes());
        }
        let mut head = vec![0u8; 54];
        head[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[38..40].copy_from_slice(&1000i16.to_be_bytes());
        head[40..42].copy_from_slice(&1000i16.to_be_bytes());
        head[46..48].copy_from_slice(&8u16.to_be_bytes());
        head[48..50].copy_from_slice(&2i16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[10..12].copy_from_slice(&1000u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let mut hmtx = Vec::new();
        for (adv, lsb) in [(500u16, 0i16), (600u16, 0i16)] {
            hmtx.extend(adv.to_be_bytes());
            hmtx.extend(lsb.to_be_bytes());
        }
        let mut loca = Vec::new();
        for off in [0u16, 0, (glyf.len() / 2) as u16] {
            loca.extend(off.to_be_bytes());
        }
        let mut maxp = vec![0u8; 32];
        maxp[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
        maxp[6..8].copy_from_slice(&4u16.to_be_bytes());
        maxp[8..10].copy_from_slice(&1u16.to_be_bytes());
        let mut name = Vec::new();
        let family_utf16: Vec<u8> = "BenchFont"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        for v in [0u16, 1, 18] {
            name.extend(v.to_be_bytes());
        }
        for v in [3u16, 1, 0x0409, 1, family_utf16.len() as u16, 0] {
            name.extend(v.to_be_bytes());
        }
        name.extend(family_utf16);

        let tables: [(&[u8; 4], Vec<u8>); 8] = [
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"loca", loca),
            (b"maxp", maxp),
            (b"name", name),
        ];
        let mut sfnt = Vec::new();
        sfnt.extend(0x0001_0000u32.to_be_bytes());
        sfnt.extend((tables.len() as u16).to_be_bytes());
        sfnt.extend(128u16.to_be_bytes());
        sfnt.extend(3u16.to_be_bytes());
        sfnt.extend(0u16.to_be_bytes());
        let mut offset = 12 + 16 * tables.len();
        let mut payload = Vec::new();
        for (tag, data) in &tables {
            sfnt.extend_from_slice(*tag);
            sfnt.extend(0u32.to_be_bytes());
            sfnt.extend((offset as u32).to_be_bytes());
            sfnt.extend((data.len() as u32).to_be_bytes());
            payload.extend_from_slice(data);
            let pad = (4 - data.len() % 4) % 4;
            payload.resize(payload.len() + pad, 0);
            offset += data.len() + pad;
        }
        sfnt.extend(payload);
        sfnt
    }

    fn ps2_script(code: &[u8], text: &[u8]) -> cmvs_core::format::ps3::Script {
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend_from_slice(code);
        payload.extend_from_slice(text);
        let mut raw = b"PS2A".to_vec();
        for n in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        cmvs_core::format::ps3::Script::decode(&raw).unwrap()
    }

    fn push_call(code: &mut Vec<u8>, command: u16, words: &[u32]) {
        for &word in words {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(word.to_le_bytes());
        }
        code.extend(command.to_le_bytes());
    }

    #[test]
    fn idle_glyph_scene_cache_hits_and_invalidates_on_reveal_opacity_and_visibility_changes() {
        use cmvs_core::{
            host::input::ActionInput,
            host::settings::TextOutputSettings,
            profile::Semantics,
            vm::dispatch::{self, Hosts},
            vm::machine::{Machine, Step},
        };

        let mut font = FontRenderer::from_bytes(minimal_test_ttf(), 0).unwrap();
        let mut messages = MessageHost::default();
        messages.set_glyph_metrics(font.metrics());
        messages.set_record_output(true);

        // Script:
        // 1) 0x215d create window 0 bound to text 7
        // 2) 0x214e show window 0 (shown = 1)
        // 3) 0x2152 output text string 0 ("AA\0") under reveal_mode 2 (width wipe)
        // 4) 0x214a clear window 0
        // 5) 0x2152 output text string 0 ("AA\0") under reveal_mode 1 (opacity fade)
        // 6) 0x214e hide window 0 (shown = 0)
        // 7) 0x214e show window 0 (shown = 1)
        let mut code = Vec::new();
        push_call(&mut code, 0x215d, &[720, 1280, 120, 400, 0, 0, 7, 0]);
        push_call(&mut code, 0x214e, &[1, 0]);
        push_call(&mut code, 0x2152, &[0, u32::MAX, 0]);
        push_call(&mut code, 0x214a, &[0]);
        push_call(&mut code, 0x2152, &[0, u32::MAX, 0]);
        push_call(&mut code, 0x214e, &[0, 0]);
        push_call(&mut code, 0x214e, &[1, 0]);
        let script = ps2_script(&code, b"AA\0");
        let mut vm = Machine::new(&script).unwrap();

        let step_window_cmd = |vm: &mut Machine, messages: &mut MessageHost| {
            while !matches!(vm.step().unwrap(), Step::System { .. }) {}
            let mut hosts = Hosts {
                messages: Some(messages),
                ..Hosts::default()
            };
            let flow = dispatch::run(Semantics::Root, vm, &mut hosts).unwrap();
            assert!(flow.is_some());
        };
        let step_record_text = |vm: &mut Machine, messages: &mut MessageHost| {
            if vm.pending_system().is_none() {
                while !matches!(vm.step().unwrap(), Step::System { .. }) {}
            }
            let mut input = ActionInput::default();
            messages
                .try_record_output(vm, &mut input, Default::default())
                .unwrap()
                .unwrap();
        };

        // 1) Create window 0 -> text 7 and show window 0.
        step_window_cmd(&mut vm, &mut messages);
        step_window_cmd(&mut vm, &mut messages);

        // Configure width-wipe reveal (mode 2, duration index 1 = 200ms) and emit "AA".
        messages.set_text_settings(TextOutputSettings {
            bold: false,
            revision_alpha: false,
            speed: 3,
            outline: 1,
            reveal_mode: 2,
            reveal_duration: 1,
        });
        step_record_text(&mut vm, &mut messages);

        // Advance 50ms: first glyph is partially revealed by width.
        messages.advance_frame(50);
        let frame_w1 = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 1);
        assert_eq!(font.scene_cache_hits, 0);
        let width_1 = frame_w1[0].children[&0]
            .draw
            .as_ref()
            .unwrap()
            .source_rect()[2];
        assert!(width_1 > 0 && width_1 < 31);

        // Stationary repeat frame reuses the cached SceneObject tree without rebuilding!
        let frame_w1_hit = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 1);
        assert_eq!(font.scene_cache_hits, 1);
        assert_eq!(frame_w1_hit, frame_w1);
        assert!(std::ptr::eq(
            frame_w1_hit[0].children[&0]
                .image
                .as_ref()
                .unwrap()
                .pixels
                .pixels(),
            frame_w1[0].children[&0]
                .image
                .as_ref()
                .unwrap()
                .pixels
                .pixels()
        ));

        // Advance 100ms: `reveal_width` increases and second glyph becomes shown -> invalidates!
        messages.advance_frame(100);
        let frame_w2 = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 2);
        let width_2 = frame_w2[0].children[&0]
            .draw
            .as_ref()
            .unwrap()
            .source_rect()[2];
        assert!(width_2 > width_1);

        // Finish reveal, complete 0x2152, and simulate 5 stationary wait frames at 60 fps -> 5 hits!
        messages.advance_frame(1000);
        step_record_text(&mut vm, &mut messages);
        let settled = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 3);
        let hits_before = font.scene_cache_hits;
        for _ in 0..5 {
            assert_eq!(font.prepare_objects(&messages).unwrap(), settled);
        }
        assert_eq!(font.glyph_build_calls, 3);
        assert_eq!(font.scene_cache_hits, hits_before + 5);

        // Clear window and emit "AA" under opacity-fade reveal (mode 1, duration index 1 = 200ms).
        step_window_cmd(&mut vm, &mut messages);
        messages.set_text_settings(TextOutputSettings {
            bold: false,
            revision_alpha: false,
            speed: 3,
            outline: 1,
            reveal_mode: 1,
            reveal_duration: 1,
        });
        step_record_text(&mut vm, &mut messages);

        messages.advance_frame(50);
        let fade_1 = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 4);
        let opacity_1 = fade_1[0].children[&0].draw.as_ref().unwrap().opacity();
        assert!(opacity_1 > 0 && opacity_1 < 255);

        // Advancing opacity invalidates the cache and updates the glyph SceneObject opacity.
        messages.advance_frame(80);
        let fade_2 = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 5);
        let opacity_2 = fade_2[0].children[&0].draw.as_ref().unwrap().opacity();
        assert!(opacity_2 > opacity_1);

        // Visibility toggle (0x214e hide -> empty glyphs, 0x214e show -> restored glyphs) invalidates!
        messages.advance_frame(1000);
        step_record_text(&mut vm, &mut messages);
        let visible_objs = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 6);
        assert!(!visible_objs.is_empty());

        step_window_cmd(&mut vm, &mut messages); // 0x214e hide
        let hidden_objs = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 7);
        assert!(hidden_objs.is_empty());

        // Repeated hidden frame also hits the cache.
        assert!(font.prepare_objects(&messages).unwrap().is_empty());
        assert_eq!(font.glyph_build_calls, 7);

        step_window_cmd(&mut vm, &mut messages); // 0x214e show
        let reshown_objs = font.prepare_objects(&messages).unwrap();
        assert_eq!(font.glyph_build_calls, 8);
        assert_eq!(reshown_objs, visible_objs);
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

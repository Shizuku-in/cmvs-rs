//! Software presentation of verified static 2D objects, their children, full-screen color
//! fades and mask transitions. Unsupported transforms, solid-color objects and blend modes
//! fail explicitly.
use crate::{
    FormatError, MAX_PAYLOAD,
    raster::RgbaImage,
    scene::{MaskTransition, ObjectImage, SceneHost, SceneObject},
};

enum Layer<'a> {
    Image {
        image: &'a ObjectImage,
        /// Source x/y/width/height.
        rect: [i64; 4],
        /// Top-left destination corner: position minus origin.
        corner: [i64; 2],
        alpha: u32,
        /// The +4632 transition shading this slot object (not its children).
        mask: Option<&'a MaskTransition>,
    },
    Fade(&'a crate::fade::Fade),
}

/// A main queue entry and the children 0x14003de80 attaches to its sublist (flag bit 1).
struct Entry<'a> {
    order: f32,
    layer: Option<Layer<'a>>,
    children: Vec<(f32, Layer<'a>)>,
}

/// Only crop, origin, position, order and opacity may differ from the constructor values.
fn check(words: &[u32; 22]) -> Result<(), FormatError> {
    let mut allowed = crate::scene::DrawState::default().words;
    allowed[2..11].copy_from_slice(&words[2..11]);
    allowed[17] = words[17];
    if *words != allowed {
        return Err(FormatError("unsupported scene transform or blend mode"));
    }
    Ok(())
}

/// Order key as inserted by 0x14003ddd0: the layer word as i32, converted to f32.
fn order(words: &[u32; 22]) -> f32 {
    words[10] as i32 as f32
}

/// Builds an image layer, or None when the object draws nothing (empty rect, zero alpha).
fn image_layer<'a>(
    image: &'a ObjectImage,
    words: &[u32; 22],
    position: [i64; 2],
    alpha: i64,
    mask: Option<&'a MaskTransition>,
) -> Result<Option<Layer<'a>>, FormatError> {
    let [sx, sy, w, h] = [2, 3, 4, 5].map(|i| i64::from(words[i] as i16));
    if w <= 0 || h <= 0 || alpha <= 0 {
        return Ok(None);
    }
    let size = [image.pixels.width(), image.pixels.height()].map(i64::from);
    if sx < 0 || sy < 0 || sx + w > size[0] || sy + h > size[1] {
        return Err(FormatError("scene source rectangle outside image"));
    }
    // The mask is the object's second texture; only the case where it covers the whole,
    // equally sized image texel for texel is modeled.
    if let Some(mask) = mask {
        let mask_size = [mask.mask.pixels.width(), mask.mask.pixels.height()].map(i64::from);
        if [sx, sy, w, h] != [0, 0, size[0], size[1]] || mask_size != size {
            return Err(FormatError(
                "mask transition on a cropped or differently sized image is not verified",
            ));
        }
    }
    Ok(Some(Layer::Image {
        image,
        rect: [sx, sy, w, h],
        corner: [
            position[0] - i64::from(words[6] as i16),
            position[1] - i64::from(words[7] as i16),
        ],
        alpha: alpha.min(255) as u32,
        mask,
    }))
}

/// 0x14003b420/0x140020340 for a child of a top-level object. Without its own image the
/// child uses the parent's (its +56 color has a zero alpha byte, as `check` enforces).
/// Flag bit 0 inherits the parent's position and opacity at unit scale.
fn child_layer<'a>(
    parent: &'a SceneObject,
    parent_words: &[u32; 22],
    child: &'a SceneObject,
    words: &[u32; 22],
) -> Result<Option<Layer<'a>>, FormatError> {
    let image = child
        .image
        .as_ref()
        .or(parent.image.as_ref())
        .ok_or(FormatError(
            "child without an image source uses an unverified solid-color path",
        ))?;
    let own = [words[8] as i32, words[9] as i32].map(i64::from);
    let (position, alpha) = if child.flags & 1 != 0 {
        let (opacity, parent_opacity) = (words[17] as i32, parent_words[17] as i32);
        if opacity < 0 || parent_opacity < 0 {
            return Err(FormatError(
                "negative opacity with parent inheritance is not verified",
            ));
        }
        let base = [parent_words[8] as i32, parent_words[9] as i32].map(i64::from);
        // Fade factors are the constructor's 256, so (256 * 256 * (c * p / 255)) >> 16.
        (
            [base[0] + own[0], base[1] + own[1]],
            i64::from(opacity * parent_opacity / 255),
        )
    } else {
        (own, i64::from(words[17] as i32))
    };
    image_layer(image, words, position, alpha, None)
}

/// Render in script coordinates. Window aspect fitting is a separate desktop policy.
pub fn render(scene: &SceneHost, width: u32, height: u32) -> Result<RgbaImage, FormatError> {
    render_with_objects(scene, &[], width, height)
}

/// Additional objects (e.g. a backend's glyph images and message windows) join the
/// same stable layer queue before the fade. Their order breaks ties after slot objects.
pub fn render_with_objects(
    scene: &SceneHost,
    additional: &[&SceneObject],
    width: u32,
    height: u32,
) -> Result<RgbaImage, FormatError> {
    let size = (u64::from(width) * u64::from(height))
        .checked_mul(4)
        .ok_or(FormatError("scene surface size overflow"))?;
    if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 {
        return Err(FormatError("invalid scene surface dimensions"));
    }
    // Validate the entire scene before rasterizing. Slot order is the stable tie-breaker.
    let mut entries = Vec::new();
    let slots = scene
        .objects()
        .iter()
        .map(|(&slot, object)| (Some(slot), object));
    for (slot, object) in slots.chain(additional.iter().map(|&object| (None, object))) {
        // 0x14003de80 returns early for +6244 == 0; no draw state means +6240 == 0.
        let Some(draw) = object.draw.as_ref().filter(|_| object.shown) else {
            continue;
        };
        let d = &draw.words;
        check(d)?;
        let position = [d[8] as i32, d[9] as i32].map(i64::from);
        let mask = scene.mask().filter(|mask| Some(mask.slot) == slot);
        let layer = match &object.image {
            Some(image) => image_layer(image, d, position, i64::from(d[17] as i32), mask)?,
            None => None,
        };
        let mut entry = Entry {
            order: order(d),
            layer,
            children: Vec::new(),
        };
        let mut independent = Vec::new();
        // Children are visited only once 0x2022 has set the parent's bit 31.
        if object.flags & 0x8000_0000 != 0 {
            for child in object.children.values().filter(|child| child.shown) {
                let words = &child
                    .draw
                    .as_ref()
                    .ok_or(FormatError("shown child object has no draw state"))?
                    .words;
                if words[0] != d[0] || order(words) < 0.0 {
                    continue;
                }
                check(words)?;
                let Some(layer) = child_layer(object, d, child, words)? else {
                    continue;
                };
                if child.flags & 2 != 0 {
                    entry.children.push((order(words), layer));
                } else {
                    independent.push(Entry {
                        order: order(words),
                        layer: Some(layer),
                        children: Vec::new(),
                    });
                }
            }
        }
        entries.push(entry);
        entries.extend(independent);
    }
    // The engine enqueues the fade after ordinary object slots (0x140015da0).
    if let Some(fade) = scene.fade().current() {
        entries.push(Entry {
            order: fade.order as f32,
            layer: Some(Layer::Fade(fade)),
            children: Vec::new(),
        });
    }
    // 0x14003ddd0 inserts after equal keys, with larger orders first; the traversal
    // 0x14003dc40 drains an entry's sublist right after the entry itself.
    entries.sort_by(|a, b| b.order.total_cmp(&a.order));
    let mut pixels = vec![0; size as usize];
    for p in pixels.chunks_exact_mut(4) {
        p.copy_from_slice(&scene.clear().rgba());
    }
    for mut entry in entries {
        entry.children.sort_by(|a, b| b.0.total_cmp(&a.0));
        let layers = entry
            .layer
            .into_iter()
            .chain(entry.children.into_iter().map(|(_, layer)| layer));
        for layer in layers {
            draw(&mut pixels, width, height, layer);
        }
    }
    RgbaImage::new(width, height, pixels)
}

fn draw(pixels: &mut [u8], width: u32, height: u32, layer: Layer<'_>) {
    let (image, [sx, sy, w, h], [left, top], opacity, mask) = match layer {
        Layer::Image {
            image,
            rect,
            corner,
            alpha,
            mask,
        } => (image, rect, corner, alpha, mask),
        Layer::Fade(fade) => {
            let [b, g, r, _] = fade.color.to_le_bytes();
            let alpha = fade.opacity.clamp(0, 255) as u32;
            for p in pixels.chunks_exact_mut(4) {
                for (c, source) in [r, g, b].iter().enumerate() {
                    p[c] = ((u32::from(*source) * alpha + u32::from(p[c]) * (255 - alpha) + 127)
                        / 255) as u8;
                }
            }
            return;
        }
    };
    // Shader factor per mask green byte; the float result is rounded into the 8-bit alpha,
    // so pixels approximate rather than match the device's blending.
    let shade = mask.map(|mask| {
        let coverage: [f32; 256] = std::array::from_fn(|green| mask.coverage(green as u8));
        (mask.mask.pixels.pixels(), coverage)
    });
    for y in top.max(0)..(top + h).min(i64::from(height)) {
        for x in left.max(0)..(left + w).min(i64::from(width)) {
            let src =
                (((sy + y - top) * i64::from(image.pixels.width()) + sx + x - left) * 4) as usize;
            let dst = ((y * i64::from(width) + x) * 4) as usize;
            let p = &image.pixels.pixels()[src..src + 4];
            let mut alpha = (u32::from(p[3]) * opacity + 127) / 255;
            if let Some((texture, coverage)) = &shade {
                // Same size and coordinates as the image, so the same texel.
                alpha = (alpha as f32 * coverage[usize::from(texture[src + 1])]).round() as u32;
            }
            for c in 0..3 {
                pixels[dst + c] =
                    ((u32::from(p[c]) * alpha + u32::from(pixels[dst + c]) * (255 - alpha) + 127)
                        / 255) as u8;
            }
        }
    }
}

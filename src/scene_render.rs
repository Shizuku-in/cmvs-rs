//! Minimal software presentation of verified static, top-level 2D objects.
//! Unsupported transforms, children and blend modes fail explicitly.
use crate::{FormatError, MAX_PAYLOAD, raster::RgbaImage, scene::SceneHost};

/// Render in script coordinates. Window aspect fitting is a separate desktop policy.
pub fn render(scene: &SceneHost, width: u32, height: u32) -> Result<RgbaImage, FormatError> {
    let size = (u64::from(width) * u64::from(height))
        .checked_mul(4)
        .ok_or(FormatError("scene surface size overflow"))?;
    if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 {
        return Err(FormatError("invalid scene surface dimensions"));
    }
    // Validate the entire scene before rasterizing. Slot order is the stable tie-breaker.
    let mut objects = Vec::new();
    for object in scene.objects().values() {
        if !object.children.is_empty() {
            return Err(FormatError("child object rendering is not implemented"));
        }
        let Some(draw) = &object.draw else { continue };
        let d = &draw.words;
        let mut allowed = crate::scene::DrawState::default().words;
        allowed[2..11].copy_from_slice(&d[2..11]);
        allowed[17] = d[17];
        if *d != allowed {
            return Err(FormatError("unsupported scene transform or blend mode"));
        }
        let Some(image) = &object.image else { continue };
        let rect = d[2..6]
            .iter()
            .map(|v| i64::from(*v as i16))
            .collect::<Vec<_>>();
        let [sx, sy, w, h] = rect[..] else {
            unreachable!()
        };
        if w <= 0 || h <= 0 || (d[17] as i32) <= 0 {
            continue;
        }
        if sx < 0
            || sy < 0
            || sx + w > i64::from(image.pixels.width())
            || sy + h > i64::from(image.pixels.height())
        {
            return Err(FormatError("scene source rectangle outside image"));
        }
        objects.push((d[10] as i32 as f32, image, d, [sx, sy, w, h]));
    }
    // 0x14003ddd0 inserts after equal keys, with larger orders first.
    objects.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut pixels = vec![0; size as usize];
    for p in pixels.chunks_exact_mut(4) {
        p.copy_from_slice(&scene.clear().rgba());
    }
    for (_, image, d, [sx, sy, w, h]) in objects {
        let left = i64::from(d[8] as i32) - i64::from(d[6] as i16);
        let top = i64::from(d[9] as i32) - i64::from(d[7] as i16);
        for y in top.max(0)..(top + h).min(i64::from(height)) {
            for x in left.max(0)..(left + w).min(i64::from(width)) {
                let src = (((sy + y - top) * i64::from(image.pixels.width()) + sx + x - left) * 4)
                    as usize;
                let dst = ((y * i64::from(width) + x) * 4) as usize;
                let p = &image.pixels.pixels()[src..src + 4];
                let alpha = (u32::from(p[3]) * d[17].min(255) + 127) / 255;
                for c in 0..3 {
                    pixels[dst + c] = ((u32::from(p[c]) * alpha
                        + u32::from(pixels[dst + c]) * (255 - alpha)
                        + 127)
                        / 255) as u8;
                }
            }
        }
    }
    RgbaImage::new(width, height, pixels)
}

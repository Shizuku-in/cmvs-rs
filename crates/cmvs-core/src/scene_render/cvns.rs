//! Scene preparation and software rasterization for CVNS (`cvns-20090203`) sprites, filters, fades, quake, and masks.

use super::{Layer, RenderFrame};
use crate::{
    FormatError, MAX_PAYLOAD,
    raster::RgbaImage,
    scene::{
        SceneObject,
        cvns::{
            CVNS_IMAGES, CVNS_Z_BUCKETS, CvnsCurtainTile, CvnsImageBuffer, CvnsMaskLayer,
            CvnsMaskTransition, CvnsQuake, CvnsScene, CvnsSprite, CvnsSpriteKind, unpack_rgb,
        },
    },
};

#[derive(Clone, Copy)]
pub(super) struct ShadeParams {
    pub(super) alpha: u16,
    pub(super) tint: [u8; 3],
    pub(super) weight: u16,
    pub(super) additive: bool,
}

#[inline]
fn resolve_shade_params(
    alpha: u16,
    color: u32,
    color_strength: i16,
    blend_mode: i16,
    exempt_fade: bool,
    fade_color: u32,
    fade_level: u16,
) -> ShadeParams {
    let sp_strength = u16::try_from(color_strength.clamp(0, 256)).unwrap_or(256);
    let sp_rgb = unpack_rgb(color);
    let fd_rgb = unpack_rgb(fade_color);
    let (tint, weight) = if exempt_fade || fade_level >= 256 {
        (sp_rgb, sp_strength)
    } else if fade_level == 0 {
        (fd_rgb, 0)
    } else if sp_strength == 256 {
        (fd_rgb, fade_level)
    } else {
        let inv = 256 - u32::from(fade_level);
        let mut mixed = [0u8; 3];
        for c in 0..3 {
            let s = i32::from(sp_rgb[c]);
            let f = i32::from(fd_rgb[c]);
            mixed[c] = (s + ((inv as i32 * (f - s)) >> 8)).clamp(0, 255) as u8;
        }
        let w = sp_strength.saturating_sub(((inv * u32::from(sp_strength)) >> 8) as u16);
        (mixed, w)
    };
    ShadeParams {
        alpha: alpha.min(256),
        tint,
        weight,
        additive: blend_mode == 1,
    }
}

#[inline]
fn resolve_shade(sprite: &CvnsSprite, fade_color: u32, fade_level: u16) -> ShadeParams {
    resolve_shade_params(
        sprite.alpha,
        sprite.color,
        sprite.color_strength,
        sprite.blend_mode,
        (sprite.flags & 1) != 0,
        fade_color,
        fade_level,
    )
}

#[inline]
pub(super) fn apply_tint(rgb: [u8; 3], shade: ShadeParams) -> [u8; 3] {
    if shade.weight >= 256 {
        return rgb;
    }
    if shade.weight == 0 {
        return shade.tint;
    }
    let w = shade.weight;
    let inv = 256 - w;
    [
        ((w * u16::from(rgb[0]) + inv * u16::from(shade.tint[0])) >> 8) as u8,
        ((w * u16::from(rgb[1]) + inv * u16::from(shade.tint[1])) >> 8) as u8,
        ((w * u16::from(rgb[2]) + inv * u16::from(shade.tint[2])) >> 8) as u8,
    ]
}

#[inline]
fn blend_pixel(dst: &mut [u8], src_rgb: [u8; 3], src_alpha: u16, shade: ShadeParams) {
    if src_alpha == 0 || shade.alpha == 0 {
        return;
    }
    let eff_a = if src_alpha == 255 {
        shade.alpha
    } else {
        (shade.alpha * src_alpha) >> 8
    };
    if eff_a == 0 {
        return;
    }
    let c = apply_tint(src_rgb, shade);
    if shade.additive {
        for ch in 0..3 {
            let add = if eff_a >= 256 {
                u16::from(c[ch])
            } else {
                (eff_a * u16::from(c[ch])) >> 8
            };
            dst[ch] = u16::from(dst[ch]).saturating_add(add).min(255) as u8;
        }
        return;
    }
    if eff_a >= 256 {
        dst[0..3].copy_from_slice(&c);
        dst[3] = 255;
        return;
    }
    let inv = 256 - eff_a;
    for ch in 0..3 {
        dst[ch] = ((inv * u16::from(dst[ch]) + eff_a * u16::from(c[ch])) >> 8) as u8;
    }
    dst[3] = 255;
}

#[derive(Clone, Copy)]
pub struct Draw<'a> {
    // Clipped device-space edges: left, top, right, bottom.
    pub(super) bounds: [i32; 4],
    pub(super) shade: ShadeParams,
    pub(super) source: Source<'a>,
}

#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    Solid([u8; 3]),
    Tile {
        image: &'a RgbaImage,
        at: [i32; 2],
        use_alpha: bool,
        image_mask: Option<&'a CvnsMaskLayer>,
        transition: Option<&'a CvnsMaskTransition>,
        slot_mask: Option<&'a RgbaImage>,
    },
    Transformed {
        image: &'a RgbaImage,
        // Texture bounds and UV at the top-left pixel, with 10 fractional bits.
        src_rect: [i32; 4],
        base: [i64; 2],
        step_x: [i64; 2],
        step_y: [i64; 2],
        use_alpha: bool,
    },
}

impl Draw<'_> {
    pub(super) fn texture(&self) -> Option<(&RgbaImage, bool)> {
        match self.source {
            Source::Tile {
                image, use_alpha, ..
            }
            | Source::Transformed {
                image, use_alpha, ..
            } => Some((image, use_alpha)),
            Source::Solid(_) => None,
        }
    }

    fn gpu_supported(&self) -> bool {
        // Normal CVNS alpha blending truncates after every write to the destination.
        // Additive shaders already truncate their contribution before the blend.
        if !self.shade.additive
            && (self.shade.alpha < 256
                || self
                    .texture()
                    .is_some_and(|(image, use_alpha)| use_alpha && !image.is_opaque()))
        {
            return false;
        }
        match &self.source {
            Source::Tile {
                image_mask,
                transition,
                slot_mask,
                ..
            } => image_mask.is_none() && transition.is_none() && slot_mask.is_none(),
            Source::Transformed {
                base,
                step_x,
                step_y,
                ..
            } => {
                let [x0, y0, x1, y1] = self.bounds;
                // Uniforms carry floats, but the fragment shader evaluates UVs as integers.
                (0..2).all(|axis| {
                    let (base, dx, dy) = (base[axis], step_x[axis], step_y[axis]);
                    let limit = 1 << 24;
                    base.abs() <= limit
                        && dx.abs() <= limit
                        && dy.abs() <= limit
                        && base.abs()
                            + dx.abs() * i64::from(x1 - x0 - 1)
                            + dy.abs() * i64::from(y1 - y0 - 1)
                            <= i64::from(i32::MAX)
                })
            }
            Source::Solid(_) => true,
        }
    }

    pub(super) fn render(&self, dst: &mut [u8], width: u32) {
        let shade = self.shade;
        let [x0, y0, x1, y1] = self.bounds;
        let fb_w = width as usize;
        match &self.source {
            Source::Solid(rgb) => {
                for y in y0..y1 {
                    let row = y as usize * fb_w;
                    let pixels = &mut dst[(row + x0 as usize) * 4..(row + x1 as usize) * 4];
                    if shade.alpha == 256 && !shade.additive {
                        let [r, g, b] = apply_tint(*rgb, shade);
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            *pixel = [r, g, b, 255];
                        }
                    } else {
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            blend_pixel(pixel, *rgb, 255, shade);
                        }
                    }
                }
            }
            Source::Tile {
                image,
                at,
                use_alpha,
                image_mask,
                transition,
                slot_mask,
            } => {
                render_tile(
                    dst,
                    fb_w,
                    self.bounds,
                    image,
                    *at,
                    *use_alpha,
                    shade,
                    *image_mask,
                    *transition,
                    *slot_mask,
                );
            }
            Source::Transformed {
                image,
                src_rect,
                base,
                step_x,
                step_y,
                use_alpha,
            } => {
                render_transformed(
                    dst,
                    fb_w,
                    self.bounds,
                    image,
                    *src_rect,
                    *base,
                    *step_x,
                    *step_y,
                    *use_alpha,
                    shade,
                );
            }
        }
    }
}

/// Renders CVNS scene sprites, including effects that capture the current scene.
pub fn render(
    scene: &CvnsScene,
    backdrop_enabled: bool,
    fade_color: u32,
    fade_level: u16,
    width: u32,
    height: u32,
) -> Result<RgbaImage, FormatError> {
    render_with_objects(
        scene,
        backdrop_enabled,
        fade_color,
        fade_level,
        &[],
        width,
        height,
    )
}

/// Renders CVNS sprites together with additional window and glyph objects.
pub fn render_with_objects(
    scene: &CvnsScene,
    backdrop_enabled: bool,
    fade_color: u32,
    fade_level: u16,
    additional: &[&SceneObject],
    width: u32,
    height: u32,
) -> Result<RgbaImage, FormatError> {
    let size = (u64::from(width) * u64::from(height))
        .checked_mul(4)
        .ok_or(FormatError("CVNS scene surface size overflow"))?;
    if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 {
        return Err(FormatError("CVNS scene surface exceeds byte limit"));
    }
    prepare(
        scene,
        backdrop_enabled,
        fade_color,
        fade_level,
        additional,
        width,
        height,
    )
    .render_cpu()
}

pub(super) fn prepare<'a>(
    scene: &'a CvnsScene,
    backdrop_enabled: bool,
    fade_color: u32,
    fade_level: u16,
    additional: &[&'a SceneObject],
    width: u32,
    height: u32,
) -> RenderFrame<'a> {
    enum Object<'a> {
        Sprite(&'a CvnsSprite),
        Additional(&'a SceneObject),
    }

    let [r, g, b] = if backdrop_enabled {
        unpack_rgb(fade_color)
    } else {
        [0; 3]
    };
    let mut objects = Vec::new();
    let valid_z = 0..CVNS_Z_BUCKETS;
    for obj in additional.iter().rev().filter(|obj| obj.shown) {
        if let Some(draw) = &obj.draw {
            let z = draw.words[10] as i32;
            if valid_z.contains(&z) {
                objects.push((z, Object::Additional(obj)));
            }
        }
    }
    for sprite in scene.sprites.iter().rev().flatten() {
        if sprite.visible && !sprite.finished_delete && valid_z.contains(&sprite.z) {
            objects.push((sprite.z, Object::Sprite(sprite)));
        }
    }
    // Stable ordering keeps reverse slot order and puts additional objects first at equal Z.
    objects.sort_by_key(|(z, _)| std::cmp::Reverse(*z));
    let mut commands = Vec::with_capacity(objects.len());
    let (w, h) = (width as i32, height as i32);
    for (_, obj) in objects {
        match obj {
            Object::Sprite(sprite) => {
                draw_sprite(&mut commands, w, h, scene, sprite, fade_color, fade_level)
            }
            Object::Additional(obj) => {
                draw_additional_object(&mut commands, w, h, scene, obj, fade_color, fade_level)
            }
        }
    }
    RenderFrame {
        width,
        height,
        clear: [r, g, b, 255],
        commands,
        precomposited: None,
    }
}

pub(super) fn prepare_gpu(frame: RenderFrame<'_>) -> Result<RenderFrame<'_>, FormatError> {
    if frame
        .commands()
        .iter()
        .any(|layer| matches!(layer, Layer::Cvns(draw) if !draw.gpu_supported()))
    {
        return Ok(RenderFrame::from_precomposited(frame.render_cpu()?));
    }
    Ok(frame)
}

fn draw_sprite<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    scene: &'a CvnsScene,
    sprite: &CvnsSprite,
    fade_color: u32,
    fade_level: u16,
) {
    if sprite.alpha == 0 && sprite.mask_mode < 0 {
        return;
    }
    let shade = resolve_shade(sprite, fade_color, fade_level);
    let qoff = if (sprite.flags & 8) == 0 {
        scene
            .quake
            .as_ref()
            .map_or([0, 0], |q: &CvnsQuake| q.offset_for(false))
    } else {
        [0, 0]
    };
    if let CvnsSpriteKind::SolidRect { color } = sprite.kind {
        if sprite.alpha == 0 {
            return;
        }
        draw_solid_rect(queue, fb_w, fb_h, sprite, color, qoff, shade);
        return;
    }
    if matches!(sprite.kind, CvnsSpriteKind::Video) {
        return;
    }
    let img_idx = sprite.image_slot;
    if img_idx < 0 || (img_idx as usize) >= CVNS_IMAGES {
        return;
    }
    let Some(buf) = scene.images[img_idx as usize].as_ref() else {
        return;
    };
    if let Some(curtain) = &sprite.curtain {
        for tile in &curtain.tiles {
            let mut tile_shade = shade;
            tile_shade.alpha = tile.alpha.min(256);
            draw_image_tile(queue, fb_w, fb_h, buf, tile, qoff, tile_shade, None, None);
        }
        return;
    }
    let trans_mask = if sprite.mask_mode == -200 {
        scene.mask_transition.as_ref()
    } else {
        None
    };
    let slot_mask = if sprite.mask_mode >= 0 && (sprite.mask_mode as usize) < CVNS_IMAGES {
        scene.images[sprite.mask_mode as usize].as_ref()
    } else {
        None
    };
    if sprite.scale_x == 1000 && sprite.scale_y == 1000 && sprite.rotation == 0 {
        let tile = CvnsCurtainTile {
            sx: sprite.src_rect[0],
            sy: sprite.src_rect[1],
            ex: sprite.src_rect[2],
            ey: sprite.src_rect[3],
            dx: sprite.x - sprite.origin_x,
            dy: sprite.y - sprite.origin_y,
            alpha: sprite.alpha,
        };
        draw_image_tile(
            queue, fb_w, fb_h, buf, &tile, qoff, shade, trans_mask, slot_mask,
        );
    } else if sprite.scale_x != 0 && sprite.scale_y != 0 {
        draw_image_transformed(queue, fb_w, fb_h, buf, sprite, qoff, shade);
    }
}

fn draw_solid_rect(
    queue: &mut Vec<Layer<'_>>,
    fb_w: i32,
    fb_h: i32,
    sprite: &CvnsSprite,
    color: u32,
    qoff: [i32; 2],
    shade: ShadeParams,
) {
    let rgb = unpack_rgb(color);
    let sw = sprite.src_rect[2] - sprite.src_rect[0];
    let sh = sprite.src_rect[3] - sprite.src_rect[1];
    if sw <= 0 || sh <= 0 || sprite.scale_x == 0 || sprite.scale_y == 0 {
        return;
    }
    let dx0 = qoff[0] + (sprite.x - sprite.origin_x * sprite.scale_x / 1000);
    let dy0 = qoff[1] + (sprite.y - sprite.origin_y * sprite.scale_y / 1000);
    let dw = sw * sprite.scale_x / 1000;
    let dh = sh * sprite.scale_y / 1000;
    let x0 = dx0.max(0);
    let y0 = dy0.max(0);
    let x1 = (dx0 + dw).min(fb_w);
    let y1 = (dy0 + dh).min(fb_h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    queue.push(Layer::Cvns(Draw {
        bounds: [x0, y0, x1, y1],
        shade,
        source: Source::Solid(rgb),
    }));
}

#[allow(clippy::too_many_arguments)]
fn draw_image_tile<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    buf: &'a CvnsImageBuffer,
    tile: &CvnsCurtainTile,
    qoff: [i32; 2],
    shade: ShadeParams,
    trans_mask: Option<&'a CvnsMaskTransition>,
    slot_mask: Option<&'a CvnsImageBuffer>,
) {
    let Some(img) = buf.display_pixels() else {
        return;
    };
    let use_alpha = buf.base().is_some_and(|l| l.use_alpha);
    draw_rgba_tile(
        queue,
        fb_w,
        fb_h,
        img,
        use_alpha,
        tile,
        qoff,
        shade,
        buf.mask.as_ref(),
        trans_mask,
        slot_mask.and_then(CvnsImageBuffer::display_pixels),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_rgba_tile<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    img: &'a RgbaImage,
    use_alpha: bool,
    tile: &CvnsCurtainTile,
    qoff: [i32; 2],
    shade: ShadeParams,
    img_mask: Option<&'a CvnsMaskLayer>,
    trans_mask: Option<&'a CvnsMaskTransition>,
    slot_mask: Option<&'a RgbaImage>,
) {
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    let mut sx0 = tile.sx;
    let mut sy0 = tile.sy;
    let mut sx1 = tile.ex;
    let mut sy1 = tile.ey;
    let mut dx0 = qoff[0] + tile.dx;
    let mut dy0 = qoff[1] + tile.dy;
    let mut dx1 = dx0 + (sx1 - sx0);
    let mut dy1 = dy0 + (sy1 - sy0);

    if sx0 < 0 {
        dx0 -= sx0;
        sx0 = 0;
    }
    if sy0 < 0 {
        dy0 -= sy0;
        sy0 = 0;
    }
    if sx1 > iw {
        dx1 -= sx1 - iw;
        sx1 = iw;
    }
    if sy1 > ih {
        dy1 -= sy1 - ih;
        sy1 = ih;
    }
    if dx0 < 0 {
        sx0 -= dx0;
        dx0 = 0;
    }
    if dy0 < 0 {
        sy0 -= dy0;
        dy0 = 0;
    }
    if dx1 > fb_w {
        sx1 -= dx1 - fb_w;
        dx1 = fb_w;
    }
    if dy1 > fb_h {
        sy1 -= dy1 - fb_h;
        dy1 = fb_h;
    }
    let cw = (sx1 - sx0).min(dx1 - dx0);
    let ch = (sy1 - sy0).min(dy1 - dy0);
    if cw <= 0 || ch <= 0 {
        return;
    }
    if shade.alpha != 0 || trans_mask.is_some() {
        queue.push(Layer::Cvns(Draw {
            bounds: [dx0, dy0, dx0 + cw, dy0 + ch],
            shade,
            source: Source::Tile {
                image: img,
                at: [sx0, sy0],
                use_alpha,
                image_mask: img_mask,
                transition: trans_mask,
                slot_mask,
            },
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tile(
    dst: &mut [u8],
    fb_w: usize,
    [dx0, dy0, dx1, dy1]: [i32; 4],
    img: &RgbaImage,
    [sx0, sy0]: [i32; 2],
    use_alpha: bool,
    shade: ShadeParams,
    img_mask: Option<&CvnsMaskLayer>,
    trans_mask: Option<&CvnsMaskTransition>,
    slot_mask: Option<&RgbaImage>,
) {
    let iw = img.width() as usize;
    let cw = dx1 - dx0;
    let ch = dy1 - dy0;
    let pixels = img.pixels();
    if !use_alpha
        && shade.alpha == 256
        && shade.weight == 256
        && !shade.additive
        && img_mask.is_none()
        && trans_mask.is_none()
        && slot_mask.is_none()
    {
        let bytes = cw as usize * 4;
        for row in 0..ch {
            let s = ((sy0 + row) as usize * iw + sx0 as usize) * 4;
            let d = ((dy0 + row) as usize * fb_w + dx0 as usize) * 4;
            let line = &mut dst[d..d + bytes];
            line.copy_from_slice(&pixels[s..s + bytes]);
            for alpha in line.iter_mut().skip(3).step_by(4) {
                *alpha = 255;
            }
        }
        return;
    }
    for row in 0..ch {
        let sy = (sy0 + row) as usize;
        let dy = (dy0 + row) as usize;
        for col in 0..cw {
            let sx = (sx0 + col) as usize;
            let dx = (dx0 + col) as usize;
            let s = (sy * iw + sx) * 4;
            let d = (dy * fb_w + dx) * 4;
            if let Some(mt) = trans_mask {
                let mx = sx.min(mt.width.saturating_sub(1) as usize);
                let my = sy.min(mt.height.saturating_sub(1) as usize);
                let m_val = mt.mask[my * mt.width as usize + mx];
                let thresh = mt.thresholds[usize::from(m_val)];
                let mask_alpha = if mt.reveal {
                    if mt.elapsed_ms < thresh {
                        0
                    } else if mt.ramp_ms > 0 && mt.elapsed_ms < mt.ramp_ms + thresh {
                        (((mt.elapsed_ms - thresh) << 8) / mt.ramp_ms).min(256) as u16
                    } else {
                        256
                    }
                } else if mt.elapsed_ms < thresh {
                    256
                } else if mt.ramp_ms > 0 && mt.elapsed_ms < mt.ramp_ms + thresh {
                    (255u32.saturating_sub(255 * (mt.elapsed_ms - thresh) / mt.ramp_ms)) as u16
                } else {
                    0
                };
                if mask_alpha != 0 {
                    let mut m_shade = shade;
                    m_shade.alpha = mask_alpha;
                    blend_pixel(
                        &mut dst[d..d + 4],
                        [pixels[s], pixels[s + 1], pixels[s + 2]],
                        255,
                        m_shade,
                    );
                }
                continue;
            }
            let mut src_a = if use_alpha {
                u16::from(pixels[s + 3])
            } else {
                255
            };
            if let Some(im) = img_mask
                && dx < im.width as usize
                && dy < im.height as usize
            {
                let ma = u16::from(im.data[dy * im.width as usize + dx]);
                src_a = (src_a * ma) >> 8;
            }
            if let Some(sm) = slot_mask
                && (sx as u32) < sm.width()
                && (sy as u32) < sm.height()
            {
                let m_idx = (sy * sm.width() as usize + sx) * 4;
                let ma = u16::from(sm.pixels()[m_idx]);
                src_a = (src_a * ma) >> 8;
            }
            blend_pixel(
                &mut dst[d..d + 4],
                [pixels[s], pixels[s + 1], pixels[s + 2]],
                src_a,
                shade,
            );
        }
    }
}

fn draw_image_transformed<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    buf: &'a CvnsImageBuffer,
    sprite: &CvnsSprite,
    qoff: [i32; 2],
    shade: ShadeParams,
) {
    let Some(img) = buf.display_pixels() else {
        return;
    };
    let use_alpha = buf.base().is_some_and(|l| l.use_alpha);
    draw_rgba_transformed(
        queue,
        fb_w,
        fb_h,
        img,
        use_alpha,
        sprite.src_rect,
        sprite.x,
        sprite.y,
        sprite.origin_x,
        sprite.origin_y,
        sprite.scale_x,
        sprite.scale_y,
        sprite.rotation,
        qoff,
        shade,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_rgba_transformed<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    img: &'a RgbaImage,
    use_alpha: bool,
    src_rect: [i32; 4],
    piv_x: i32,
    piv_y: i32,
    origin_x: i32,
    origin_y: i32,
    scale_x: i32,
    scale_y: i32,
    rotation: i32,
    qoff: [i32; 2],
    shade: ShadeParams,
) {
    let sx0 = src_rect[0];
    let sy0 = src_rect[1];
    let sx1 = src_rect[2];
    let sy1 = src_rect[3];
    let sw = sx1 - sx0;
    let sh = sy1 - sy0;
    if sw <= 0 || sh <= 0 {
        return;
    }
    let avg_scale = ((scale_x + scale_y) / 2).max(1);
    let dst_left: i32;
    let dst_top: i32;
    let dst_w: i32;
    let dst_h: i32;
    let step_xx: i64;
    let step_xy: i64;
    let step_yx: i64;
    let step_yy: i64;
    let mut base_u: i64;
    let mut base_v: i64;
    if rotation == 0 {
        dst_left = qoff[0] + (piv_x - scale_x * origin_x / 1000);
        dst_top = qoff[1] + (piv_y - scale_y * origin_y / 1000);
        dst_w = sw * scale_x / 1000;
        dst_h = sh * scale_y / 1000;
        step_xx = (1_024_000 / scale_x.max(1)) as i64;
        step_yy = (1_024_000 / scale_y.max(1)) as i64;
        step_xy = 0;
        step_yx = 0;
        base_u = 0;
        base_v = 0;
    } else {
        let rev = (360 - rotation) as f64 / 180.0 * std::f64::consts::PI;
        let fwd = rotation as f64 / 180.0 * std::f64::consts::PI;
        let sin_r = (rev.sin() * 1024.0) as i64;
        let cos_r = (rev.cos() * 1024.0) as i64;
        let sin_f = (fwd.sin() * 1024.0) as i32;
        let cos_f = (fwd.cos() * 1024.0) as i32;
        step_xx = 1000 * cos_r / avg_scale as i64;
        step_xy = 1000 * sin_r / avg_scale as i64;
        step_yx =
            1000 * ((std::f64::consts::FRAC_PI_2 + rev).cos() * 1024.0) as i64 / avg_scale as i64;
        step_yy =
            1000 * ((std::f64::consts::FRAC_PI_2 + rev).sin() * 1024.0) as i64 / avg_scale as i64;
        let side = sw.max(sh);
        let box_x = if sw <= sh {
            (piv_x - origin_x) - (sh - sw) / 2
        } else {
            piv_x - origin_x
        };
        let box_y = if sw > sh {
            (piv_y - origin_y) - (sw - sh) / 2
        } else {
            piv_y - origin_y
        };
        let mut min_x = 9999;
        let mut min_y = 9999;
        let mut max_x = 0;
        let mut max_y = 0;
        for &(cx, cy) in &[
            (box_x, box_y),
            (box_x + side, box_y),
            (box_x, box_y + side),
            (box_x + side, box_y + side),
        ] {
            let rx = piv_x + ((cos_f * (cx - piv_x) - sin_f * (cy - piv_y) + 512) >> 10);
            let ry = piv_y + ((sin_f * (cx - piv_x) + cos_f * (cy - piv_y) + 512) >> 10);
            min_x = min_x.min(rx);
            min_y = min_y.min(ry);
            max_x = max_x.max(rx);
            max_y = max_y.max(ry);
        }
        let bw = max_x - min_x;
        let bh = max_y - min_y;
        base_u =
            (i64::from(sw / 2) << 10) + cos_r * i64::from(-(bw / 2)) - sin_r * i64::from(-(bh / 2));
        base_v =
            (i64::from(sh / 2) << 10) + cos_r * i64::from(-(bh / 2)) + sin_r * i64::from(-(bw / 2));
        dst_left = qoff[0] + piv_x - avg_scale * (piv_x - min_x) / 1000;
        dst_top = qoff[1] + piv_y - avg_scale * (piv_y - min_y) / 1000;
        dst_w = avg_scale * bw / 1000 + 1;
        dst_h = avg_scale * bh / 1000 + 1;
    }

    let mut x0 = dst_left;
    let mut y0 = dst_top;
    let x1 = (dst_left + dst_w).min(fb_w);
    let y1 = (dst_top + dst_h).min(fb_h);
    let mut clip_x = 0i64;
    let mut clip_y = 0i64;
    if x0 < 0 {
        clip_x = i64::from(-x0);
        x0 = 0;
    }
    if y0 < 0 {
        clip_y = i64::from(-y0);
        y0 = 0;
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    base_u += step_xx * clip_x + step_yx * clip_y;
    base_v += step_xy * clip_x + step_yy * clip_y;
    if shade.alpha != 0 {
        queue.push(Layer::Cvns(Draw {
            bounds: [x0, y0, x1, y1],
            shade,
            source: Source::Transformed {
                image: img,
                src_rect,
                base: [base_u, base_v],
                step_x: [step_xx, step_xy],
                step_y: [step_yx, step_yy],
                use_alpha,
            },
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_transformed(
    dst: &mut [u8],
    fb_w: usize,
    [x0, y0, x1, y1]: [i32; 4],
    img: &RgbaImage,
    [sx0, sy0, sx1, sy1]: [i32; 4],
    [mut base_u, mut base_v]: [i64; 2],
    [step_xx, step_xy]: [i64; 2],
    [step_yx, step_yy]: [i64; 2],
    use_alpha: bool,
    shade: ShadeParams,
) {
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    let pixels = img.pixels();
    let cols = x1 - x0;
    let rows = y1 - y0;
    for r in 0..rows {
        let mut u = base_u;
        let mut v = base_v;
        let dy = (y0 + r) as usize;
        for c in 0..cols {
            let sx = sx0 + (u >> 10) as i32;
            let sy = sy0 + (v >> 10) as i32;
            if sx >= sx0
                && sx < sx1
                && sy >= sy0
                && sy < sy1
                && sx >= 0
                && sx < iw
                && sy >= 0
                && sy < ih
            {
                let s = (sy as usize * iw as usize + sx as usize) * 4;
                let d = (dy * fb_w + (x0 + c) as usize) * 4;
                let src_a = if use_alpha {
                    u16::from(pixels[s + 3])
                } else {
                    255
                };
                blend_pixel(
                    &mut dst[d..d + 4],
                    [pixels[s], pixels[s + 1], pixels[s + 2]],
                    src_a,
                    shade,
                );
            }
            u += step_xx;
            v += step_xy;
        }
        base_u += step_yx;
        base_v += step_yy;
    }
}

fn draw_additional_object<'a>(
    queue: &mut Vec<Layer<'a>>,
    fb_w: i32,
    fb_h: i32,
    scene: &CvnsScene,
    obj: &'a SceneObject,
    fade_color: u32,
    fade_level: u16,
) {
    if !obj.shown {
        return;
    }
    let qoff = if (obj.flags & 8) == 0 {
        scene
            .quake
            .as_ref()
            .map_or([0, 0], |q: &CvnsQuake| q.offset_for(true))
    } else {
        [0, 0]
    };
    let Some(draw) = obj.draw.as_ref() else {
        return;
    };
    let parent_alpha = if draw.words[17] == 0 {
        256u16
    } else {
        (draw.words[17] as u16).min(256)
    };
    let parent_x = draw.words[8] as i32;
    let parent_y = draw.words[9] as i32;

    if !obj.children.is_empty() {
        for child in obj.children.values() {
            if !child.shown {
                continue;
            }
            let Some(cdraw) = child.draw.as_ref() else {
                continue;
            };
            let Some(cimg) = child.image.as_ref() else {
                continue;
            };
            let raw_alpha = ((cdraw.words[17] * u32::from(parent_alpha)) >> 8) as u16;
            if raw_alpha == 0 {
                continue;
            }
            let alpha = ((u32::from(raw_alpha) * 256) / 255).min(256) as u16;
            let shade = resolve_shade_params(
                alpha,
                0,
                256,
                0,
                (child.flags & 1) != 0,
                fade_color,
                fade_level,
            );
            let w = (cdraw.words[4] as i16 as i32).min(cimg.pixels.width() as i32);
            let h = (cdraw.words[5] as i16 as i32).min(cimg.pixels.height() as i32);
            if w <= 0 || h <= 0 {
                continue;
            }
            let tile = CvnsCurtainTile {
                sx: 0,
                sy: 0,
                ex: w,
                ey: h,
                dx: parent_x + cdraw.words[8] as i32,
                dy: parent_y + cdraw.words[9] as i32,
                alpha,
            };
            draw_rgba_tile(
                queue,
                fb_w,
                fb_h,
                &cimg.pixels,
                cimg.alpha,
                &tile,
                qoff,
                shade,
                None,
                None,
                None,
            );
        }
        return;
    }

    let alpha = (draw.words[17] as u16).min(256);
    if alpha == 0 {
        return;
    }
    let sx = i32::from(draw.words[2] as i16);
    let sy = i32::from(draw.words[3] as i16);
    let w = i32::from(draw.words[4] as i16);
    let h = i32::from(draw.words[5] as i16);
    let ox = i32::from(draw.words[6] as i16);
    let oy = i32::from(draw.words[7] as i16);
    let scale_x = (f32::from_bits(draw.words[18]) * 1000.0).round() as i32;
    let scale_y = (f32::from_bits(draw.words[19]) * 1000.0).round() as i32;
    let scale_x = if draw.words[18] == 0 { 1000 } else { scale_x };
    let scale_y = if draw.words[19] == 0 { 1000 } else { scale_y };
    let rot = draw.words[20] as i32;
    let shade = resolve_shade_params(
        alpha,
        draw.words[14],
        256,
        i16::from(draw.words[16] != 0),
        (obj.flags & 1) != 0,
        fade_color,
        fade_level,
    );
    let Some(img) = obj.image.as_ref() else {
        return;
    };
    if scale_x == 1000 && scale_y == 1000 && rot == 0 {
        let tile = CvnsCurtainTile {
            sx,
            sy,
            ex: sx + w,
            ey: sy + h,
            dx: parent_x - ox,
            dy: parent_y - oy,
            alpha,
        };
        draw_rgba_tile(
            queue,
            fb_w,
            fb_h,
            &img.pixels,
            img.alpha,
            &tile,
            qoff,
            shade,
            None,
            None,
            None,
        );
    } else {
        draw_rgba_transformed(
            queue,
            fb_w,
            fb_h,
            &img.pixels,
            img.alpha,
            [sx, sy, sx + w, sy + h],
            parent_x,
            parent_y,
            ox,
            oy,
            scale_x,
            scale_y,
            rot,
            qoff,
            shade,
        );
    }
}

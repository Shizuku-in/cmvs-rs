//! Device-neutral parameters for GPU compositors executing [`Layer`] queues. The wgpu
//! (WGSL), Switch deko3d (GLSL) and Vita GXM (Cg) scene shaders share this uniform
//! layout, read as `vec4 v[70]`:
//!
//! | floats | meaning |
//! |---|---|
//! | 0..2 | target width/height |
//! | 2 | draw alpha 0..255 |
//! | 3 | 1 = clamped bilinear source sampling |
//! | 4..8 | source x/y/width/height |
//! | 8..16 | device corners in strip order TL, TR, BL, BR |
//! | 16..19 | solid or CVNS tint RGB 0..255 |
//! | 19 | [`flags`] |
//! | 20..23 | CVNS color weight, source alpha toggle and additive toggle |
//! | 24..280 | CMVS mask coverage or CVNS affine UV (base, X step, Y step) |
use super::{Blend, Layer, cvns::Source};
use crate::raster::RgbaImage;

pub const PARAM_FLOATS: usize = 280;
/// Floats read by draws that need neither mask coverage nor affine UVs.
pub const BASE_FLOATS: usize = 24;

pub mod flags {
    /// Solid color instead of the source texture.
    pub const SOLID: u32 = 1;
    /// Texture subtraction: the color becomes black, alpha is kept.
    pub const BLACK: u32 = 2;
    pub const MASK: u32 = 4;
    /// Screen-space second texture scales alpha by its green channel.
    pub const SCREEN: u32 = 8;
    /// Wrapped instead of clamped texel addressing (world billboards).
    pub const WRAP: u32 = 16;
    pub const CVNS: u32 = 32;
    pub const CVNS_AFFINE: u32 = 64;
}

/// Fixed-function blending of a draw; alpha writes are always disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderBlend {
    /// No blending.
    Replace,
    /// SRCALPHA / INVSRCALPHA.
    Alpha,
    /// SRCALPHA / ONE.
    Additive,
}

/// One quad: uniform parameters plus the textures bound at slots 0 (source),
/// 1 (mask) and 2 (screen mask). Unused slots may hold any texture.
pub struct ShaderDraw<'a> {
    pub params: [f32; PARAM_FLOATS],
    pub blend: ShaderBlend,
    pub textures: [Option<&'a RgbaImage>; 3],
}
impl ShaderDraw<'_> {
    pub fn flags(&self) -> u32 {
        self.params[19] as u32
    }
    /// Floats a device must upload.
    pub fn used_params(&self) -> &[f32] {
        if self.flags() & (flags::MASK | flags::CVNS_AFFINE) != 0 {
            &self.params
        } else {
            &self.params[..BASE_FLOATS]
        }
    }
}

/// Translates one validated command for a target of `size` pixels.
pub fn shader_draw<'a>(command: &Layer<'a>, size: [u32; 2]) -> ShaderDraw<'a> {
    let [w, h] = size;
    let mut p = [0f32; PARAM_FLOATS];
    p[..2].copy_from_slice(&[w as f32, h as f32]);
    let mut source = None;
    let mut mask = None;
    let mut screen = None;
    let mut flag = 0;
    let (corners, alpha, blend) = match command {
        Layer::Cvns(draw) => {
            flag |= flags::CVNS;
            let [l, t, r, b] = draw.bounds.map(|v| v as f32);
            let shade = draw.shade;
            p[16..19].copy_from_slice(&shade.tint.map(f32::from));
            p[20] = f32::from(shade.weight);
            let use_alpha = match draw.source {
                Source::Solid(rgb) => {
                    p[16..19].copy_from_slice(&super::cvns::apply_tint(rgb, shade).map(f32::from));
                    p[20] = 256.0;
                    flag |= flags::SOLID;
                    false
                }
                Source::Tile {
                    image,
                    at: [x, y],
                    use_alpha,
                    ..
                } => {
                    source = Some(image);
                    p[4..8].copy_from_slice(&[x as f32, y as f32, r - l, b - t]);
                    use_alpha
                }
                Source::Transformed {
                    image,
                    src_rect,
                    base,
                    step_x,
                    step_y,
                    use_alpha,
                } => {
                    source = Some(image);
                    flag |= flags::CVNS_AFFINE;
                    p[4..8].copy_from_slice(&src_rect.map(|v| v as f32));
                    for (i, pair) in [base, step_x, step_y].into_iter().enumerate() {
                        p[24 + 2 * i..26 + 2 * i].copy_from_slice(&pair.map(|v| v as f32));
                    }
                    use_alpha
                }
            };
            p[21] = u8::from(use_alpha) as f32;
            p[22] = u8::from(shade.additive) as f32;
            let blend = if shade.additive {
                Blend::Additive
            } else if !use_alpha && shade.alpha == 256 {
                Blend::Replace
            } else {
                Blend::Alpha
            };
            (
                [[l, t], [r, t], [l, b], [r, b]],
                u32::from(shade.alpha),
                blend,
            )
        }
        Layer::Image {
            image,
            rect,
            corner,
            alpha,
            mask: m,
            screen_mask: s,
            blend,
        } => {
            source = Some(&image.pixels);
            mask = *m;
            screen = s.map(|s| &s.pixels);
            p[4..8].copy_from_slice(&rect.map(|v| v as f32));
            let [x, y] = corner.map(|v| v as f32);
            let [cw, ch] = [rect[2] as f32, rect[3] as f32];
            (
                [[x, y], [x + cw, y], [x, y + ch], [x + cw, y + ch]],
                *alpha,
                *blend,
            )
        }
        Layer::Billboard { image, quad, alpha } => {
            source = Some(&image.pixels);
            p[4..8].copy_from_slice(&[
                0.0,
                0.0,
                image.pixels.width() as f32,
                image.pixels.height() as f32,
            ]);
            p[3] = 1.0;
            flag |= flags::WRAP;
            let [l, t, r, b] = quad.map(|v| v + 0.5);
            ([[l, t], [r, t], [l, b], [r, b]], *alpha, Blend::Alpha)
        }
        Layer::Projected {
            image,
            rect,
            quad,
            alpha,
            blend,
        } => {
            source = Some(&image.pixels);
            p[4..8].copy_from_slice(&rect.map(|v| v as f32));
            p[3] = 1.0;
            let [l, t, r, b] = quad.map(|v| v + 0.5);
            ([[l, t], [r, t], [l, b], [r, b]], *alpha, *blend)
        }
        Layer::ProjectedMasked {
            image,
            rect,
            quad,
            alpha,
            mask: m,
            blend,
        } => {
            source = Some(&image.pixels);
            mask = Some(*m);
            p[4..8].copy_from_slice(&rect.map(|v| v as f32));
            p[3] = 1.0;
            let [l, t, r, b] = quad.map(|v| v + 0.5);
            ([[l, t], [r, t], [l, b], [r, b]], *alpha, *blend)
        }
        Layer::Rotated {
            image,
            rect,
            corners,
            alpha,
            mask: m,
            blend,
        } => {
            source = Some(&image.pixels);
            mask = *m;
            p[4..8].copy_from_slice(rect);
            p[3] = 1.0;
            (corners.map(|c| c.map(|v| v + 0.5)), *alpha, *blend)
        }
        Layer::Solid {
            rect,
            color,
            alpha,
            screen_mask,
            blend,
        } => {
            screen = screen_mask.map(|s| &s.pixels);
            p[16..19].copy_from_slice(&color.map(f32::from));
            flag |= flags::SOLID;
            let [x, y, cw, ch] = rect.map(|v| v as f32);
            (
                [[x, y], [x + cw, y], [x, y + ch], [x + cw, y + ch]],
                *alpha,
                *blend,
            )
        }
        Layer::Fade(fade) => {
            let [b, g, r, _] = fade.color.to_le_bytes();
            p[16..19].copy_from_slice(&[r, g, b].map(f32::from));
            flag |= flags::SOLID;
            (
                [
                    [0.0, 0.0],
                    [w as f32, 0.0],
                    [0.0, h as f32],
                    [w as f32, h as f32],
                ],
                fade.opacity.clamp(0, 255) as u32,
                Blend::Alpha,
            )
        }
    };
    p[2] = alpha as f32;
    for (i, c) in corners.iter().enumerate() {
        p[8 + i * 2..10 + i * 2].copy_from_slice(c);
    }
    let blend_state = match blend {
        Blend::Replace | Blend::BlackReplace => ShaderBlend::Replace,
        Blend::Alpha | Blend::BlackAlpha => ShaderBlend::Alpha,
        Blend::Additive | Blend::BlackAdditive => ShaderBlend::Additive,
    };
    if matches!(
        blend,
        Blend::BlackReplace | Blend::BlackAlpha | Blend::BlackAdditive
    ) {
        flag |= flags::BLACK;
    }
    if let Some(m) = mask {
        flag |= flags::MASK;
        for green in 0..256 {
            p[BASE_FLOATS + green] = m.coverage(green as u8);
        }
    }
    if screen.is_some() {
        flag |= flags::SCREEN;
    }
    p[19] = flag as f32;
    ShaderDraw {
        params: p,
        blend: blend_state,
        textures: [source, mask.map(|m| &m.mask.pixels), screen],
    }
}

/// Builds a full-target opaque draw for a [`super::RenderFrame::precomposited`] image.
pub fn precomposited_draw<'a>(image: &'a RgbaImage, size: [u32; 2]) -> ShaderDraw<'a> {
    let [w, h] = size.map(|v| v as f32);
    let [sw, sh] = [image.width() as f32, image.height() as f32];
    let mut p = [0f32; PARAM_FLOATS];
    p[0] = w;
    p[1] = h;
    p[2] = 255.0;
    p[4..8].copy_from_slice(&[0.0, 0.0, sw, sh]);
    p[8..16].copy_from_slice(&[0.0, 0.0, w, 0.0, 0.0, h, w, h]);
    ShaderDraw {
        params: p,
        blend: ShaderBlend::Replace,
        textures: [Some(image), None, None],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solid_rect_packs_corners_color_and_skips_the_coverage_table() {
        let draw = shader_draw(
            &Layer::Solid {
                rect: [2, 3, 4, 5],
                color: [10, 20, 30],
                alpha: 200,
                screen_mask: None,
                blend: Blend::BlackAdditive,
            },
            [16, 9],
        );
        assert_eq!(draw.blend, ShaderBlend::Additive);
        assert_eq!(draw.flags(), flags::SOLID | flags::BLACK);
        assert_eq!(&draw.params[..3], &[16.0, 9.0, 200.0]);
        assert_eq!(
            &draw.params[8..16],
            &[2.0, 3.0, 6.0, 3.0, 2.0, 8.0, 6.0, 8.0]
        );
        assert_eq!(&draw.params[16..19], &[10.0, 20.0, 30.0]);
        assert_eq!(draw.used_params().len(), BASE_FLOATS);
        assert!(draw.textures.iter().all(Option::is_none));
    }
}

//! Device-neutral parameters for GPU compositors executing [`Layer`] queues. The wgpu
//! (WGSL) and Switch deko3d (GLSL) scene shaders share this uniform layout, read as
//! `vec4 v[70]`:
//!
//! | floats | meaning |
//! |---|---|
//! | 0..2 | target width/height |
//! | 2 | draw alpha 0..255 |
//! | 3 | 1 = clamped bilinear source sampling |
//! | 4..8 | source x/y/width/height |
//! | 8..16 | device corners in strip order TL, TR, BL, BR |
//! | 16..19 | solid RGB 0..255 |
//! | 19 | [`flags`] |
//! | 24..280 | mask coverage per green value, only read with [`flags::MASK`] |
use super::{Blend, Layer};
use crate::raster::RgbaImage;

pub const PARAM_FLOATS: usize = 280;
/// Floats read when [`flags::MASK`] is clear; the coverage table follows them.
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
    /// Floats a device must upload; the coverage table only when it is read.
    pub fn used_params(&self) -> &[f32] {
        if self.flags() & flags::MASK != 0 {
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
        Layer::Rotated {
            image,
            rect,
            corners,
            alpha,
            blend,
        } => {
            source = Some(&image.pixels);
            p[4..8].copy_from_slice(&rect.map(|v| v as f32));
            p[3] = 1.0;
            (corners.map(|c| c.map(|v| v + 0.5)), *alpha, *blend)
        }
        Layer::Solid {
            rect,
            color,
            alpha,
            blend,
        } => {
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

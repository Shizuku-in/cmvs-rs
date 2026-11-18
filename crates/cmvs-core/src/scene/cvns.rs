//! CVNS (`cvns-20090203`) 2D image buffers, 1536-slot sprite table, transitions, fade, and quake.

use super::ImageSource;
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow},
    evaluate::error,
    fade::SkipGate,
    image,
    input::ActionInput,
    raster::RgbaImage,
};
use std::{collections::BTreeSet, error::Error};

pub const CVNS_IMAGES: usize = 128;
pub const CVNS_LAYERS: usize = 16;
pub const CVNS_SPRITES: usize = 1536;
pub const CVNS_ANIM_FRAMES: usize = 64;
pub const CVNS_PLAYLISTS: usize = 6;
pub const CVNS_Z_BUCKETS: i32 = 128;

#[inline]
pub(crate) fn unpack_rgb(color: u32) -> [u8; 3] {
    [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    ]
}

#[inline]
pub(crate) fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn empty_rgba(
    width: u32,
    height: u32,
    color: u32,
) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("CVNS image buffer size overflow")?;
    let [r, g, b] = unpack_rgb(color);
    let mut pixels = vec![0u8; count];
    for chunk in pixels.as_chunks_mut::<4>().0 {
        chunk[0] = r;
        chunk[1] = g;
        chunk[2] = b;
        chunk[3] = 255;
    }
    Ok(RgbaImage::new(width, height, pixels)?)
}

fn extract_mask_channel(image: &RgbaImage, channel: usize) -> Vec<u8> {
    image
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| px[channel])
        .collect()
}

#[derive(Debug, Clone)]
pub struct CvnsImageLayer {
    pub name: Vec<u8>,
    pub part: Vec<u8>,
    pub frame: Option<u32>,
    pub fill_color: Option<u32>,
    pub captured: bool,
    pub pixels: RgbaImage,
    pub filtered: Option<RgbaImage>,
    pub use_alpha: bool,
    pub dx: i32,
    pub dy: i32,
    pub filter_mode: u32,
    pub filter_arg: i32,
    pub monochrome: u32,
    pub hsl_from: i32,
    pub hsl_to: i32,
    pub rgb_mul: i32,
}

impl CvnsImageLayer {
    pub(crate) fn from_image(
        name: Vec<u8>,
        part: Vec<u8>,
        pixels: RgbaImage,
        dx: i32,
        dy: i32,
        use_alpha: bool,
    ) -> Self {
        Self {
            name,
            part,
            frame: None,
            fill_color: None,
            captured: false,
            pixels,
            filtered: None,
            use_alpha,
            dx,
            dy,
            filter_mode: 0,
            filter_arg: 0,
            monochrome: 0x00ff_ffff,
            hsl_from: -1,
            hsl_to: -1,
            rgb_mul: -1,
        }
    }

    pub(crate) fn empty_rgba(
        width: u32,
        height: u32,
        color: u32,
    ) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        empty_rgba(width, height, color)
    }

    pub(crate) fn make_mask(name: Vec<u8>, img: &RgbaImage) -> CvnsMaskLayer {
        CvnsMaskLayer {
            name,
            width: img.width(),
            height: img.height(),
            data: extract_mask_channel(img, 2),
        }
    }

    pub(crate) fn apply_saved_effects(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if self.monochrome != 0x00ff_ffff {
            apply_monochrome(self, self.monochrome);
        }
        if self.hsl_from >= 0 && self.hsl_to >= 0 {
            apply_hsl_tint(self, self.hsl_from as u32, self.hsl_to as u32);
        }
        if self.rgb_mul >= 0 {
            apply_rgb_mul(self, self.rgb_mul as u32);
        }
        if self.filter_mode != 0 {
            apply_filter(self, self.filter_mode, self.filter_arg)?;
        }
        Ok(())
    }

    /// Returns the pixel buffer active for compositing (`filtered` when set, else base `pixels`).
    pub fn active_pixels(&self) -> &RgbaImage {
        self.filtered.as_ref().unwrap_or(&self.pixels)
    }

    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    pub fn height(&self) -> u32 {
        self.pixels.height()
    }
}

#[derive(Debug, Clone)]
pub struct CvnsMaskLayer {
    pub name: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Single-byte mask intensity per pixel (`sub_438ba0` extracts blue / index channel).
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CvnsImageBuffer {
    pub layers: Vec<Option<CvnsImageLayer>>,
    pub composited: Option<RgbaImage>,
    pub mask: Option<CvnsMaskLayer>,
}

impl CvnsImageBuffer {
    pub(crate) fn with_base(layer0: CvnsImageLayer) -> Self {
        let mut layers = vec![None; CVNS_LAYERS];
        layers[0] = Some(layer0);
        Self {
            layers,
            composited: None,
            mask: None,
        }
    }

    /// Returns the active display pixels (`composited` when present, else layer 0 active pixels).
    pub fn display_pixels(&self) -> Option<&RgbaImage> {
        if let Some(comp) = &self.composited {
            return Some(comp);
        }
        self.layers[0].as_ref().map(CvnsImageLayer::active_pixels)
    }

    pub fn base(&self) -> Option<&CvnsImageLayer> {
        self.layers[0].as_ref()
    }

    /// `sub_439a90`: composites layers `1..16` over base layer 0, or clears `composited` if none exist.
    pub fn rebuild_composite(&mut self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !self.layers[1..].iter().any(Option::is_some) {
            self.composited = None;
            return Ok(false);
        }
        let Some(base) = self.layers[0].as_ref() else {
            return Ok(false);
        };
        let base_img = base.active_pixels();
        let w = base_img.width() as usize;
        let h = base_img.height() as usize;
        let mut out = base_img.pixels().to_vec();
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        for overlay in self.layers[1..].iter().flatten() {
            let src_img = overlay.active_pixels();
            let sw = src_img.width() as i32;
            let sh = src_img.height() as i32;
            let src_x0 = if overlay.dx < 0 {
                (-overlay.dx).min(sw)
            } else {
                0
            };
            let src_y0 = if overlay.dy < 0 {
                (-overlay.dy).min(sh)
            } else {
                0
            };
            let dst_x0 = overlay.dx.max(0);
            let dst_y0 = overlay.dy.max(0);
            let cw = (sw - src_x0).min(w as i32 - dst_x0);
            let ch = (sh - src_y0).min(h as i32 - dst_y0);
            if cw <= 0 || ch <= 0 {
                continue;
            }
            let src = src_img.pixels();
            for row in 0..ch {
                let sy = (src_y0 + row) as usize * sw as usize;
                let dy = (dst_y0 + row) as usize * w;
                for col in 0..cw {
                    let s = (sy + (src_x0 + col) as usize) * 4;
                    let d = (dy + (dst_x0 + col) as usize) * 4;
                    let a = u16::from(src[s + 3]);
                    if a == 0 {
                        continue;
                    }
                    if a == 255 {
                        out[d..d + 4].copy_from_slice(&src[s..s + 4]);
                    } else {
                        let inv = 256 - a;
                        for c in 0..3 {
                            out[d + c] = ((inv * u16::from(out[d + c]) + a * u16::from(src[s + c]))
                                >> 8) as u8;
                        }
                    }
                }
            }
        }
        self.composited = Some(RgbaImage::new(w as u32, h as u32, out)?);
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CvnsSpriteKind {
    Image,
    Video,
    SolidRect { color: u32 },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CvnsAnimFrame {
    pub image_slot: i16,
    pub sx: i16,
    pub sy: i16,
    pub sw: i16,
    pub sh: i16,
    pub ox: i16,
    pub oy: i16,
    pub duration_ms: i16,
    pub next_frame: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvnsSpriteAnim {
    pub track: i16,
    pub current_frame: i16,
    pub remaining_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvnsMoveTween {
    pub active: bool,
    pub start_x: i32,
    pub start_y: i32,
    pub start_alpha: i32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub ctrl_x: i32,
    pub ctrl_y: i32,
    pub dst_alpha: i32,
    pub flags: u32,
    pub duration_ms: u32,
    pub elapsed_ms: u32,
    pub delay_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvnsCurtainTile {
    pub sx: i32,
    pub sy: i32,
    pub ex: i32,
    pub ey: i32,
    pub dx: i32,
    pub dy: i32,
    pub alpha: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvnsCurtain {
    pub mode: u32,
    pub delete_on_finish: bool,
    pub cols: u32,
    pub rows: u32,
    pub interval_ms: u32,
    pub steps: u32,
    pub band_size: i32,
    pub delay_step: u32,
    pub speed: i32,
    pub elapsed_ms: u32,
    pub wave_phase: u32,
    pub tiles: Vec<CvnsCurtainTile>,
}

#[derive(Debug, Clone)]
pub struct CvnsSprite {
    pub kind: CvnsSpriteKind,
    pub visible: bool,
    pub finished_delete: bool,
    pub dirty: bool,
    pub image_slot: i16,
    pub src_rect: [i32; 4],
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale_x: i32,
    pub scale_y: i32,
    pub rotation: i32,
    pub z: i32,
    pub blend_mode: i16,
    pub color_strength: i16,
    pub color: u32,
    pub alpha: u16,
    pub flags: u32,
    pub mask_mode: i16,
    pub anim: Option<CvnsSpriteAnim>,
    pub playlists: [i16; CVNS_PLAYLISTS],
    pub frames: Vec<CvnsAnimFrame>,
    pub move_tween: Option<CvnsMoveTween>,
    pub curtain: Option<CvnsCurtain>,
}

impl Default for CvnsSprite {
    fn default() -> Self {
        Self {
            kind: CvnsSpriteKind::Image,
            visible: true,
            finished_delete: false,
            dirty: false,
            image_slot: -1,
            src_rect: [0; 4],
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            origin_x: 0,
            origin_y: 0,
            scale_x: 1000,
            scale_y: 1000,
            rotation: 0,
            z: 1,
            blend_mode: 0,
            color_strength: 256,
            color: 0,
            alpha: 256,
            flags: 0,
            mask_mode: -1,
            anim: None,
            playlists: [-1; CVNS_PLAYLISTS],
            frames: vec![CvnsAnimFrame::default(); CVNS_ANIM_FRAMES],
            move_tween: None,
            curtain: None,
        }
    }
}

impl CvnsSprite {
    /// Sets `src_rect`, width, height, and optional `image_slot`, clamping negatives to full bounds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_src_rect(
        &mut self,
        image_slot: i16,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        dims: (i32, i32),
        clamp_neg: bool,
        mark_dirty: bool,
    ) {
        let rx = if clamp_neg && sx < 0 { 0 } else { sx };
        let ry = if clamp_neg && sy < 0 { 0 } else { sy };
        let rw = if clamp_neg && sw < 0 { dims.0 } else { sw };
        let rh = if clamp_neg && sh < 0 { dims.1 } else { sh };
        self.image_slot = image_slot;
        self.src_rect = [rx, ry, rx + rw, ry + rh];
        self.w = rw;
        self.h = rh;
        if mark_dirty {
            self.dirty = true;
        }
    }

    /// Configures a keyframe slot (`0..64`) and optional playlist start track (`0..6`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_anim_frame(
        &mut self,
        frame: usize,
        next_frame: i16,
        duration_ms: i16,
        image_slot: i16,
        sx: i16,
        sy: i16,
        sw: i16,
        sh: i16,
        ox: i16,
        oy: i16,
        playlist: i32,
    ) {
        if (0..CVNS_PLAYLISTS as i32).contains(&playlist) {
            self.playlists[playlist as usize] = frame as i16;
        }
        self.frames[frame] = CvnsAnimFrame {
            image_slot,
            sx,
            sy,
            sw,
            sh,
            ox,
            oy,
            duration_ms,
            next_frame,
        };
    }

    pub(crate) fn apply_anim_frame(&mut self, frame_index: i16) {
        let Some(frame) = self.frames.get(frame_index as usize).copied() else {
            return;
        };
        self.image_slot = frame.image_slot;
        self.src_rect = [
            i32::from(frame.sx),
            i32::from(frame.sy),
            i32::from(frame.sx) + i32::from(frame.sw),
            i32::from(frame.sy) + i32::from(frame.sh),
        ];
        self.origin_x = i32::from(frame.ox);
        self.origin_y = i32::from(frame.oy);
        self.w = i32::from(frame.sw);
        self.h = i32::from(frame.sh);
        self.dirty = true;
    }

    pub(crate) fn start_anim(&mut self, track: i16) {
        let Some(&frame_idx) = self.playlists.get(track as usize) else {
            return;
        };
        let duration = self
            .frames
            .get(frame_idx as usize)
            .map_or(0, |f| i32::from(f.duration_ms).max(0) as u32);
        self.anim = Some(CvnsSpriteAnim {
            track,
            current_frame: frame_idx,
            remaining_ms: duration,
        });
        self.apply_anim_frame(frame_idx);
    }

    pub(crate) fn stop_anim(&mut self) {
        if self.anim.take().is_none() {
            return;
        }
        let default_frame = self.playlists[0];
        if default_frame >= 0 {
            self.apply_anim_frame(default_frame);
        }
        self.dirty = true;
    }

    /// Completes `move_tween` immediately (`sub_435bf0`). Returns `true` when `0x1000_0000` demands deletion.
    fn finish_move(&mut self) -> bool {
        let Some(tw) = self.move_tween.take() else {
            return false;
        };
        if !tw.active {
            return false;
        }
        self.x = tw.dst_x;
        self.y = tw.dst_y;
        self.alpha = tw.dst_alpha.clamp(0, 256) as u16;
        self.dirty = true;
        (tw.flags & 0x1000_0000) != 0
    }

    /// Completes `curtain` immediately (`sub_437af0`), optionally hiding/deleting or revealing cleanly.
    pub(crate) fn finish_curtain(&mut self) {
        let Some(c) = self.curtain.take() else {
            return;
        };
        if c.delete_on_finish {
            self.visible = true;
        }
        if !c.tiles.is_empty() && c.mode >= 256 {
            self.alpha = 256;
        }
        self.dirty = true;
    }

    pub(crate) fn advance(&mut self, delta_ms: u32) {
        if self.finished_delete || self.image_slot < 0 && matches!(self.kind, CvnsSpriteKind::Image)
        {
            return;
        }
        if let Some(mut curtain) = self.curtain.take() {
            if curtain.advance(self, delta_ms) {
                if curtain.delete_on_finish {
                    self.finished_delete = true;
                    return;
                }
            } else {
                self.curtain = Some(curtain);
            }
        }
        if let Some(mut anim) = self.anim.take() {
            if delta_ms < anim.remaining_ms {
                anim.remaining_ms -= delta_ms;
                self.anim = Some(anim);
            } else {
                let next = self
                    .frames
                    .get(anim.current_frame as usize)
                    .map_or(-1, |f| f.next_frame);
                if next < 0 {
                    self.stop_anim();
                    match next {
                        -1 => {
                            self.finished_delete = true;
                            return;
                        }
                        -3 => {
                            self.visible = false;
                            return;
                        }
                        -13..=-11 => self.start_anim(-next - 10),
                        _ => {}
                    }
                } else {
                    anim.current_frame = next;
                    anim.remaining_ms = self
                        .frames
                        .get(next as usize)
                        .map_or(0, |f| i32::from(f.duration_ms).max(0) as u32);
                    self.anim = Some(anim);
                    self.apply_anim_frame(next);
                }
            }
        }
        if let Some(mut tw) = self.move_tween.take()
            && tw.active
        {
            if tw.advance(self, delta_ms) {
                self.x = tw.dst_x;
                self.y = tw.dst_y;
                self.alpha = tw.dst_alpha.clamp(0, 256) as u16;
                self.dirty = true;
                if (tw.flags & 0x1000_0000) != 0 {
                    self.finished_delete = true;
                }
            } else {
                self.move_tween = Some(tw);
            }
        }
    }
}

/// Tenth-degree (`18000 = 180 deg`) polar offset matching `sub_473610` / `sub_4734f0`.
fn polar_offset(sx: i32, sy: i32, dx: i32, dy: i32) -> (i32, i32) {
    let rx = dx - sx;
    let ry = dy - sy;
    let rad = (ry as f64).atan2(rx as f64);
    let mut angle = -(rad / std::f64::consts::PI * 180.0 * 100.0) as i32;
    if angle < 0 {
        angle += 36000;
    }
    let mut deg = 36000 - angle;
    if deg == 36000 {
        deg = 0;
    }
    let dist = if rx != 0 {
        (rx as f64 / rad.cos()) as i32
    } else {
        ry.wrapping_abs()
    };
    (deg, dist)
}

#[inline]
fn sin_cos_scale(angle_tenth_deg: i32, dist: i32) -> (i32, i32) {
    let rad = if angle_tenth_deg != 0 {
        (angle_tenth_deg as f64 / 18000.0) * std::f64::consts::PI
    } else {
        0.0
    };
    (
        (rad.cos() * dist as f64) as i32,
        (rad.sin() * dist as f64) as i32,
    )
}

impl CvnsMoveTween {
    fn advance(&mut self, sprite: &mut CvnsSprite, delta_ms: u32) -> bool {
        let mut dt = delta_ms;
        if self.delay_ms != 0 {
            if self.delay_ms < dt {
                dt -= self.delay_ms;
                self.delay_ms = 0;
            } else {
                self.delay_ms -= dt;
                return false;
            }
        }
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt);
        if self.duration_ms == 0 || self.elapsed_ms >= self.duration_ms {
            return true;
        }
        let elapsed = self.elapsed_ms;
        let dur = self.duration_ms;
        if (self.flags & 0x0100_0000) == 0 {
            if self.dst_x != self.start_x || self.dst_y != self.start_y {
                let mode = (self.flags & 0xff) as u8;
                let (dx, dy) = match mode {
                    1 => {
                        let (ang, dist) =
                            polar_offset(self.start_x, self.start_y, self.dst_x, self.dst_y);
                        let (_, r) = sin_cos_scale(((9000 * elapsed) / dur) as i32, dist);
                        sin_cos_scale(ang, r)
                    }
                    2 => {
                        let (ang, dist) =
                            polar_offset(self.start_x, self.start_y, self.dst_x, self.dst_y);
                        let (_, r) = sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, dist);
                        sin_cos_scale(ang, dist - r)
                    }
                    3..=5 => {
                        let (a1, _) =
                            polar_offset(self.start_x, self.start_y, self.ctrl_x, self.ctrl_y);
                        let (a2, dist) =
                            polar_offset(self.start_x, self.start_y, self.dst_x, self.dst_y);
                        let a = a1 + ((elapsed as i64 * i64::from(a2 - a1)) / dur as i64) as i32;
                        let r = match mode {
                            3 => ((elapsed as i64 * i64::from(dist)) / dur as i64) as i32,
                            4 => sin_cos_scale(((9000 * elapsed) / dur) as i32, dist).1,
                            _ => {
                                dist - sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, dist).1
                            }
                        };
                        sin_cos_scale(a, r)
                    }
                    _ => {
                        let vx = self.dst_x - self.start_x;
                        let vy = self.dst_y - self.start_y;
                        (
                            ((elapsed as i64 * i64::from(vx)) / dur as i64) as i32,
                            ((elapsed as i64 * i64::from(vy)) / dur as i64) as i32,
                        )
                    }
                };
                sprite.x = self.start_x + dx;
                sprite.y = self.start_y + dy;
            }
        } else {
            let (a1, d1) = polar_offset(self.start_x, self.start_y, self.ctrl_x, self.ctrl_y);
            let (a2, d2) = polar_offset(self.ctrl_x, self.ctrl_y, self.dst_x, self.dst_y);
            let total = d1 + d2;
            let raw_dist = ((elapsed as i64 * i64::from(total)) / dur as i64) as i32;
            let mode = (self.flags & 0xff) as u8;
            let pos_along = match mode {
                1 => sin_cos_scale(((9000 * elapsed) / dur) as i32, raw_dist).1,
                2 => {
                    let (_, s) = sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, raw_dist);
                    raw_dist - s
                }
                _ => raw_dist,
            };
            let (ox, oy, dx, dy) = if mode == 2 {
                if raw_dist >= d1 {
                    let (_, s) = sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, raw_dist);
                    let rem = d2 - (raw_dist - d1) + s - raw_dist;
                    let (ux, uy) = sin_cos_scale(a2, rem);
                    (self.ctrl_x, self.ctrl_y, ux, uy)
                } else {
                    let (_, s) = sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, raw_dist);
                    let (ux, uy) = sin_cos_scale(a1, d1 - s);
                    (self.start_x, self.start_y, ux, uy)
                }
            } else if pos_along < d1 {
                let (ux, uy) = sin_cos_scale(a1, pos_along);
                (self.start_x, self.start_y, ux, uy)
            } else {
                let (ux, uy) = sin_cos_scale(a2, pos_along - d1);
                (self.ctrl_x, self.ctrl_y, ux, uy)
            };
            sprite.x = ox + dx;
            sprite.y = oy + dy;
        }
        if self.dst_alpha != self.start_alpha {
            let da = self.dst_alpha - self.start_alpha;
            let alpha_mode = ((self.flags >> 8) & 0xff) as u8;
            let step = match alpha_mode {
                1 => {
                    let s = sin_cos_scale(((9000 * elapsed) / dur) as i32, da.abs()).1;
                    if da < 0 { -s } else { s }
                }
                2 => {
                    let s = da.abs()
                        - sin_cos_scale(9000 - ((9000 * elapsed) / dur) as i32, da.abs()).1;
                    if da < 0 { -s } else { s }
                }
                _ => ((elapsed as i64 * i64::from(da)) / dur as i64) as i32,
            };
            sprite.alpha = (self.start_alpha + step).clamp(0, 256) as u16;
        }
        sprite.dirty = true;
        false
    }
}

impl CvnsCurtain {
    fn new(
        sprite: &CvnsSprite,
        mode: u32,
        interval_ms: u32,
        band_size: i32,
        delay_step: u32,
        speed: i32,
    ) -> Self {
        let sw = sprite.w.max(1);
        let sh = sprite.h.max(1);
        let reveal = mode < 0x100;
        let delete_on_finish = if mode < 0x100 {
            false
        } else {
            !matches!(mode, 2 | 4)
        };
        let (cols, rows, steps) = match mode {
            764 => (1, sh as u32, 0),
            765 => (sw as u32, 1, 0),
            1 | 0x101 => (1, 1, 0),
            0x118 | 0x119 => (
                1,
                sh as u32,
                delay_step.saturating_mul((sw / 2).max(1) as u32),
            ),
            0x132 => {
                let n = band_size.max(1) as u32;
                (n, n, n * n)
            }
            _ => {
                let b = band_size.max(1);
                let vertical = matches!(mode & 0xff, 3 | 5 | 7 | 9);
                let grid = matches!(mode & 0xff, 10..=19);
                if grid {
                    let c = ((sw + b - 1) / b) as u32;
                    let r = ((sh + b - 1) / b) as u32;
                    (c, r, c + r)
                } else if vertical {
                    let c = ((sw + b - 1) / b) as u32;
                    let d = delay_step.max(1);
                    let s = speed.max(1);
                    (c, 1, c.saturating_sub(1) / d + (b / s) as u32)
                } else {
                    let r = ((sh + b - 1) / b) as u32;
                    let d = delay_step.max(1);
                    let s = speed.max(1);
                    (1, r, r.saturating_sub(1) / d + (b / s) as u32)
                }
            }
        };
        let _ = reveal;
        let mut c = Self {
            mode,
            delete_on_finish,
            cols,
            rows,
            interval_ms: interval_ms.max(1),
            steps: steps.max(1),
            band_size: band_size.max(1),
            delay_step: delay_step.max(1),
            speed: speed.max(1),
            elapsed_ms: 0,
            wave_phase: 0,
            tiles: Vec::new(),
        };
        c.update_tiles(sprite);
        c
    }

    fn update_tiles(&mut self, sprite: &CvnsSprite) {
        self.tiles.clear();
        let sx0 = sprite.src_rect[0];
        let sy0 = sprite.src_rect[1];
        let sw = sprite.w.max(0);
        let sh = sprite.h.max(0);
        let dx0 = sprite.x - sprite.origin_x;
        let dy0 = sprite.y - sprite.origin_y;
        if self.mode == 764 || self.mode == 765 {
            let amp = self.speed;
            let freq = self.band_size.max(1);
            let count = if self.mode == 764 { sh } else { sw };
            let phase_step = 360 / freq.max(1);
            self.tiles.reserve(count as usize);
            for i in 0..count {
                let angle = (self.wave_phase as i32 + i * phase_step).rem_euclid(360);
                let rad = (angle as f64) * std::f64::consts::PI / 180.0;
                let off = (rad.sin() * amp as f64) as i32;
                let tile = if self.mode == 764 {
                    CvnsCurtainTile {
                        sx: sx0,
                        sy: sy0 + i,
                        ex: sx0 + sw,
                        ey: sy0 + i + 1,
                        dx: dx0 + off,
                        dy: dy0 + i,
                        alpha: sprite.alpha,
                    }
                } else {
                    CvnsCurtainTile {
                        sx: sx0 + i,
                        sy: sy0,
                        ex: sx0 + i + 1,
                        ey: sy0 + sh,
                        dx: dx0 + i,
                        dy: dy0 + off,
                        alpha: sprite.alpha,
                    }
                };
                self.tiles.push(tile);
            }
            return;
        }
        let dur = self.interval_ms.max(1);
        let t = self.elapsed_ms.min(dur);
        let hide = self.mode >= 0x100;
        let prog = if hide { dur - t } else { t };
        if self.mode == 1 || self.mode == 0x101 {
            self.tiles.push(CvnsCurtainTile {
                sx: sx0,
                sy: sy0,
                ex: sx0 + sw,
                ey: sy0 + sh,
                dx: dx0,
                dy: dy0,
                alpha: ((u32::from(sprite.alpha) * prog) / dur) as u16,
            });
            return;
        }
        let b = self.band_size.max(1);
        let step_num = (self.steps * t / dur) as i32;
        if self.rows == 1 {
            for col in 0..self.cols as i32 {
                let idx = if matches!(self.mode & 0xff, 3 | 7) {
                    col
                } else {
                    self.cols as i32 - 1 - col
                };
                let mut w = (step_num - idx / self.delay_step as i32) * self.speed;
                w = w.clamp(0, b);
                if hide {
                    w = (b - w).max(0);
                }
                let x = (col * b).min(sw);
                let ex = (x + w).min(sw);
                if ex > x {
                    self.tiles.push(CvnsCurtainTile {
                        sx: sx0 + x,
                        sy: sy0,
                        ex: sx0 + ex,
                        ey: sy0 + sh,
                        dx: dx0 + x,
                        dy: dy0,
                        alpha: sprite.alpha,
                    });
                }
            }
        } else {
            for row in 0..self.rows as i32 {
                let idx = if matches!(self.mode & 0xff, 2 | 6) {
                    row
                } else {
                    self.rows as i32 - 1 - row
                };
                let mut h = (step_num - idx / self.delay_step as i32) * self.speed;
                h = h.clamp(0, b);
                if hide {
                    h = (b - h).max(0);
                }
                let y = (row * b).min(sh);
                let ey = (y + h).min(sh);
                if ey > y {
                    self.tiles.push(CvnsCurtainTile {
                        sx: sx0,
                        sy: sy0 + y,
                        ex: sx0 + sw,
                        ey: sy0 + ey,
                        dx: dx0,
                        dy: dy0 + y,
                        alpha: sprite.alpha,
                    });
                }
            }
        }
    }

    fn advance(&mut self, sprite: &mut CvnsSprite, delta_ms: u32) -> bool {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        if self.mode == 764 || self.mode == 765 {
            if self.elapsed_ms >= self.interval_ms {
                self.wave_phase = (self.wave_phase + self.delay_step) % 720;
                self.elapsed_ms = 0;
            }
            self.update_tiles(sprite);
            return false;
        }
        if self.elapsed_ms >= self.interval_ms {
            return true;
        }
        self.update_tiles(sprite);
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvnsFade {
    pub to_normal: bool,
    pub color: u32,
    pub duration_ms: u32,
    pub elapsed_ms: u32,
}

impl CvnsFade {
    fn advance(&mut self, delta_ms: u32) -> (u16, bool) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        if self.duration_ms == 0 || self.elapsed_ms >= self.duration_ms {
            return (if self.to_normal { 256 } else { 0 }, true);
        }
        let frac = ((self.elapsed_ms << 8) / self.duration_ms) as u16;
        let alpha = if self.to_normal {
            frac
        } else {
            256u16.saturating_sub(frac)
        };
        (alpha, false)
    }

    fn finish_alpha(self) -> u16 {
        if self.to_normal { 256 } else { 0 }
    }

    pub fn remaining(&self) -> u32 {
        self.duration_ms.saturating_sub(self.elapsed_ms)
    }
}

#[derive(Debug, Clone)]
pub struct CvnsMaskTransition {
    pub sprite_slot: u16,
    pub reveal: bool,
    pub duration_ms: u32,
    pub ramp_ms: u32,
    pub elapsed_ms: u32,
    pub width: u32,
    pub height: u32,
    /// `sub_445030` extracts green channel / 8-bit index per pixel.
    pub mask: Vec<u8>,
    pub thresholds: [u32; 256],
}

impl CvnsMaskTransition {
    fn new(
        sprite_slot: u16,
        reveal: bool,
        duration_ms: u32,
        ramp_ms: u32,
        image: &RgbaImage,
    ) -> Self {
        let dur = duration_ms.saturating_sub(ramp_ms);
        Self {
            sprite_slot,
            reveal,
            duration_ms: dur,
            ramp_ms,
            elapsed_ms: 0,
            width: image.width(),
            height: image.height(),
            mask: extract_mask_channel(image, 1),
            thresholds: Self::compute_thresholds(dur),
        }
    }

    pub(crate) fn compute_thresholds(duration_ms: u32) -> [u32; 256] {
        let mut thresholds = [0u32; 256];
        for (i, slot) in thresholds.iter_mut().enumerate() {
            *slot = (i as u32 * duration_ms) / 255;
        }
        thresholds
    }

    pub fn total_ms(&self) -> u32 {
        self.duration_ms.saturating_add(self.ramp_ms)
    }

    pub fn remaining(&self) -> u32 {
        self.total_ms().saturating_sub(self.elapsed_ms)
    }

    fn advance(&mut self, delta_ms: u32) -> bool {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        self.elapsed_ms >= self.total_ms()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvnsQuake {
    pub kind: u32,
    pub amplitude: i32,
    pub interval_ms: u32,
    pub count: i32,
    pub phase: u32,
    pub paused: bool,
    pub elapsed_ms: u32,
    pub step_count: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    rng: u32,
}

impl CvnsQuake {
    pub(crate) fn rng(&self) -> u32 {
        self.rng
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_saved(
        kind: u32,
        amplitude: i32,
        interval_ms: u32,
        count: i32,
        phase: u32,
        paused: bool,
        elapsed_ms: u32,
        step_count: i32,
        offset_x: i32,
        offset_y: i32,
        rng: u32,
    ) -> Self {
        Self {
            kind,
            amplitude,
            interval_ms,
            count,
            phase,
            paused,
            elapsed_ms,
            step_count,
            offset_x,
            offset_y,
            rng,
        }
    }

    fn new(kind: u32, amplitude: i32, interval_ms: u32, count: i32) -> Self {
        Self {
            kind,
            amplitude,
            interval_ms,
            count,
            phase: 0,
            paused: false,
            elapsed_ms: 0,
            step_count: 0,
            offset_x: 0,
            offset_y: 0,
            rng: 1,
        }
    }

    fn next_rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(214013).wrapping_add(2531011);
        (self.rng >> 16) & 0x7fff
    }

    /// `sub_445ab0`: returns `(dx, dy)` applying to `target_kind` (1 = window, 2 = background sprite).
    pub fn offset_for(&self, is_window: bool) -> [i32; 2] {
        if self.paused {
            return [0, 0];
        }
        let target = self.kind & 0xf;
        if (is_window && target == 1) || (!is_window && target == 2) {
            [0, 0]
        } else {
            [self.offset_x, self.offset_y]
        }
    }

    fn advance(&mut self, delta_ms: u32) -> bool {
        if self.paused {
            return false;
        }
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        let interval = self.interval_ms.max(1);
        let amp = self.amplitude;
        let mut finished = false;
        if (self.kind & 0xf00) != 0 {
            while self.elapsed_ms >= interval {
                if self.count != -1 && self.count < self.step_count {
                    finished = true;
                }
                self.elapsed_ms -= interval;
                self.phase = (self.phase + 1) % 2;
                self.step_count += 1;
            }
            let pos = if amp != 0 {
                (self.elapsed_ms as i32 * 2 * amp) / interval as i32
            } else {
                0
            };
            let v = if self.phase != 0 {
                if pos >= amp { amp - (pos - amp) } else { pos }
            } else if pos >= amp {
                pos - amp - amp
            } else {
                -pos
            };
            if (self.kind & 0xf0) == 0 {
                self.offset_x = v;
                self.offset_y = 0;
            } else if (self.kind & 0xf0) == 0x10 {
                self.offset_y = v;
                self.offset_x = 0;
            }
        } else {
            while self.elapsed_ms >= interval {
                self.elapsed_ms -= interval;
                match self.kind & 0xf0 {
                    0 => {
                        self.offset_x = if self.offset_x != 0 {
                            -self.offset_x
                        } else {
                            amp
                        };
                    }
                    0x10 => {
                        self.offset_y = if self.offset_y != 0 {
                            -self.offset_y
                        } else {
                            amp
                        };
                    }
                    0x20 if amp > 0 => {
                        let span = amp as u32;
                        let r1 = (self.next_rand() % span) as i32;
                        let r2 = (self.next_rand() % span) as i32;
                        self.offset_x = 2 * r1 - amp;
                        self.offset_y = 2 * r2 - amp;
                    }
                    _ => {}
                }
                if self.count != -1 && self.count < self.step_count {
                    finished = true;
                }
                self.step_count += 1;
            }
        }
        finished
    }
}

#[derive(Debug, Clone)]
pub struct CvnsScene {
    pub images: Vec<Option<CvnsImageBuffer>>,
    pub sprites: Vec<Option<CvnsSprite>>,
    pub fade: Option<CvnsFade>,
    pub mask_transition: Option<CvnsMaskTransition>,
    pub quake: Option<CvnsQuake>,
    pub cg_seen: BTreeSet<Vec<u8>>,
}

impl Default for CvnsScene {
    fn default() -> Self {
        Self {
            images: vec![None; CVNS_IMAGES],
            sprites: vec![None; CVNS_SPRITES],
            fade: None,
            mask_transition: None,
            quake: None,
            cg_seen: BTreeSet::new(),
        }
    }
}

impl CvnsScene {
    fn check_image_slot(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) >= CVNS_IMAGES {
            return Err(error(pc, op, "CVNS image slot outside 0..127"));
        }
        Ok(slot as usize)
    }

    fn check_sprite_slot(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) >= CVNS_SPRITES {
            return Err(error(pc, op, "CVNS sprite slot outside 0..1535"));
        }
        Ok(slot as usize)
    }

    fn require_sprite_mut(
        &mut self,
        pc: usize,
        op: u16,
        slot: u32,
    ) -> Result<&mut CvnsSprite, DecodeError> {
        let idx = Self::check_sprite_slot(pc, op, slot)?;
        self.sprites[idx]
            .as_mut()
            .ok_or_else(|| error(pc, op, "CVNS sprite slot is empty"))
    }

    fn record_cg(&mut self, name: &[u8]) {
        if name.len() < 64 {
            self.cg_seen.insert(name.to_vec());
        }
    }

    /// `0x205a` screen capture install into image `slot`.
    pub fn install_captured_image(
        &mut self,
        slot: u32,
        screen: RgbaImage,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let idx = slot as usize;
        if idx >= CVNS_IMAGES {
            return Err("CVNS capture image slot outside 0..127".into());
        }
        let mut pixels = screen.pixels().to_vec();
        for px in pixels.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let opaque = RgbaImage::new(screen.width(), screen.height(), pixels)?;
        let mut layer =
            CvnsImageLayer::from_image(b" ".to_vec(), b" ".to_vec(), opaque, 0, 0, false);
        layer.captured = true;
        self.images[idx] = Some(CvnsImageBuffer::with_base(layer));
        Ok(())
    }

    /// `0x2189` full-screen capture + mask transition install.
    pub fn install_mask_capture(
        &mut self,
        screen: RgbaImage,
        mask_img: Option<&RgbaImage>,
        duration_ms: u32,
        ramp_ms: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.mask_transition = None;
        self.images[0] = None;
        self.sprites[0] = None;
        let Some(mask) = mask_img else {
            return Ok(false);
        };
        let w = screen.width() as i32;
        let h = screen.height() as i32;
        self.install_captured_image(0, screen)?;
        let mut sprite = CvnsSprite {
            image_slot: 0,
            src_rect: [0, 0, w, h],
            w,
            h,
            z: 0,
            mask_mode: -200,
            ..CvnsSprite::default()
        };
        sprite.dirty = true;
        self.sprites[0] = Some(sprite);
        self.mask_transition = Some(CvnsMaskTransition::new(
            0,
            false,
            duration_ms,
            ramp_ms,
            mask,
        ));
        Ok(true)
    }

    pub fn advance_frame(&mut self, delta_ms: u32, fade_color: &mut u32, fade_level: &mut u16) {
        if let Some(mut quake) = self.quake.take()
            && !quake.advance(delta_ms)
        {
            self.quake = Some(quake);
        }
        if let Some(mut mask) = self.mask_transition.take() {
            if mask.advance(delta_ms) {
                let s = mask.sprite_slot as usize;
                if let Some(sprite) = self.sprites.get_mut(s).and_then(Option::as_mut) {
                    if mask.reveal {
                        sprite.mask_mode = -1;
                        sprite.dirty = true;
                    } else {
                        let img = sprite.image_slot;
                        if img >= 0 && (img as usize) < CVNS_IMAGES {
                            self.images[img as usize] = None;
                        }
                        self.sprites[s] = None;
                    }
                }
            } else {
                self.mask_transition = Some(mask);
            }
        }
        if let Some(mut fade) = self.fade.take() {
            let (alpha, done) = fade.advance(delta_ms);
            *fade_color = fade.color;
            *fade_level = alpha;
            if !done {
                self.fade = Some(fade);
            }
        }
        for slot in &mut self.sprites {
            let Some(sprite) = slot.as_mut() else {
                continue;
            };
            sprite.advance(delta_ms);
            if sprite.finished_delete {
                *slot = None;
            }
        }
    }

    fn stop_mask_transition(&mut self) {
        let Some(mask) = self.mask_transition.take() else {
            return;
        };
        let s = mask.sprite_slot as usize;
        if let Some(sprite) = self.sprites.get_mut(s).and_then(Option::as_mut) {
            if mask.reveal {
                sprite.mask_mode = -1;
                sprite.dirty = true;
            } else {
                let img = sprite.image_slot;
                if img >= 0 && (img as usize) < CVNS_IMAGES {
                    self.images[img as usize] = None;
                }
                self.sprites[s] = None;
            }
        }
    }

    fn apply_layer_op(
        &mut self,
        pc: usize,
        op: u16,
        slot: u32,
        layer: u32,
        mut f: impl FnMut(&mut CvnsImageLayer),
    ) -> Result<(), DecodeError> {
        let idx = Self::check_image_slot(pc, op, slot)?;
        let Some(buf) = self.images[idx].as_mut() else {
            return Err(error(pc, op, "CVNS image slot is empty"));
        };
        if layer >= CVNS_LAYERS as u32 {
            // `sub_439ED0` / `sub_43A060` / `sub_43A300`: loop layers and stop cleanly at the first empty layer.
            for entry in &mut buf.layers {
                let Some(l) = entry.as_mut() else {
                    break;
                };
                f(l);
            }
        } else if let Some(l) = buf.layers[layer as usize].as_mut() {
            f(l);
        }
        Ok(())
    }

    pub(crate) fn handle_image(
        &mut self,
        call: &mut Call<'_>,
        source: &mut dyn ImageSource,
        backdrop_size: [u32; 2],
        backdrop: (bool, u32, u16),
    ) -> Result<Flow, Box<dyn Error + Send + Sync>> {
        let cmd = call.command;
        let op = cmd + 0x1c00;
        match cmd {
            0x450 => {
                let slot = call.argument(4, "image slot")?;
                let w = call.argument(8, "width")?;
                let h = call.argument(12, "height")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                call.complete(0x400c)?;
                let img = empty_rgba(w, h, 0)?;
                let mut layer =
                    CvnsImageLayer::from_image(b" ".to_vec(), b" ".to_vec(), img, 0, 0, false);
                layer.fill_color = Some(0);
                self.images[idx] = Some(CvnsImageBuffer::with_base(layer));
            }
            0x451 | 0x452 | 0x465 => {
                let slot = call.argument(4, "image slot")?;
                let name_ref = call.argument(8, "name")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let name = call.vm.resolve_string(name_ref)?;
                let (part, frame_idx, use_alpha, decoded, stack_bytes) = if cmd == 0x451 {
                    let alpha = call.argument(12, "alpha flag")? != 0;
                    let dec = image::load(source, &name)?
                        .ok_or_else(|| error(call.pc, op, "failed to load CVNS image"))?;
                    (Vec::new(), None, alpha, dec, 0x400c)
                } else if cmd == 0x452 {
                    let frame = call.argument(12, "frame")? as usize;
                    let alpha = call.argument(16, "alpha flag")? != 0;
                    let dec = image::load_frame(source, &name, frame)?
                        .ok_or_else(|| error(call.pc, op, "failed to load CVNS image frame"))?;
                    (Vec::new(), Some(frame as u32), alpha, dec, 0x4010)
                } else {
                    let part_ref = call.argument(12, "part name")?;
                    let alpha = call.argument(16, "alpha flag")? != 0;
                    let part = call.vm.resolve_string(part_ref)?;
                    let dec = image::load_part(source, &name, &part)?
                        .ok_or_else(|| error(call.pc, op, "failed to load CVNS image part"))?;
                    (part, None, alpha, dec, 0x4010)
                };
                call.complete(stack_bytes)?;
                self.record_cg(&name);
                let mut layer =
                    CvnsImageLayer::from_image(name, part, decoded.pixels, 0, 0, use_alpha);
                layer.frame = frame_idx;
                self.images[idx] = Some(CvnsImageBuffer::with_base(layer));
            }
            0x453 | 0x464 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let dx = call.argument(12, "dx")? as i32;
                let dy = call.argument(16, "dy")? as i32;
                let name_ref = call.argument(20, "name")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                if layer as usize >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS image layer outside 0..15").into());
                }
                let name = call.vm.resolve_string(name_ref)?;
                let (part, use_alpha, decoded, stack_bytes) = if cmd == 0x453 {
                    let alpha = call.argument(24, "alpha flag")? != 0;
                    let dec = image::load(source, &name)?
                        .ok_or_else(|| error(call.pc, op, "failed to load CVNS layer image"))?;
                    (Vec::new(), alpha, dec, 0x4018)
                } else {
                    let part_ref = call.argument(24, "part name")?;
                    let alpha = call.argument(28, "alpha flag")? != 0;
                    let part = call.vm.resolve_string(part_ref)?;
                    let dec = image::load_part(source, &name, &part)?
                        .ok_or_else(|| error(call.pc, op, "failed to load CVNS layer part"))?;
                    (part, alpha, dec, 0x401c)
                };
                if self.images[idx].is_none() {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                }
                call.complete(stack_bytes)?;
                self.record_cg(&name);
                let buf = self.images[idx].as_mut().expect("checked");
                buf.layers[layer as usize] = Some(CvnsImageLayer::from_image(
                    name,
                    part,
                    decoded.pixels,
                    dx,
                    dy,
                    use_alpha,
                ));
            }
            0x455 => {
                let slot = call.argument(4, "image slot")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                call.complete(0x4004)?;
                self.images[idx] = None;
            }
            0x456 => {
                call.complete(0x4000)?;
                for slot in &mut self.images {
                    *slot = None;
                }
            }
            0x457 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                if layer == 0 || layer as usize >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS clear layer outside 1..15").into());
                }
                let Some(buf) = self.images[idx].as_mut() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                call.complete(0x4008)?;
                buf.layers[layer as usize] = None;
                if !buf.layers[1..].iter().any(Option::is_some) {
                    buf.composited = None;
                }
            }
            0x458 => {
                let slot = call.argument(4, "image slot")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let Some(buf) = self.images[idx].as_mut() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                buf.rebuild_composite()?;
                call.complete(0x4004)?;
            }
            0x45a => {
                let slot = call.argument(4, "image slot")?;
                let _ = Self::check_image_slot(call.pc, op, slot)?;
                let screen = crate::scene_render::cvns::render(
                    self,
                    backdrop.0,
                    backdrop.1,
                    backdrop.2,
                    backdrop_size[0],
                    backdrop_size[1],
                )?;
                self.install_captured_image(slot, screen)?;
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 1)
                    .expect("slot 0 exists");
            }
            0x45c => {
                let dst = Self::check_image_slot(call.pc, op, call.argument(4, "dst slot")?)?;
                let src = Self::check_image_slot(call.pc, op, call.argument(8, "src slot")?)?;
                if self.images[src].is_none() {
                    return Err(error(call.pc, op, "CVNS source image slot is empty").into());
                }
                call.complete(0x4008)?;
                self.images[dst] = self.images[src].take();
            }
            0x45d => {
                let dst = Self::check_image_slot(call.pc, op, call.argument(4, "dst slot")?)?;
                let src = Self::check_image_slot(call.pc, op, call.argument(8, "src slot")?)?;
                let cloned = self.images[src]
                    .clone()
                    .ok_or_else(|| error(call.pc, op, "CVNS source image slot is empty"))?;
                call.complete(0x4008)?;
                self.images[dst] = Some(cloned);
            }
            0x45e => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let color = call.argument(12, "color")?;
                self.apply_layer_op(call.pc, op, slot, layer, |l| apply_monochrome(l, color))?;
                call.complete(0x400c)?;
            }
            0x45f => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let mode = call.argument(12, "filter mode")?;
                let arg = call.argument(16, "filter arg")? as i32;
                if layer as usize >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS filter layer outside 0..15").into());
                }
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let Some(buf) = self.images[idx].as_mut() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                if let Some(l) = buf.layers[layer as usize].as_mut() {
                    apply_filter(l, mode, arg)?;
                }
                call.complete(0x4010)?;
            }
            0x461 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let src_c = call.argument(12, "src color")?;
                let dst_c = call.argument(16, "dst color")?;
                self.apply_layer_op(call.pc, op, slot, layer, |l| {
                    apply_hsl_tint(l, src_c, dst_c)
                })?;
                call.complete(0x4010)?;
            }
            0x463 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let dx = call.argument(12, "dx")? as i32;
                let dy = call.argument(16, "dy")? as i32;
                if layer as usize >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS layer outside 0..15").into());
                }
                self.apply_layer_op(call.pc, op, slot, layer, |l| {
                    l.dx = dx;
                    l.dy = dy;
                })?;
                call.complete(0x4010)?;
            }
            0x466 => {
                let slot = call.argument(4, "image slot")?;
                let color = call.argument(8, "fill color")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let Some(buf) = self.images[idx].as_mut() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                let Some(l0) = buf.layers[0].as_mut() else {
                    return Err(error(call.pc, op, "CVNS base layer is empty").into());
                };
                l0.pixels = empty_rgba(l0.width(), l0.height(), color)?;
                l0.fill_color = Some(color);
                l0.captured = false;
                l0.filtered = None;
                l0.filter_mode = 0;
                l0.monochrome = 0x00ff_ffff;
                l0.hsl_from = -1;
                l0.hsl_to = -1;
                l0.rgb_mul = -1;
                call.complete(0x4008)?;
            }
            0x467 => {
                let dst = Self::check_image_slot(call.pc, op, call.argument(4, "dst slot")?)?;
                let dst_layer = call.argument(8, "dst layer")? as usize;
                let src = Self::check_image_slot(call.pc, op, call.argument(12, "src slot")?)?;
                let src_layer = call.argument(16, "src layer")? as usize;
                let dx = call.argument(20, "dx")? as i32;
                let dy = call.argument(24, "dy")? as i32;
                if dst_layer >= CVNS_LAYERS || src_layer >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS copy layer outside 0..15").into());
                }
                let cloned = self.images[src]
                    .as_ref()
                    .ok_or_else(|| error(call.pc, op, "CVNS source image slot is empty"))?
                    .layers[src_layer]
                    .clone();
                let Some(dst_buf) = self.images[dst].as_mut() else {
                    return Err(error(call.pc, op, "CVNS destination image slot is empty").into());
                };
                if let Some(mut cloned) = cloned {
                    cloned.dx = dx;
                    cloned.dy = dy;
                    dst_buf.layers[dst_layer] = Some(cloned);
                }
                call.complete(0x4018)?;
            }
            0x618 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let mul = call.argument(12, "multiplier color")?;
                self.apply_layer_op(call.pc, op, slot, layer, |l| apply_rgb_mul(l, mul))?;
                call.complete(0x400c)?;
            }
            0x61e => {
                let slot = Self::check_image_slot(call.pc, op, call.argument(4, "image slot")?)?;
                let name_ref = call.argument(8, "mask name")?;
                if self.images[slot].is_none() {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                }
                let name = call.vm.resolve_string(name_ref)?;
                let dec = image::load(source, &name)?
                    .ok_or_else(|| error(call.pc, op, "failed to load CVNS image mask"))?;
                let mask = CvnsImageLayer::make_mask(name, &dec.pixels);
                self.images[slot].as_mut().expect("checked").mask = Some(mask);
                call.complete(0x4008)?;
            }
            0x61f => {
                let slot = Self::check_image_slot(call.pc, op, call.argument(4, "image slot")?)?;
                let Some(buf) = self.images[slot].as_mut() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                buf.mask = None;
                call.complete(0x4004)?;
            }
            0x588 => {
                let mask_ref = call.argument(4, "mask name")?;
                let sprite_slot = call.argument(8, "sprite slot")?;
                let duration_ms = call.argument(12, "duration ms")?;
                let ramp_ms = call.argument(16, "ramp ms")?;
                let reveal = call.argument(20, "reveal")? != 0;
                let mask_name = call.vm.resolve_string(mask_ref)?;
                let loaded = image::load(source, &mask_name)?;
                call.complete(0x4014)?;
                self.stop_mask_transition();
                let ok = if let Some(dec) = loaded {
                    self.mask_transition = Some(CvnsMaskTransition::new(
                        sprite_slot as u16,
                        reveal,
                        duration_ms,
                        ramp_ms,
                        &dec.pixels,
                    ));
                    true
                } else {
                    false
                };
                if let Some(sprite) = self
                    .sprites
                    .get_mut(sprite_slot as usize)
                    .and_then(Option::as_mut)
                {
                    sprite.mask_mode = -200;
                }
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(ok))
                    .expect("slot 0 exists");
            }
            0x589 => {
                let mask_ref = call.argument(4, "mask name")?;
                let duration_ms = call.argument(8, "duration ms")?;
                let ramp_ms = call.argument(12, "ramp ms")?;
                let mask_name = call.vm.resolve_string(mask_ref)?;
                let loaded = image::load(source, &mask_name)?;
                let screen = crate::scene_render::cvns::render(
                    self,
                    backdrop.0,
                    backdrop.1,
                    backdrop.2,
                    backdrop_size[0],
                    backdrop_size[1],
                )?;
                let ok = self.install_mask_capture(
                    screen,
                    loaded.as_ref().map(|d| &d.pixels),
                    duration_ms,
                    ramp_ms,
                )?;
                call.complete(0x400c)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(ok))
                    .expect("slot 0 exists");
            }
            _ => return Err(call.error("unregistered CVNS image command").into()),
        }
        Ok(Flow::Done)
    }

    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        fade_color: &mut u32,
        fade_level: &mut u16,
    ) -> Result<Flow, Box<dyn Error + Send + Sync>> {
        let cmd = call.command;
        let op = cmd + 0x1c00;
        match cmd {
            0x459 => {
                let slot = call.argument(4, "image slot")? as usize;
                let dims = self
                    .images
                    .get(slot)
                    .and_then(Option::as_ref)
                    .and_then(|b| b.base())
                    .map(|l| (l.width(), l.height()));
                call.complete(0x4004)?;
                let state = call.vm.state_mut();
                match dims {
                    Some((w, h)) => {
                        state.set_system_integer(0, 1).expect("slot 0");
                        state.set_system_integer(1, w).expect("slot 1");
                        state.set_system_integer(2, h).expect("slot 2");
                    }
                    None => state.set_system_integer(0, 0).expect("slot 0"),
                }
            }
            0x45b => {
                let slot = call.argument(4, "image slot")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let present = self.images[idx].is_some();
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(present))
                    .expect("slot 0");
            }
            0x462 => {
                let slot = call.argument(4, "image slot")?;
                let layer = call.argument(8, "layer")?;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                if layer as usize >= CVNS_LAYERS {
                    return Err(error(call.pc, op, "CVNS image layer outside 0..15").into());
                }
                let Some(buf) = self.images[idx].as_ref() else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                let dims = buf.layers[layer as usize]
                    .as_ref()
                    .map(|l| (l.width(), l.height()));
                call.complete(0x4008)?;
                let state = call.vm.state_mut();
                match dims {
                    Some((w, h)) => {
                        state.set_system_integer(0, 1).expect("slot 0");
                        state.set_system_integer(1, w).expect("slot 1");
                        state.set_system_integer(2, h).expect("slot 2");
                    }
                    None => state.set_system_integer(0, 0).expect("slot 0"),
                }
            }
            0x61c => {
                let slot = call.argument(4, "image slot")?;
                let x = call.argument(8, "x")? as i32;
                let y = call.argument(12, "y")? as i32;
                let idx = Self::check_image_slot(call.pc, op, slot)?;
                let Some(l0) = self.images[idx].as_ref().and_then(|b| b.base()) else {
                    return Err(error(call.pc, op, "CVNS image slot is empty").into());
                };
                let rgb = if x >= 0 && y >= 0 && (x as u32) < l0.width() && (y as u32) < l0.height()
                {
                    let i = (y as usize * l0.width() as usize + x as usize) * 4;
                    let px = l0.pixels.pixels();
                    pack_rgb(px[i], px[i + 1], px[i + 2])
                } else {
                    0
                };
                call.complete(0x400c)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, rgb)
                    .expect("slot 0");
            }
            0x470 | 0x471 => {
                let slot = call.argument(4, "sprite slot")?;
                let idx = Self::check_sprite_slot(call.pc, op, slot)?;
                call.complete(0x4004)?;
                let kind = if cmd == 0x471 {
                    CvnsSpriteKind::Video
                } else {
                    CvnsSpriteKind::Image
                };
                self.sprites[idx] = Some(CvnsSprite {
                    kind,
                    ..CvnsSprite::default()
                });
            }
            0x472 => {
                let slot = call.argument(4, "sprite slot")?;
                let x = call.argument(8, "x")? as i32;
                let y = call.argument(12, "y")? as i32;
                let w = call.argument(16, "w")? as i32;
                let h = call.argument(20, "h")? as i32;
                let color = call.argument(24, "color")?;
                let idx = Self::check_sprite_slot(call.pc, op, slot)?;
                call.complete(0x4018)?;
                self.sprites[idx] = Some(CvnsSprite {
                    kind: CvnsSpriteKind::SolidRect { color },
                    src_rect: [0, 0, w, h],
                    x,
                    y,
                    w,
                    h,
                    dirty: true,
                    ..CvnsSprite::default()
                });
            }
            0x474 => {
                let slot = call.argument(4, "sprite slot")?;
                let idx = Self::check_sprite_slot(call.pc, op, slot)?;
                call.complete(0x4004)?;
                self.sprites[idx] = None;
            }
            0x475 => {
                call.complete(0x4000)?;
                self.sprites.fill(None);
            }
            0x476 | 0x477 => {
                let slot = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let img = call.argument(8, "source slot")?;
                let sx = call.argument(12, "sx")? as i32;
                let sy = call.argument(16, "sy")? as i32;
                let sw = call.argument(20, "sw")? as i32;
                let sh = call.argument(24, "sh")? as i32;
                let is_video = matches!(
                    self.sprites[slot].as_ref().map(|s| s.kind),
                    Some(CvnsSpriteKind::Video)
                );
                let dims = if cmd == 0x476 && is_video {
                    return Err(error(call.pc, op, "CVNS video sprites are not implemented").into());
                } else {
                    self.images
                        .get(img as usize)
                        .and_then(Option::as_ref)
                        .and_then(|b| b.base())
                        .map(|l| (l.width() as i32, l.height() as i32))
                        .ok_or_else(|| error(call.pc, op, "CVNS source image slot is empty"))?
                };
                let sprite = self.sprites[slot]
                    .as_mut()
                    .ok_or_else(|| error(call.pc, op, "CVNS sprite slot is empty"))?;
                call.complete(0x4018)?;
                sprite.set_src_rect(img as i16, sx, sy, sw, sh, dims, cmd == 0x476, cmd == 0x477);
            }
            0x478 => {
                let slot = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let img = Self::check_image_slot(call.pc, op, call.argument(8, "image slot")?)?;
                if self.images[img].is_none() {
                    return Err(error(call.pc, op, "CVNS source image slot is empty").into());
                }
                let sprite = self.sprites[slot]
                    .as_mut()
                    .ok_or_else(|| error(call.pc, op, "CVNS sprite slot is empty"))?;
                call.complete(0x4008)?;
                sprite.image_slot = img as i16;
                sprite.dirty = true;
            }
            0x479 | 0x47a => {
                let slot = call.argument(4, "sprite slot")?;
                let x = call.argument(8, "x")? as i32;
                let y = call.argument(12, "y")? as i32;
                let stack_bytes = if cmd == 0x479 {
                    let _ = call.argument(16, "w")?;
                    let _ = call.argument(20, "h")?;
                    0x4014
                } else {
                    0x400c
                };
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(stack_bytes)?;
                sprite.x = x;
                sprite.y = y;
                if cmd == 0x479 {
                    sprite.w = sprite.src_rect[2] - sprite.src_rect[0];
                    sprite.h = sprite.src_rect[3] - sprite.src_rect[1];
                }
                sprite.dirty = true;
            }
            0x47b => {
                let slot = call.argument(4, "sprite slot")?;
                let z = call.argument(8, "z priority")? as i32;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                if (0..CVNS_Z_BUCKETS).contains(&z) {
                    sprite.z = z;
                }
                sprite.dirty = true;
            }
            0x47c => {
                let slot = call.argument(4, "sprite slot")?;
                let alpha = call.argument(8, "alpha")?;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.alpha = alpha.min(256) as u16;
                sprite.dirty = true;
            }
            0x47d => {
                let slot = call.argument(4, "sprite slot")?;
                let mode = call.argument(8, "blend mode")? as i16;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.blend_mode = mode;
                sprite.dirty = true;
            }
            0x47e => {
                let slot = call.argument(4, "sprite slot")?;
                let color = call.argument(8, "tint color")?;
                let strength = call.argument(12, "tint strength")? as i16;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x400c)?;
                sprite.color = color & 0x00ff_ffff;
                sprite.color_strength = strength;
                sprite.dirty = true;
            }
            0x47f => {
                let slot = call.argument(4, "sprite slot")?;
                let ox = call.argument(8, "origin x")? as i32;
                let oy = call.argument(12, "origin y")? as i32;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x400c)?;
                sprite.origin_x = ox;
                sprite.origin_y = oy;
                sprite.dirty = true;
            }
            0x480 => {
                let slot = call.argument(4, "sprite slot")?;
                let deg = call.argument(8, "angle")? as i32;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.rotation = deg.wrapping_abs() % 360;
                sprite.dirty = true;
            }
            0x481 => {
                let slot = call.argument(4, "sprite slot")?;
                let sx = call.argument(8, "scale x")? as i32;
                let sy = call.argument(12, "scale y")? as i32;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x400c)?;
                sprite.scale_x = sx;
                sprite.scale_y = sy;
                sprite.dirty = true;
            }
            0x482 => {
                let slot = call.argument(4, "sprite slot")?;
                let visible = call.argument(8, "visible")? != 0;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.visible = visible;
                sprite.dirty = true;
            }
            0x483 => {
                let slot = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let present = self.sprites[slot].is_some();
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(present))
                    .expect("slot 0");
            }
            0x484..=0x486 => {
                let slot = call.argument(4, "sprite slot")?;
                let idx = Self::check_sprite_slot(call.pc, op, slot)?;
                let s = self.sprites[idx]
                    .as_ref()
                    .ok_or_else(|| error(call.pc, op, "CVNS sprite slot is empty"))?;
                let (a, b, c) = (s.x as u32, s.y as u32, s.z as u32);
                let (w, h) = (s.w as u32, s.h as u32);
                call.complete(0x4004)?;
                let state = call.vm.state_mut();
                match cmd {
                    0x484 => {
                        state.set_system_integer(1, a).expect("slot 1");
                        state.set_system_integer(2, b).expect("slot 2");
                    }
                    0x485 => {
                        state.set_system_integer(1, w).expect("slot 1");
                        state.set_system_integer(2, h).expect("slot 2");
                    }
                    _ => state.set_system_integer(0, c).expect("slot 0"),
                }
            }
            0x488 => {
                let dst = Self::check_sprite_slot(call.pc, op, call.argument(4, "dst slot")?)?;
                let src = Self::check_sprite_slot(call.pc, op, call.argument(8, "src slot")?)?;
                if self.sprites[src].is_none() {
                    return Err(error(call.pc, op, "CVNS source sprite slot is empty").into());
                }
                call.complete(0x4008)?;
                self.sprites[dst] = self.sprites[src].take();
            }
            0x48b => {
                let slot = call.argument(4, "sprite slot")?;
                let flags = call.argument(8, "flags")?;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.flags = flags;
                sprite.dirty = true;
            }
            0x490 | 0x491 => {
                let slot = call.argument(4, "sprite slot")?;
                let frame = call.argument(8, "frame")? as usize;
                let next_frame = call.argument(12, "next frame")? as i16;
                let duration_ms = call.argument(16, "duration ms")? as i16;
                let image_slot = call.argument(20, "image slot")? as i16;
                let sx = call.argument(24, "sx")? as i16;
                let sy = call.argument(28, "sy")? as i16;
                let sw = call.argument(32, "sw")? as i16;
                let sh = call.argument(36, "sh")? as i16;
                let ox = call.argument(40, "ox")? as i16;
                let oy = call.argument(44, "oy")? as i16;
                let (playlist, stack_bytes) = if cmd == 0x490 {
                    (call.argument(48, "playlist slot")? as i32, 0x4030)
                } else {
                    (-1, 0x402c)
                };
                if frame >= CVNS_ANIM_FRAMES {
                    return Err(error(call.pc, op, "CVNS animation frame outside 0..63").into());
                }
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(stack_bytes)?;
                sprite.set_anim_frame(
                    frame,
                    next_frame,
                    duration_ms,
                    image_slot,
                    sx,
                    sy,
                    sw,
                    sh,
                    ox,
                    oy,
                    playlist,
                );
            }
            0x492 => {
                let slot = call.argument(4, "sprite slot")?;
                let track = call.argument(8, "playlist track")? as i16;
                if (track as usize) >= CVNS_PLAYLISTS {
                    return Err(error(call.pc, op, "CVNS animation track outside 0..5").into());
                }
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4008)?;
                sprite.start_anim(track);
            }
            0x493 => {
                let slot = call.argument(4, "sprite slot")?;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4004)?;
                sprite.stop_anim();
            }
            0x494 => {
                let slot = call.argument(4, "sprite slot")?;
                let idx = Self::check_sprite_slot(call.pc, op, slot)?;
                let sprite = self.sprites[idx]
                    .as_ref()
                    .ok_or_else(|| error(call.pc, op, "CVNS sprite slot is empty"))?;
                let track = sprite.anim.map_or(-1, |a| i32::from(a.track));
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, track as u32)
                    .expect("slot 0");
            }
            0x498 | 0x49c => {
                let slot = call.argument(4, "sprite slot")?;
                let dst_x = call.argument(8, "dst x")? as i32;
                let dst_y = call.argument(12, "dst y")? as i32;
                let (ctrl_x, ctrl_y, sh) = if cmd == 0x49c {
                    (
                        call.argument(16, "ctrl x")? as i32,
                        call.argument(20, "ctrl y")? as i32,
                        8usize,
                    )
                } else {
                    (0, 0, 0usize)
                };
                let dst_alpha = call.argument(16 + sh, "dst alpha")? as i32;
                let flags = call.argument(20 + sh, "flags")?;
                let show = call.argument(24 + sh, "show")? != 0;
                let duration_ms = call.argument(28 + sh, "duration ms")?;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(if cmd == 0x49c { 0x4024 } else { 0x401c })?;
                sprite.move_tween = Some(CvnsMoveTween {
                    active: true,
                    start_x: sprite.x,
                    start_y: sprite.y,
                    start_alpha: i32::from(sprite.alpha),
                    dst_x,
                    dst_y,
                    ctrl_x,
                    ctrl_y,
                    dst_alpha,
                    flags,
                    duration_ms,
                    elapsed_ms: 0,
                    delay_ms: 0,
                });
                if show {
                    sprite.visible = true;
                    sprite.dirty = true;
                }
            }
            0x499 => {
                let slot = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let remove = self.sprites[slot]
                    .as_mut()
                    .is_some_and(CvnsSprite::finish_move);
                call.complete(0x4004)?;
                if remove {
                    self.sprites[slot] = None;
                }
            }
            0x49b => {
                let slot = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let rem = self.sprites[slot].as_ref().map_or(-1, |s| {
                    s.move_tween
                        .map(|t| t.duration_ms.saturating_sub(t.elapsed_ms) as i32)
                        .unwrap_or(-1)
                });
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, rem as u32)
                    .expect("slot 0");
            }
            0x49d => {
                let _ = Self::check_sprite_slot(call.pc, op, call.argument(4, "sprite slot")?)?;
                let _ = call.argument(8, "reserved")?;
                call.complete(0x4008)?;
            }
            0x58a => {
                call.complete(0x4000)?;
                let rem = self
                    .mask_transition
                    .as_ref()
                    .map_or(u32::MAX, CvnsMaskTransition::remaining);
                call.vm
                    .state_mut()
                    .set_system_integer(0, rem)
                    .expect("slot 0");
            }
            0x58b => {
                call.complete(0x4000)?;
                self.stop_mask_transition();
            }
            0x58d => {
                let slot = call.argument(4, "sprite slot")?;
                let mode = call.argument(8, "curtain mode")?;
                let interval = call.argument(12, "interval ms")?;
                let band = call.argument(16, "band size")? as i32;
                let delay = call.argument(20, "delay step")?;
                let speed = call.argument(24, "speed")? as i32;
                let sprite = self.require_sprite_mut(call.pc, op, slot)?;
                call.complete(0x4018)?;
                sprite.finish_curtain();
                sprite.curtain = Some(CvnsCurtain::new(sprite, mode, interval, band, delay, speed));
            }
            0x58f => {
                let to_normal = call.argument(4, "to normal")? != 0;
                let color = call.argument(8, "color")?;
                let duration_ms = call.argument(12, "duration ms")?;
                call.complete(0x400c)?;
                self.fade = Some(CvnsFade {
                    to_normal,
                    color,
                    duration_ms,
                    elapsed_ms: 0,
                });
                call.vm
                    .state_mut()
                    .set_system_integer(0, 1)
                    .expect("slot 0");
            }
            0x590 => {
                call.complete(0x4000)?;
                let rem = self.fade.as_ref().map_or(u32::MAX, CvnsFade::remaining);
                call.vm
                    .state_mut()
                    .set_system_integer(0, rem)
                    .expect("slot 0");
            }
            0x591 => {
                call.complete(0x4000)?;
                if let Some(fade) = self.fade.take() {
                    *fade_color = fade.color;
                    *fade_level = fade.finish_alpha();
                }
            }
            0x593 => {
                let kind = call.argument(4, "quake kind")?;
                let amp = call.argument(8, "amplitude")? as i32;
                let interval = call.argument(12, "interval ms")?;
                let count = call.argument(16, "count")? as i32;
                call.complete(0x4010)?;
                self.quake = Some(CvnsQuake::new(kind, amp, interval, count));
            }
            0x595 => {
                let paused = call.argument(4, "paused")? != 0;
                call.complete(0x4004)?;
                if let Some(q) = &mut self.quake {
                    q.paused = paused;
                }
            }
            0x596 => {
                call.complete(0x4000)?;
                let rem = self.quake.as_ref().map_or(-1, |q| {
                    if q.count == -1 {
                        -1
                    } else {
                        q.count - q.step_count
                    }
                });
                call.vm
                    .state_mut()
                    .set_system_integer(0, rem as u32)
                    .expect("slot 0");
            }
            _ => return Err(call.error("unregistered CVNS scene command").into()),
        }
        Ok(Flow::Done)
    }

    pub(crate) fn wait(
        &mut self,
        call: &mut Call<'_>,
        input: &mut ActionInput,
        gate: SkipGate,
        fade_color: &mut u32,
        fade_level: &mut u16,
    ) -> Result<Flow, Box<dyn Error + Send + Sync>> {
        let cmd = call.command;
        match cmd {
            0x58c => {
                let allow = call.argument(4, "interrupt mask")?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 0)
                    .expect("slot 0");
                if self.mask_transition.is_none() {
                    call.complete(0x4004)?;
                    return Ok(Flow::Done);
                }
                let hit = cvns_interrupted(input, gate, allow, true);
                if hit {
                    self.stop_mask_transition();
                    call.vm
                        .state_mut()
                        .set_system_integer(0, 1)
                        .expect("slot 0");
                    return call.finish(0xc004).map_err(Into::into);
                }
                call.wait(4).map_err(Into::into)
            }
            0x58e => {
                let slot = Self::check_sprite_slot(
                    call.pc,
                    cmd + 0x1c00,
                    call.argument(4, "sprite slot")?,
                )?;
                let allow = call.argument(8, "interrupt mask")?;
                let consume = call.argument(12, "consume flag")? != 0;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 0)
                    .expect("slot 0");
                let active = self.sprites[slot]
                    .as_ref()
                    .is_some_and(|s| s.curtain.is_some());
                if !active {
                    call.complete(0x400c)?;
                    return Ok(Flow::Done);
                }
                if cvns_interrupted(input, gate, allow, consume) {
                    if let Some(s) = self.sprites[slot].as_mut() {
                        s.finish_curtain();
                    }
                    call.vm
                        .state_mut()
                        .set_system_integer(0, 1)
                        .expect("slot 0");
                    return call.finish(0xc00c).map_err(Into::into);
                }
                call.wait(12).map_err(Into::into)
            }
            0x592 => {
                let allow = call.argument(4, "interrupt mask")?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 0)
                    .expect("slot 0");
                if self.fade.is_none() {
                    call.complete(0x4004)?;
                    return Ok(Flow::Done);
                }
                if cvns_interrupted(input, gate, allow, true) {
                    if let Some(fade) = self.fade.take() {
                        *fade_color = fade.color;
                        *fade_level = fade.finish_alpha();
                    }
                    call.vm
                        .state_mut()
                        .set_system_integer(0, 1)
                        .expect("slot 0");
                    return call.finish(0xc004).map_err(Into::into);
                }
                call.wait(4).map_err(Into::into)
            }
            0x594 => {
                let wait = call.argument(4, "wait flag")? != 0;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 0)
                    .expect("slot 0");
                if self.quake.is_none() {
                    call.vm
                        .state_mut()
                        .set_system_integer(0, 1)
                        .expect("slot 0");
                    call.complete(0x4004)?;
                    return Ok(Flow::Done);
                }
                if wait {
                    return call.wait(4).map_err(Into::into);
                }
                self.quake = None;
                call.vm
                    .state_mut()
                    .set_system_integer(0, 1)
                    .expect("slot 0");
                call.finish(0xc004).map_err(Into::into)
            }
            _ => Err(call.error("unregistered CVNS wait command").into()),
        }
    }
}

fn cvns_interrupted(
    input: &mut ActionInput,
    gate: SkipGate,
    allow_mask: u32,
    consume: bool,
) -> bool {
    (allow_mask != 0 && !gate.effects)
        || (allow_mask != 0 && (gate.skip || input.cvns_skip_held()))
        || input.cvns_wait_interrupt(allow_mask, consume)
}

fn apply_monochrome(layer: &mut CvnsImageLayer, color: u32) {
    let [tr, tg, tb] = unpack_rgb(color);
    let mut pixels = layer.pixels.pixels().to_vec();
    for px in pixels.as_chunks_mut::<4>().0 {
        let avg = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3;
        px[0] = ((avg * u32::from(tr)) / 255) as u8;
        px[1] = ((avg * u32::from(tg)) / 255) as u8;
        px[2] = ((avg * u32::from(tb)) / 255) as u8;
    }
    if let Ok(img) = RgbaImage::new(layer.width(), layer.height(), pixels) {
        layer.pixels = img;
    }
    layer.monochrome = color;
}

fn apply_rgb_mul(layer: &mut CvnsImageLayer, mul: u32) {
    let [mr, mg, mb] = unpack_rgb(mul);
    let mut pixels = layer.pixels.pixels().to_vec();
    for px in pixels.as_chunks_mut::<4>().0 {
        px[0] = ((u32::from(mr) * u32::from(px[0])) >> 7).min(255) as u8;
        px[1] = ((u32::from(mg) * u32::from(px[1])) >> 7).min(255) as u8;
        px[2] = ((u32::from(mb) * u32::from(px[2])) >> 7).min(255) as u8;
    }
    if let Ok(img) = RgbaImage::new(layer.width(), layer.height(), pixels) {
        layer.pixels = img;
    }
    layer.rgb_mul = mul as i32;
}

fn rgb_to_hls_bgr(r: u8, g: u8, b: u8) -> (i16, i16, i16) {
    let max = r.max(g).max(b) as i32;
    let min = r.min(g).min(b) as i32;
    let l = ((240 * (min + max) + 255) / 510) as i16;
    if max == min {
        return (160, l, 0);
    }
    let span = max - min;
    let s = if l > 120 {
        (((510 - max - min) / 2 + 240 * span) / (510 - max - min)) as i16
    } else {
        (((min + max) / 2 + 240 * span) / (min + max)) as i16
    };
    let db = ((span / 2 + 40 * (max - b as i32)) / span) as i16;
    let dg = ((span / 2 + 40 * (max - g as i32)) / span) as i16;
    let dr = ((span / 2 + 40 * (max - r as i32)) / span) as i16;
    let mut h = if b as i32 == max {
        dr - dg
    } else if g as i32 == max {
        db + 80 - dr
    } else {
        dg + 160 - db
    };
    if h < 0 {
        h += 240;
    }
    if h > 240 {
        h -= 240;
    }
    (h, l, s)
}

fn hue_to_rgb(n1: i32, n2: i32, mut hue: i32) -> i32 {
    if hue < 0 {
        hue += 240;
    } else if hue > 240 {
        hue -= 240;
    }
    if hue < 40 {
        (hue * (n2 - n1) + 20) / 40 + n1
    } else if hue < 120 {
        n2
    } else if hue >= 160 {
        n1
    } else {
        ((160 - hue) * (n2 - n1) + 20) / 40 + n1
    }
}

fn hls_bgr_to_rgb(h: i16, l: i16, s: i16) -> [u8; 3] {
    if s == 0 {
        let v = (255 * i32::from(l) / 240).clamp(0, 255) as u8;
        return [v, v, v];
    }
    let l32 = i32::from(l);
    let s32 = i32::from(s);
    let m2 = if l32 > 120 {
        s32 + l32 - (s32 * l32 + 120) / 240
    } else {
        ((s32 + 240) * l32 + 120) / 240
    };
    let m1 = 2 * l32 - m2;
    let h32 = i32::from(h);
    let b = ((255 * hue_to_rgb(m1, m2, h32 + 80) + 120) / 240).clamp(0, 255) as u8;
    let g = ((255 * hue_to_rgb(m1, m2, h32) + 120) / 240).clamp(0, 255) as u8;
    let r = ((255 * hue_to_rgb(m1, m2, h32 - 80) + 120) / 240).clamp(0, 255) as u8;
    [r, g, b]
}

pub(crate) fn apply_hsl_tint(layer: &mut CvnsImageLayer, src_c: u32, dst_c: u32) {
    let [sr, sg, sb] = unpack_rgb(src_c);
    let [dr, dg, db] = unpack_rgb(dst_c);
    let (sh, sl, ss) = rgb_to_hls_bgr(sr, sg, sb);
    let (dh, _, _) = rgb_to_hls_bgr(dr, dg, db);
    let hue_delta = dh - sh;
    let mut pixels = layer.pixels.pixels().to_vec();
    for px in pixels.as_chunks_mut::<4>().0 {
        let (ph, pl, ps) = rgb_to_hls_bgr(px[0], px[1], px[2]);
        let nh = (ph - hue_delta + 240) % 240;
        let nl = (((i32::from(sl) - 128) as f64 * 0.2_f32 as f64) as i32
            + i32::from(sl) * i32::from(pl) / 240)
            .clamp(0, 240) as i16;
        let ns = (i32::from(ss) * i32::from(ps) / 240).clamp(0, 240) as i16;
        let out = hls_bgr_to_rgb(nh, nl, ns);
        px[0] = out[0];
        px[1] = out[1];
        px[2] = out[2];
    }
    if let Ok(img) = RgbaImage::new(layer.width(), layer.height(), pixels) {
        layer.pixels = img;
    }
    layer.hsl_from = src_c as i32;
    layer.hsl_to = dst_c as i32;
}

fn apply_filter(
    layer: &mut CvnsImageLayer,
    mode: u32,
    arg: i32,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    layer.filter_mode = mode;
    layer.filter_arg = arg;
    if mode == 0 {
        layer.filtered = None;
        return Ok(());
    }
    let w = layer.width() as usize;
    let h = layer.height() as usize;
    let src = layer.pixels.pixels();
    let mut dst = vec![0u8; src.len()];
    match mode {
        1 => {
            for (s, d) in src
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<4>().0)
            {
                d[0] = !s[0];
                d[1] = !s[1];
                d[2] = !s[2];
                d[3] = s[3];
            }
        }
        3 => {
            let block = (arg.max(1)) as usize;
            for by in (0..h).step_by(block) {
                let bh = (h - by).min(block);
                for bx in (0..w).step_by(block) {
                    let bw = (w - bx).min(block);
                    let pick = ((by + bh - 1) * w + (bx + bw - 1)) * 4;
                    let sample = &src[pick..pick + 4];
                    for y in by..by + bh {
                        for x in bx..bx + bw {
                            let d = (y * w + x) * 4;
                            dst[d..d + 4].copy_from_slice(sample);
                        }
                    }
                }
            }
        }
        4 => {
            let span = arg.max(0) as isize;
            for y in 0..h {
                for x in 0..w as isize {
                    let mut sum = [0u32; 4];
                    let mut neighbors = 0u32;
                    for nx in [x - span, x + span] {
                        if nx >= 0 && (nx as usize) < w {
                            let s = (y * w + nx as usize) * 4;
                            for c in 0..4 {
                                sum[c] += u32::from(src[s + c]);
                            }
                            neighbors += 1;
                        }
                    }
                    if neighbors >= 2 {
                        for c in &mut sum {
                            *c >>= 1;
                        }
                    }
                    let center = (y * w + x as usize) * 4;
                    for c in 0..4 {
                        dst[center + c] = ((sum[c] + u32::from(src[center + c])) >> 1) as u8;
                    }
                }
            }
        }
        9 => {
            let [tr, tg, tb] = unpack_rgb(arg as u32);
            for (s, d) in src
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<4>().0)
            {
                let avg = (u32::from(s[0]) + u32::from(s[1]) + u32::from(s[2])) / 3;
                d[0] = ((avg * u32::from(tr)) / 255) as u8;
                d[1] = ((avg * u32::from(tg)) / 255) as u8;
                d[2] = ((avg * u32::from(tb)) / 255) as u8;
                d[3] = s[3];
            }
        }
        10 => {
            let rad = (arg & !1).max(0);
            for y in 0..h {
                for x in 0..w as isize {
                    let mut sum = [0u32; 3];
                    let mut count = 0u32;
                    let mut k = -(rad as isize);
                    while k <= rad as isize {
                        let nx = x + k;
                        if nx >= 0 && (nx as usize) < w {
                            let s = (y * w + nx as usize) * 4;
                            for c in 0..3 {
                                sum[c] += u32::from(src[s + c]);
                            }
                            count += 1;
                        }
                        k += 2;
                    }
                    let d = (y * w + x as usize) * 4;
                    for c in 0..3 {
                        dst[d + c] = (sum[c] / count.max(1)) as u8;
                    }
                    dst[d + 3] = 255;
                }
            }
        }
        11..=14 => {
            let radius = (arg & !1).max(0);
            let dir = if matches!(mode, 11 | 12) { -1 } else { 1 };
            let scaled = matches!(mode, 12 | 14);
            layer.filter_arg = radius;
            filter_radial(src, &mut dst, w, h, radius, dir, scaled);
        }
        15 | 16 => {
            let radius = (arg & !1).max(0);
            layer.filter_arg = radius;
            filter_spin(src, &mut dst, w, h, radius, mode == 16);
        }
        _ => return Ok(()),
    }
    layer.filtered = Some(RgbaImage::new(w as u32, h as u32, dst)?);
    Ok(())
}

fn filter_radial(
    src: &[u8],
    dst: &mut [u8],
    w: usize,
    h: usize,
    radius: i32,
    sign: i32,
    scaled: bool,
) {
    let cx = (w >> 1) as i32;
    let cy = (h >> 1) as i32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let d_idx = (y as usize * w + x as usize) * 4;
            let dist = ((x - cx) as f64).hypot((y - cy) as f64) as i32;
            if dist <= 0 || cx == 0 {
                dst[d_idx..d_idx + 3].copy_from_slice(&src[d_idx..d_idx + 3]);
                dst[d_idx + 3] = 255;
                continue;
            }
            let steps = if scaled {
                radius * dist / cx.max(1) + 1
            } else {
                radius
            };
            let step_x = sign * ((cx - x) << 10) / dist;
            let step_y = sign * ((cy - y) << 10) / dist;
            let (mut px, mut py) = (x << 10, y << 10);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for _ in 0..=steps {
                let sx = px >> 10;
                let sy = py >> 10;
                if sx >= 0 && (sx as usize) < w && sy >= 0 && (sy as usize) < h {
                    let s = (sy as usize * w + sx as usize) * 4;
                    for c in 0..3 {
                        sum[c] += u32::from(src[s + c]);
                    }
                    count += 1;
                }
                px += step_x;
                py += step_y;
            }
            for c in 0..3 {
                dst[d_idx + c] = (sum[c] / count.max(1)) as u8;
            }
            dst[d_idx + 3] = 255;
        }
    }
}

fn filter_spin(src: &[u8], dst: &mut [u8], w: usize, h: usize, radius: i32, scaled: bool) {
    let cx = (w >> 1) as i32;
    let cy = (h >> 1) as i32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let d_idx = (y as usize * w + x as usize) * 4;
            let (deg, dist) = polar_offset(cx, cy, x, y);
            let deg = deg / 100;
            if dist <= 0 || cx == 0 {
                dst[d_idx..d_idx + 3].copy_from_slice(&src[d_idx..d_idx + 3]);
                dst[d_idx + 3] = 255;
                continue;
            }
            let steps = if scaled {
                radius * dist / cx.max(1) + 1
            } else {
                radius
            };
            let base = deg + 180 - steps / 2;
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for k in 0..=steps {
                let a = (k + base).rem_euclid(360);
                let (dx, dy) = sin_cos_scale(a * 100, dist);
                let sx = cx - dx;
                let sy = cy - dy;
                if sx >= 0 && (sx as usize) < w && sy >= 0 && (sy as usize) < h {
                    let s = (sy as usize * w + sx as usize) * 4;
                    for c in 0..3 {
                        sum[c] += u32::from(src[s + c]);
                    }
                    count += 1;
                }
            }
            for (c, s) in sum.iter().enumerate() {
                if let Some(q) = s.checked_div(count) {
                    dst[d_idx + c] = q as u8;
                }
            }
            dst[d_idx + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::Call;
    use crate::input::{ActionInput, CodeInput, KeyBindings};
    use crate::machine::{Machine, Step};
    use crate::ps3::Script;
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    enum TestArg {
        Int(i32),
        Str(&'static [u8]),
    }

    fn build_pb2(
        kind: u16,
        width: u16,
        height: u16,
        bpp: u16,
        count: u32,
        diff_offset: u32,
        body: &[u8],
    ) -> Vec<u8> {
        let mut raw = vec![0u8; 32];
        raw.extend_from_slice(body);
        if raw.len() < 33 {
            raw.resize(33, 0);
        }
        raw.extend_from_slice(&[0u8; 32]); // zero key/tail => header stays plain LE
        let total = raw.len() as u32;
        raw[..4].copy_from_slice(b"PB2A");
        raw[4..8].copy_from_slice(&total.to_le_bytes());
        raw[8..12].copy_from_slice(&count.to_le_bytes());
        raw[12..16].copy_from_slice(&diff_offset.to_le_bytes());
        raw[16..18].copy_from_slice(&kind.to_le_bytes());
        raw[18..20].copy_from_slice(&width.to_le_bytes());
        raw[20..22].copy_from_slice(&height.to_le_bytes());
        raw[22..24].copy_from_slice(&bpp.to_le_bytes());
        raw
    }

    fn lzss_literals(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        (vec![0u8; bytes.len().div_ceil(8)], bytes.to_vec())
    }

    fn xor_base_block(planes: [&[u8]; 4]) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        let mut stream = Vec::new();
        for (p, plane) in planes.iter().enumerate() {
            let (flags, data) = lzss_literals(plane);
            out[p * 8..p * 8 + 4].copy_from_slice(&(stream.len() as u32).to_le_bytes());
            stream.extend_from_slice(&flags);
            out[p * 8 + 4..p * 8 + 8].copy_from_slice(&(stream.len() as u32).to_le_bytes());
            stream.extend_from_slice(&data);
        }
        out.extend_from_slice(&stream);
        out
    }

    struct MemImages(HashMap<String, Vec<u8>>);

    impl ImageSource for MemImages {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
            let key = String::from_utf8_lossy(name).to_ascii_lowercase();
            Ok(self.0.get(&key).cloned())
        }
    }

    fn vm_pending(command: u16, args: &[TestArg]) -> (Machine, usize) {
        let mut text = Vec::new();
        let mut code = Vec::new();
        for arg in args.iter().rev() {
            match *arg {
                TestArg::Int(v) => {
                    code.extend_from_slice(&0x0200u16.to_le_bytes());
                    code.extend_from_slice(&0x0100u16.to_le_bytes());
                    code.extend_from_slice(&(v as u32).to_le_bytes());
                    code.extend_from_slice(&0x020fu16.to_le_bytes());
                    code.extend_from_slice(&0x0430u16.to_le_bytes());
                }
                TestArg::Str(s) => {
                    let off = text.len() as u32;
                    text.extend_from_slice(s);
                    text.push(0);
                    code.extend_from_slice(&0x0201u16.to_le_bytes());
                    code.extend_from_slice(&0x0120u16.to_le_bytes());
                    code.extend_from_slice(&off.to_le_bytes());
                    code.extend_from_slice(&0x020fu16.to_le_bytes());
                    code.extend_from_slice(&0x0430u16.to_le_bytes());
                }
            }
        }
        code.extend_from_slice(&(0x2000u16 | command).to_le_bytes());
        code.extend_from_slice(&0x0416u16.to_le_bytes());

        let mut payload = code;
        let code_size = payload.len() as u32;
        let text_size = text.len() as u32;
        payload.extend_from_slice(&text);

        let mut raw = Vec::new();
        raw.extend_from_slice(b"PS2A");
        raw.extend_from_slice(&48u32.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&code_size.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&text_size.to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes()); // entry_pc
        raw.extend_from_slice(&0u32.to_le_bytes()); // compressed_size
        raw.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // decoded_size
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&payload);

        let script = Box::leak(Box::new(Script::from_memory_image(&raw).unwrap()));
        let mut vm = Machine::new(script).unwrap();
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
        let pc = vm.pc();
        (vm, pc)
    }

    fn exec_img(
        cvns: &mut CvnsScene,
        images: &mut MemImages,
        command: u16,
        args: &[TestArg],
    ) -> Machine {
        let (mut vm, pc) = vm_pending(command, args);
        let mut call = Call::new(&mut vm, pc, command);
        cvns.handle_image(&mut call, images, [64, 64], (false, 0, 0x100))
            .unwrap();
        vm
    }

    fn exec_scene(
        cvns: &mut CvnsScene,
        fade_color: &mut u32,
        fade_level: &mut u16,
        command: u16,
        args: &[TestArg],
    ) -> Machine {
        let (mut vm, pc) = vm_pending(command, args);
        let mut call = Call::new(&mut vm, pc, command);
        cvns.handle(&mut call, fade_color, fade_level).unwrap();
        vm
    }

    fn exec_wait(
        cvns: &mut CvnsScene,
        command: u16,
        args: &[TestArg],
        input: &mut ActionInput,
        effects: bool,
    ) -> (Machine, Flow) {
        let (mut vm, pc) = vm_pending(command, args);
        let (mut fc, mut fl) = (0u32, 0x100u16);
        let flow = cvns
            .wait(
                &mut Call::new(&mut vm, pc, command),
                input,
                SkipGate {
                    effects,
                    skip: false,
                },
                &mut fc,
                &mut fl,
            )
            .unwrap();
        (vm, flow)
    }

    fn synthetic_pb2_type5() -> Vec<u8> {
        let mut body = xor_base_block([&[0x10; 64], &[0x03; 64], &[0x05; 64], &[0x0f; 64]]);
        let diff_off = 32 + body.len() as u32;
        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes());
        patch.extend(0u32.to_le_bytes());
        patch.push(0x00);
        patch.extend_from_slice(&[10u8, 20, 30].repeat(64));
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk_size = 16 + pflags.len() + pdata.len();
        body.extend((chunk_size as u32).to_le_bytes());
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);
        build_pb2(5, 8, 8, 32, 2, diff_off, &body)
    }

    fn synthetic_pb2_type6() -> Vec<u8> {
        let xor = xor_base_block([&[0x10; 18], &[0x03; 18], &[0x05; 18], &[0x0f; 18]]);
        let chunk0_size = 4 + 8 + xor.len();
        let mut body = Vec::new();
        body.extend((chunk0_size as u32).to_le_bytes());
        body.extend_from_slice(b"Base\0\0\0\0");
        body.extend_from_slice(&xor);

        let mut patch = Vec::new();
        patch.extend(1u32.to_le_bytes());
        patch.extend(0u32.to_le_bytes());
        patch.push(0x80); // keep tile 0, replace tile 1 (x=8)
        patch.extend_from_slice(&[100, 150, 200, 255, 100, 150, 200, 255]);
        let (pflags, pdata) = lzss_literals(&patch);
        let chunk1_size = 4 + 8 + 16 + pflags.len() + pdata.len();
        body.extend((chunk1_size as u32).to_le_bytes());
        body.extend_from_slice(b"Face_A\0\0");
        body.extend((pflags.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend((patch.len() as u32).to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend_from_slice(&pflags);
        body.extend_from_slice(&pdata);
        build_pb2(6, 9, 2, 32, 2, 0, &body)
    }

    #[test]
    fn pb2_type5_type6_and_layer_filters_work() {
        let mut map = HashMap::new();
        map.insert("anim".into(), synthetic_pb2_type5());
        map.insert("pose".into(), synthetic_pb2_type6());
        let mut images = MemImages(map);
        let mut cvns = CvnsScene::default();
        let (mut fc, mut fl) = (0u32, 0x100u16);

        // 0x2052: PB2A type-5 frame 1 (alpha disabled -> alpha coerced to 255).
        exec_img(
            &mut cvns,
            &mut images,
            0x452,
            &[
                TestArg::Int(127),
                TestArg::Str(b"anim"),
                TestArg::Int(1),
                TestArg::Int(0),
            ],
        );
        let vm = exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x61c,
            &[TestArg::Int(127), TestArg::Int(0), TestArg::Int(0)],
        );
        assert_eq!(vm.state().system_integer(0), Some(0x001e_140a)); // RGB (30, 20, 10)

        // 0x2065 + 0x2064 + 0x2058: PB2A type-6 Base + Face_A layer composite.
        exec_img(
            &mut cvns,
            &mut images,
            0x465,
            &[
                TestArg::Int(5),
                TestArg::Str(b"pose"),
                TestArg::Str(b"Base"),
                TestArg::Int(1),
            ],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x464,
            &[
                TestArg::Int(5),
                TestArg::Int(1),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Str(b"pose"),
                TestArg::Str(b"Face_A"),
                TestArg::Int(1),
            ],
        );
        let base = cvns.images[5].as_ref().unwrap().base().unwrap();
        assert_eq!(&base.pixels.pixels()[..4], &[10, 9, 25, 15]);
        exec_img(&mut cvns, &mut images, 0x458, &[TestArg::Int(5)]);
        let comp = cvns.images[5].as_ref().unwrap().display_pixels().unwrap();
        assert_eq!(&comp.pixels()[..4], &[10, 9, 25, 255]);
        assert_eq!(&comp.pixels()[8 * 4..8 * 4 + 4], &[200, 150, 100, 255]);

        // 0x205e monochrome & 0x205f invert on layer 0 + 0x2066 fill.
        exec_img(
            &mut cvns,
            &mut images,
            0x45e,
            &[TestArg::Int(5), TestArg::Int(0), TestArg::Int(0x00ff_ffff)],
        );
        let mono_px = &cvns.images[5]
            .as_ref()
            .unwrap()
            .base()
            .unwrap()
            .pixels
            .pixels()[..4];
        assert_eq!(mono_px[0], mono_px[1]);
        assert_eq!(mono_px[1], mono_px[2]);

        exec_img(
            &mut cvns,
            &mut images,
            0x466,
            &[TestArg::Int(5), TestArg::Int(0x0012_3456)],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x45f,
            &[
                TestArg::Int(5),
                TestArg::Int(0),
                TestArg::Int(1),
                TestArg::Int(0),
            ],
        );
        assert_eq!(
            &cvns.images[5]
                .as_ref()
                .unwrap()
                .base()
                .unwrap()
                .active_pixels()
                .pixels()[..4],
            &[0xed, 0xcb, 0xa9, 255]
        );
        // 0x205f mode=3 mosaic and mode=0 clear filter.
        exec_img(
            &mut cvns,
            &mut images,
            0x45f,
            &[
                TestArg::Int(5),
                TestArg::Int(0),
                TestArg::Int(3),
                TestArg::Int(2),
            ],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x45f,
            &[
                TestArg::Int(5),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Int(0),
            ],
        );
        assert_eq!(
            &cvns.images[5]
                .as_ref()
                .unwrap()
                .base()
                .unwrap()
                .active_pixels()
                .pixels()[..4],
            &[0x12, 0x34, 0x56, 255]
        );

        // Empty sub-layer inside a populated image slot is a clean no-op (`sub_43A4B0`, `sub_439ED0`, etc.).
        exec_img(
            &mut cvns,
            &mut images,
            0x45f,
            &[
                TestArg::Int(5),
                TestArg::Int(2),
                TestArg::Int(1),
                TestArg::Int(0),
            ],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x45e,
            &[
                TestArg::Int(5),
                TestArg::Int(255),
                TestArg::Int(0x00ff_ffff),
            ],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x463,
            &[
                TestArg::Int(5),
                TestArg::Int(2),
                TestArg::Int(7),
                TestArg::Int(9),
            ],
        );
        assert!(cvns.images[5].as_ref().unwrap().layers[2].is_none());
    }

    #[test]
    fn compositor_orders_slots_uses_alpha_flag_and_applies_fade_and_quake() {
        let mut images = MemImages(HashMap::new());
        let mut cvns = CvnsScene::default();
        let (mut fc, mut fl) = (0u32, 0x100u16);

        // Two solid 4x4 images: slot 1 red, slot 2 green.
        exec_img(
            &mut cvns,
            &mut images,
            0x450,
            &[TestArg::Int(1), TestArg::Int(4), TestArg::Int(4)],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x466,
            &[TestArg::Int(1), TestArg::Int(0x00ff_0000)],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x450,
            &[TestArg::Int(2), TestArg::Int(4), TestArg::Int(4)],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x466,
            &[TestArg::Int(2), TestArg::Int(0x0000_ff00)],
        );
        // Make slot 2 semi-transparent RGB (0, 200, 0, alpha=0) and test use_alpha=false vs true.
        {
            let l0 = cvns.images[2].as_mut().unwrap().layers[0].as_mut().unwrap();
            l0.use_alpha = false;
            l0.pixels = RgbaImage::new(4, 4, [0, 200, 0, 0].repeat(16)).unwrap();
        }

        // Sprite 10 and sprite 1535 at same z=5 -> slot 10 draws AFTER 1535 (reverse slot order).
        exec_scene(&mut cvns, &mut fc, &mut fl, 0x470, &[TestArg::Int(1535)]);
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x476,
            &[
                TestArg::Int(1535),
                TestArg::Int(1),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Int(4),
                TestArg::Int(4),
            ],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x47b,
            &[TestArg::Int(1535), TestArg::Int(5)],
        );

        exec_scene(&mut cvns, &mut fc, &mut fl, 0x470, &[TestArg::Int(10)]);
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x476,
            &[
                TestArg::Int(10),
                TestArg::Int(1),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Int(4),
                TestArg::Int(4),
            ],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x478,
            &[TestArg::Int(10), TestArg::Int(2)],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x47b,
            &[TestArg::Int(10), TestArg::Int(5)],
        );

        // Because sprite 10's image has use_alpha=false, pixel alpha 0 is ignored -> solid green covers red!
        let frame = crate::scene_render::cvns::render(&cvns, false, fc, fl, 8, 8).unwrap();
        assert_eq!(&frame.pixels()[..4], &[0, 200, 0, 255]);

        // Enabling use_alpha=true on slot 2 makes alpha 0 invisible, exposing sprite 1535's red!
        cvns.images[2].as_mut().unwrap().layers[0]
            .as_mut()
            .unwrap()
            .use_alpha = true;
        let frame = crate::scene_render::cvns::render(&cvns, false, fc, fl, 8, 8).unwrap();
        assert_eq!(&frame.pixels()[..4], &[255, 0, 0, 255]);

        // Putting sprite 1535 at z=1 puts it in front of solid black rect sprite 10 at z=5 (127 -> 0 order).
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x47b,
            &[TestArg::Int(1535), TestArg::Int(1)],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x472,
            &[
                TestArg::Int(10),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Int(4),
                TestArg::Int(4),
                TestArg::Int(0),
            ],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x47b,
            &[TestArg::Int(10), TestArg::Int(5)],
        );
        let _ = exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x47d,
            &[TestArg::Int(1535), TestArg::Int(0)],
        );
        // Sprite 1535 (red, z=1) covers solid black rect sprite 10 (z=5).
        let frame = crate::scene_render::cvns::render(&cvns, false, fc, fl, 8, 8).unwrap();
        assert_eq!(&frame.pixels()[..4], &[255, 0, 0, 255]);

        // 0x218f fade to white (0xffffff) + bit-0 exemption on sprite 10 at (4, 0).
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x479,
            &[
                TestArg::Int(10),
                TestArg::Int(4),
                TestArg::Int(0),
                TestArg::Int(4),
                TestArg::Int(4),
            ],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x48b,
            &[TestArg::Int(10), TestArg::Int(1)],
        );
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x58f,
            &[
                TestArg::Int(0),
                TestArg::Int(0x00ff_ffff),
                TestArg::Int(100),
            ],
        );
        cvns.advance_frame(100, &mut fc, &mut fl);
        assert_eq!(fl, 0);
        let frame = crate::scene_render::cvns::render(&cvns, false, fc, fl, 8, 8).unwrap();
        assert_eq!(&frame.pixels()[..4], &[255, 255, 255, 255]); // sprite 1535 faded to white
        assert_eq!(&frame.pixels()[4 * 4..4 * 4 + 4], &[0, 0, 0, 255]); // exempt black solid rect

        // Quake kind=1 shifts sprites without shifting windows.
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x593,
            &[
                TestArg::Int(1),
                TestArg::Int(3),
                TestArg::Int(0),
                TestArg::Int(100),
            ],
        );
        cvns.advance_frame(20, &mut fc, &mut fl);
        assert_eq!(cvns.quake.as_ref().unwrap().offset_for(false)[0].abs(), 3);
        assert_eq!(cvns.quake.as_ref().unwrap().offset_for(true), [0, 0]);
    }

    #[test]
    fn mask_capture_and_curtain_wait_interrupt_work() {
        let mut images = MemImages(HashMap::new());
        let mut cvns = CvnsScene::default();
        let (mut fc, mut fl) = (0u32, 0x100u16);

        // Captured old screen = pure red (4x4), new scene = pure green (4x4) on sprite 1.
        let captured = RgbaImage::new(4, 4, [255, 0, 0, 255].repeat(16)).unwrap();
        // Horizontal gradient: g=0 on x=0, g=80 on x=1, g=160 on x=2, g=240 on x=3.
        let mut grad = Vec::with_capacity(16 * 4);
        for _y in 0..4u8 {
            for x in 0..4u8 {
                let g = x * 80;
                grad.extend_from_slice(&[g, g, g, 255]);
            }
        }
        let mask = RgbaImage::new(4, 4, grad).unwrap();
        cvns.install_mask_capture(captured, Some(&mask), 200, 64)
            .unwrap();
        assert!(cvns.sprites[0].is_some() && cvns.images[0].is_some());

        exec_img(
            &mut cvns,
            &mut images,
            0x450,
            &[TestArg::Int(1), TestArg::Int(4), TestArg::Int(4)],
        );
        exec_img(
            &mut cvns,
            &mut images,
            0x466,
            &[TestArg::Int(1), TestArg::Int(0x0000_ff00)],
        );
        exec_scene(&mut cvns, &mut fc, &mut fl, 0x470, &[TestArg::Int(1)]);
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x476,
            &[
                TestArg::Int(1),
                TestArg::Int(1),
                TestArg::Int(0),
                TestArg::Int(0),
                TestArg::Int(4),
                TestArg::Int(4),
            ],
        );

        // Midway: left column is already revealed (green behind sprite 0), right column still captured red.
        cvns.advance_frame(100, &mut fc, &mut fl);
        let mid = crate::scene_render::cvns::render(&cvns, false, fc, fl, 4, 4).unwrap();
        assert!(mid.pixels()[3 * 4] > 200 && mid.pixels()[3 * 4 + 1] < 50);

        // On completion, sprite 0 and image 0 are deleted.
        cvns.advance_frame(200, &mut fc, &mut fl);
        assert!(cvns.sprites[0].is_none() && cvns.images[0].is_none());

        // Curtain + wait interrupt via virtual WaitInterrupt key (code 1 = left click / skip).
        exec_scene(
            &mut cvns,
            &mut fc,
            &mut fl,
            0x58d,
            &[
                TestArg::Int(1),
                TestArg::Int(0),
                TestArg::Int(10),
                TestArg::Int(2),
                TestArg::Int(5),
                TestArg::Int(10),
            ],
        );
        let mut input = ActionInput::default();
        let (_vm, flow) = exec_wait(
            &mut cvns,
            0x58e,
            &[TestArg::Int(1), TestArg::Int(1 << 26), TestArg::Int(1)],
            &mut input,
            true,
        );
        assert_eq!(flow, Flow::Wait);

        let mut codes = CodeInput::default();
        let bindings = KeyBindings::default();
        let mut now = [false; 128];
        now[1] = true; // triggers left-click edge (1 << 26) for ActionInput::cvns_wait_interrupt
        codes.update(&bindings, &now, &mut input);
        let (vm, flow) = exec_wait(
            &mut cvns,
            0x58e,
            &[TestArg::Int(1), TestArg::Int(1 << 26), TestArg::Int(1)],
            &mut input,
            true,
        );
        assert_eq!(flow, Flow::Yield);
        assert_eq!(vm.state().system_integer(0), Some(1));
        assert!(cvns.sprites[1].as_ref().unwrap().curtain.is_none());
    }
}

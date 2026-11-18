//! CVNS (`cvns-20090203`) message windows, text layout/reveal, local sprites, and inline choices.
//!
//! Models `*(this + 3460..3468)` (8 window surfaces, `0x14c` bytes each) and `*(this + 3456..3460)`
//! (4 message box controllers, `0x738` bytes each), mirroring `sub_45E390..0x464B17`.
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow},
    evaluate::error,
    image::{self, ImageSource},
    input::ActionInput,
    message::{AutoAdvanceContext, GlyphMask, RecordedGlyph, TextObject},
    message_flags::MessageFlags,
    scene::{
        DrawState, ObjectImage, SceneObject,
        cvns::{CVNS_ANIM_FRAMES, CvnsImageLayer, CvnsSprite, CvnsSpriteKind},
    },
    settings::TextOutputSettings,
};
use std::{collections::BTreeMap, error::Error, sync::Arc};

/// Eight independently configurable CVNS text window surfaces (`*(this + 3460..3468)`).
pub const CVNS_WINDOWS: usize = 8;
/// Four CVNS message controllers (`*(this + 3456..3460)`).
pub const CVNS_MESSAGES: usize = 4;
/// Six local image slots owned per CVNS message controller (`*(msg + 84..108)`).
pub const CVNS_LOCAL_IMAGES: usize = 6;
/// Twenty local sprites owned per CVNS message controller (`*(msg + 4..84)`).
pub const CVNS_LOCAL_SPRITES: usize = 20;
/// Twelve inline click buttons owned per CVNS message controller (`*(msg + 148..1684)`).
pub const CVNS_INLINE_BUTTONS: usize = 12;
/// Six visual states stored per CVNS inline button (`0x2150`).
pub const CVNS_BUTTON_STATES: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvnsWindow {
    pub position: [i32; 2],
    pub size: [i32; 2],
    pub area: [i32; 4],
    pub cursor: [i32; 2],
    pub glyph_size: i32,
    pub spacing: [i32; 2],
    pub face: Vec<u8>,
    pub colors: [u32; 2],
    pub gradient_dir: u32,
    pub char_delay_ms: i32,
    pub reveal: [i32; 2],
    pub kinsoku: bool,
    pub visible: bool,
    pub z: i32,
    pub ruby: bool,
    pub ruby_size: i32,
    pub ruby_spacing: i32,
    pub ruby_always: bool,
}

impl Default for CvnsWindow {
    fn default() -> Self {
        Self {
            position: [0, 0],
            size: [0, 0],
            area: [0, 0, 0, 0],
            cursor: [0, 0],
            glyph_size: 24,
            spacing: [0, 0],
            face: Vec::new(),
            colors: [0x00ff_ffff, 0x00ff_ffff],
            gradient_dir: 0,
            char_delay_ms: 30,
            reveal: [0, 0],
            kinsoku: true,
            visible: true,
            z: 1,
            ruby: false,
            ruby_size: 12,
            ruby_spacing: 0,
            ruby_always: false,
        }
    }
}

impl CvnsWindow {
    fn line_end(&self) -> i32 {
        self.area[0] + self.area[2] - (self.glyph_size + self.spacing[0])
    }

    fn apply_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.area = [x, y, w, h];
        self.cursor = [x, y];
    }

    fn clear_glyphs(&mut self, text: &mut TextObject) {
        text.glyphs.clear();
        self.cursor = [self.area[0], self.area[1]];
    }

    pub(crate) fn new_text(&self, default_face: &[u8], screen_size: [i32; 2]) -> TextObject {
        let mut text = TextObject {
            font_face: default_face.to_vec(),
            face: Arc::from(default_face),
            bold: false,
            revision_alpha: false,
            scale_bits: 1.0f32.to_bits(),
            logical_size: screen_size,
            position: [0, 0],
            area: [0; 4],
            cursor: [0, 0],
            ruby_lines: [0; 32],
            layout_scratch: [0; 2],
            glyphs: Vec::new(),
            glyph_size: 0,
            spacing: [0, 0],
            color: 0xff_ffff,
            decoration_color: 0xff_ffff,
            outline_mode: 2,
            glyph_parameter: 0,
            decoration: 0,
            reveal: [0, 0],
            char_delay: 30,
            layer: 1,
            visible: true,
            ruby: false,
            ruby_every_line: false,
            ruby_size: 12,
            ruby_spacing: 0,
            line_break: None,
            glyph_capacity: 4096,
        };
        self.sync_metadata_to_text(&mut text, false, default_face);
        text
    }

    fn sync_metadata_to_text(&self, text: &mut TextObject, paused: bool, default_face: &[u8]) {
        text.position = self.position;
        text.area = self.area;
        text.cursor = self.cursor;
        text.glyph_size = self.glyph_size;
        text.spacing = self.spacing;
        text.color = self.colors[0];
        text.decoration_color = self.colors[1];
        text.decoration = self.gradient_dir as i32;
        text.reveal = self.reveal;
        text.char_delay = self.char_delay_ms;
        text.layer = self.z;
        text.visible = self.visible && !paused;
        text.ruby = self.ruby;
        text.ruby_size = self.ruby_size;
        text.ruby_spacing = self.ruby_spacing;
        text.ruby_every_line = self.ruby_always;
        text.font_face = self.face.clone();
        let face = if self.face.is_empty() {
            default_face
        } else {
            &self.face
        };
        if text.face.as_ref() != face {
            text.face = Arc::from(face);
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CvnsButtonState {
    pub image_slot: i16,
    pub sx: i16,
    pub sy: i16,
    pub sw: i16,
    pub sh: i16,
    pub dx: i16,
    pub dy: i16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CvnsInlineButton {
    pub enabled: bool,
    pub sprite_slot: u16,
    pub cur_state: u16,
    pub prev_state: u16,
    /// Hit rectangle `[x, y, width, height]` in screen coordinates.
    pub hit: [i32; 4],
    pub states: [CvnsButtonState; CVNS_BUTTON_STATES],
}

#[derive(Debug, Clone)]
pub struct CvnsMessage {
    pub win_slot: usize,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    pub paused: bool,
    pub wait_cursor_mode: u16,
    pub wait_cursor_offset: [i16; 2],
    pub attached_flags: [bool; CVNS_LOCAL_SPRITES],
    pub attached_offsets: [[i16; 2]; CVNS_LOCAL_SPRITES],
    pub local_images: Vec<Option<CvnsImageLayer>>,
    pub local_sprites: Vec<Option<CvnsSprite>>,
    pub buttons: [CvnsInlineButton; CVNS_INLINE_BUTTONS],
    cached_objects: Vec<SceneObject>,
}

/// External `MSK0` glyph sheet (`0x600`) and Shift-JIS tile cache (`0x601`).
#[derive(Debug, Clone, Default)]
pub(crate) struct ExternalGlyphSheet {
    pub glyph_size: u32,
    pub width: u32,
    pub height: u32,
    pub mask: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternalGlyphBank {
    sheets: Vec<ExternalGlyphSheet>,
    glyphs: BTreeMap<u16, BTreeMap<u32, GlyphMask>>,
}

impl ExternalGlyphBank {
    pub(crate) fn add_sheet(&mut self, glyph_size: u32, width: u32, height: u32, mask: Vec<u8>) {
        self.sheets.push(ExternalGlyphSheet {
            glyph_size,
            width,
            height,
            mask,
        });
    }

    pub(crate) fn extract_glyph(&mut self, tile_index: u32, sjis_code: u16) {
        // The original lookup stops at the first matching character, then searches its sizes.
        if self.glyphs.contains_key(&sjis_code) {
            return;
        }
        let mut sizes = BTreeMap::new();
        for sheet in &self.sheets {
            let size = sheet.glyph_size;
            if size == 0 || sheet.width < size || sheet.height < size {
                continue;
            }
            let cols = sheet.width / size;
            if tile_index / cols >= sheet.height / size || sizes.contains_key(&size) {
                continue;
            }
            let tx = (tile_index % cols) * size;
            let ty = (tile_index / cols) * size;
            let mut tile = Vec::with_capacity((size * size) as usize);
            for y in 0..size {
                let row = ((ty + y) * sheet.width + tx) as usize;
                tile.extend_from_slice(&sheet.mask[row..row + size as usize]);
            }
            sizes.insert(size, GlyphMask::new(tile));
        }
        if !sizes.is_empty() {
            self.glyphs.insert(sjis_code, sizes);
        }
    }

    pub(crate) fn glyph(&self, sjis_code: u16, glyph_size: u32) -> Option<&GlyphMask> {
        self.glyphs.get(&sjis_code)?.get(&glyph_size)
    }
}

pub(crate) struct RestoredCvnsMessage {
    pub win_slot: usize,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
    pub paused: bool,
    pub wait_cursor_mode: u16,
    pub wait_cursor_offset: [i16; 2],
    pub attached_flags: [bool; CVNS_LOCAL_SPRITES],
    pub attached_offsets: [[i16; 2]; CVNS_LOCAL_SPRITES],
    pub local_images: Vec<Option<CvnsImageLayer>>,
    pub local_sprites: Vec<Option<CvnsSprite>>,
    pub buttons: [CvnsInlineButton; CVNS_INLINE_BUTTONS],
}

impl CvnsMessage {
    pub(crate) fn from_saved(saved: RestoredCvnsMessage) -> Self {
        let mut msg = Self {
            win_slot: saved.win_slot,
            x: saved.x,
            y: saved.y,
            w: saved.w,
            h: saved.h,
            visible: saved.visible,
            paused: saved.paused,
            wait_cursor_mode: saved.wait_cursor_mode,
            wait_cursor_offset: saved.wait_cursor_offset,
            attached_flags: saved.attached_flags,
            attached_offsets: saved.attached_offsets,
            local_images: saved.local_images,
            local_sprites: saved.local_sprites,
            buttons: saved.buttons,
            cached_objects: Vec::new(),
        };
        msg.rebuild_cached_objects();
        msg
    }

    fn new(win_slot: usize, x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            win_slot,
            x,
            y,
            w,
            h,
            visible: false,
            paused: false,
            wait_cursor_mode: 0,
            wait_cursor_offset: [0, 0],
            attached_flags: [false; CVNS_LOCAL_SPRITES],
            attached_offsets: [[0; 2]; CVNS_LOCAL_SPRITES],
            local_images: vec![None; CVNS_LOCAL_IMAGES],
            local_sprites: vec![None; CVNS_LOCAL_SPRITES],
            buttons: [CvnsInlineButton::default(); CVNS_INLINE_BUTTONS],
            cached_objects: Vec::new(),
        }
    }

    fn rebuild_cached_objects(&mut self) {
        self.cached_objects.clear();
        if !self.visible {
            return;
        }
        // `sub_41D110` inserts `m = 0..4`, `k = 0..20` at head -> `sub_41E000` draws `(3,19)..(0,0)`.
        // Store forward here because `scene_render::cvns` draws `additional.iter().rev()`.
        for sprite in self.local_sprites.iter().flatten() {
            if !sprite.visible || sprite.finished_delete || sprite.alpha == 0 {
                continue;
            }
            if let Some(obj) = local_sprite_to_scene_object(sprite, &self.local_images) {
                self.cached_objects.push(obj);
            }
        }
    }

    fn show(&mut self, win: &mut CvnsWindow, show: bool) {
        self.visible = show;
        self.paused = false;
        win.visible = show;
        if let Some(bg) = self.local_sprites[0].as_mut() {
            if show {
                bg.x = self.x;
                bg.y = self.y;
            }
            bg.visible = show;
        }
        for btn in &self.buttons {
            if !btn.enabled {
                continue;
            }
            if let Some(sp) = self
                .local_sprites
                .get_mut(usize::from(btn.sprite_slot))
                .and_then(Option::as_mut)
            {
                if show && let Some(st) = btn.states.get(usize::from(btn.prev_state)) {
                    let img = &self
                        .local_images
                        .get(st.image_slot as usize)
                        .and_then(Option::as_ref);
                    let dims = img.map_or((i32::from(st.sw), i32::from(st.sh)), |l| {
                        (l.width() as i32, l.height() as i32)
                    });
                    sp.set_src_rect(
                        st.image_slot,
                        i32::from(st.sx),
                        i32::from(st.sy),
                        i32::from(st.sw),
                        i32::from(st.sh),
                        dims,
                        false,
                        false,
                    );
                    sp.x = self.x + i32::from(st.dx);
                    sp.y = self.y + i32::from(st.dy);
                }
                sp.visible = show;
            }
        }
        if !show && let Some(cur) = self.local_sprites[1].as_mut() {
            cur.visible = false;
            cur.stop_anim();
        }
        for slot in 0..CVNS_LOCAL_SPRITES {
            if self.attached_flags[slot]
                && let Some(sp) = self.local_sprites[slot].as_mut()
            {
                if show {
                    sp.x = self.x + i32::from(self.attached_offsets[slot][0]);
                    sp.y = self.y + i32::from(self.attached_offsets[slot][1]);
                }
                sp.visible = show;
            }
        }
        self.rebuild_cached_objects();
    }

    fn set_wait_cursor(&mut self, active: bool) {
        let Some(cur) = self.local_sprites[1].as_mut() else {
            return;
        };
        if active {
            if !cur.visible {
                cur.visible = true;
                cur.x = self.x + i32::from(self.wait_cursor_offset[0]);
                cur.y = self.y + i32::from(self.wait_cursor_offset[1]);
                cur.start_anim(1);
            }
        } else if cur.visible {
            cur.stop_anim();
            cur.visible = false;
        }
        self.rebuild_cached_objects();
    }

    fn poll_buttons(&self, input: &mut ActionInput, consume_click: bool) -> (i32, bool) {
        let [px, py] = input.pointer();
        let mut hit_idx = -1i32;
        let mut clicked = false;
        for (i, btn) in self.buttons.iter().enumerate() {
            if !btn.enabled {
                continue;
            }
            let [x, y, w, h] = btn.hit;
            if px >= x && py >= y && px < x + w && py < y + h {
                hit_idx = i as i32;
                if input.mouse(0)[2] {
                    if consume_click {
                        input.consume_mouse_press(0);
                    }
                    clicked = true;
                }
            }
        }
        (hit_idx, clicked)
    }

    fn advance(&mut self, delta_ms: u32) {
        let mut dirty = false;
        for sp in self.local_sprites.iter_mut().flatten() {
            let was = (sp.visible, sp.x, sp.y, sp.alpha, sp.src_rect);
            sp.advance(delta_ms);
            if sp.finished_delete || (sp.visible, sp.x, sp.y, sp.alpha, sp.src_rect) != was {
                dirty = true;
            }
        }
        if dirty {
            self.rebuild_cached_objects();
        }
    }
}

fn local_sprite_to_scene_object(
    sprite: &CvnsSprite,
    local_images: &[Option<CvnsImageLayer>],
) -> Option<SceneObject> {
    let mut draw = DrawState::default();
    draw.words[2] = (sprite.src_rect[0] as i16 as u16) as u32;
    draw.words[3] = (sprite.src_rect[1] as i16 as u16) as u32;
    draw.words[4] = (sprite.w as i16 as u16) as u32;
    draw.words[5] = (sprite.h as i16 as u16) as u32;
    draw.words[6] = (sprite.origin_x as i16 as u16) as u32;
    draw.words[7] = (sprite.origin_y as i16 as u16) as u32;
    draw.words[8] = sprite.x as u32;
    draw.words[9] = sprite.y as u32;
    draw.words[10] = sprite.z as u32;
    draw.words[16] = u32::from(sprite.blend_mode == 1);
    draw.words[17] = u32::from(sprite.alpha.min(256));
    draw.words[18] = (sprite.scale_x as f32 / 1000.0).to_bits();
    draw.words[19] = (sprite.scale_y as f32 / 1000.0).to_bits();
    draw.words[20] = sprite.rotation as u32;

    let (image, color_flag) = match sprite.kind {
        CvnsSpriteKind::Image => {
            let layer = local_images
                .get(sprite.image_slot as usize)
                .and_then(Option::as_ref)?;
            (
                Some(ObjectImage {
                    filter: None,
                    name: layer.name.clone(),
                    pixels: layer.active_pixels().clone(),
                    alpha: layer.use_alpha,
                    surface: false,
                }),
                sprite.flags,
            )
        }
        CvnsSpriteKind::SolidRect { color } => {
            draw.words[14] = color & 0x00ff_ffff;
            (None, sprite.flags)
        }
        CvnsSpriteKind::Video => return None,
    };
    Some(SceneObject {
        shown: sprite.visible,
        flags: color_flag,
        image,
        draw: Some(draw),
        ..Default::default()
    })
}

#[inline]
fn is_sjis_lead(b: u8) -> bool {
    matches!(b, 0x81..=0x9f | 0xe0..=0xfc)
}

#[inline]
fn is_kinsoku_char(code: u16) -> bool {
    matches!(code, 33089 | 33090 | 33142 | 33144)
}

#[inline]
fn punctuation_pause(code: u16, next_pair: Option<u16>) -> usize {
    if code == 33089 {
        1
    } else if code == 33090 {
        2
    } else if matches!(next_pair, Some(33088)) && code == 33097 {
        3
    } else if matches!(next_pair, Some(33088)) && code == 33096 {
        4
    } else {
        0
    }
}

/// Counts lines needed to render `bytes` inside `win` (`sub_443920`) and populates `ruby_lines`.
pub(crate) fn layout_line_count(win: &CvnsWindow, bytes: &[u8], ruby_lines: &mut [u32; 32]) -> u32 {
    let mut x = win.cursor[0];
    let mut line = 0usize;
    let limit = win.line_end();
    let full_step = win.glyph_size + win.spacing[0];
    let half_step = (win.glyph_size >> 1) + win.spacing[0];
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != 0 {
        match bytes[i] {
            b'\\' => match bytes.get(i + 1) {
                Some(b'n') => {
                    x = win.area[0];
                    line += 1;
                    i += 2;
                }
                Some(b'c') => i += 2,
                Some(b'w') => i += 3,
                _ => i += 1,
            },
            b'{' => {
                if let Some(dst) = ruby_lines.get_mut(line) {
                    *dst = 1;
                }
                i += 1;
                while i < bytes.len() && bytes[i] != 0 && bytes[i] != b'/' {
                    if is_sjis_lead(bytes[i]) && i + 1 < bytes.len() && bytes[i + 1] != 0 {
                        i += 2;
                        x += full_step;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() && bytes[i] == b'/' {
                    i += 1;
                }
                while i < bytes.len() && bytes[i] != 0 && bytes[i] != b'}' {
                    if is_sjis_lead(bytes[i]) && i + 1 < bytes.len() && bytes[i + 1] != 0 {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() && bytes[i] == b'}' {
                    i += 1;
                }
            }
            b => {
                if is_sjis_lead(b) && i + 1 < bytes.len() && bytes[i + 1] != 0 {
                    let code = (u16::from(b) << 8) | u16::from(bytes[i + 1]);
                    i += 2;
                    if x >= limit && (!win.kinsoku || !is_kinsoku_char(code)) {
                        x = win.area[0];
                        line += 1;
                    }
                    x += full_step;
                } else {
                    i += 1;
                    if x >= limit {
                        x = win.area[0];
                        line += 1;
                    }
                    x += half_step;
                }
            }
        }
    }
    (line + 1) as u32
}

#[derive(Debug, Default)]
pub struct CvnsMessageHost {
    pub windows: [Option<CvnsWindow>; CVNS_WINDOWS],
    pub messages: [Option<CvnsMessage>; CVNS_MESSAGES],
    pub text_shadow: bool,
    pub read_skip: bool,
    pub auto_skip: bool,
    pub auto_wait_elapsed_ms: u32,
    pub click_wait_elapsed_ms: u32,
}

impl CvnsMessageHost {
    pub(crate) fn reset(&mut self) {
        self.windows = Default::default();
        self.messages = Default::default();
        self.auto_wait_elapsed_ms = 0;
        self.click_wait_elapsed_ms = 0;
    }

    pub(crate) fn visible_objects(&self) -> impl Iterator<Item = &SceneObject> {
        self.messages
            .iter()
            .flatten()
            .filter(|m| m.visible)
            .flat_map(|m| m.cached_objects.iter())
    }

    pub(crate) fn advance_frame(
        &mut self,
        delta_ms: u32,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        default_face: &[u8],
    ) {
        for msg in self.messages.iter_mut().flatten() {
            msg.advance(delta_ms);
            if let Some(win) = self.windows.get(msg.win_slot).and_then(Option::as_ref)
                && let Some(text) = texts.get_mut(msg.win_slot).and_then(Option::as_mut)
            {
                win.sync_metadata_to_text(text, msg.paused, default_face);
            }
        }
    }

    fn check_win_slot(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) < CVNS_WINDOWS {
            Ok(slot as usize)
        } else {
            Err(error(pc, op, "CVNS window slot outside 0..7"))
        }
    }

    fn check_msg_slot(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) < CVNS_MESSAGES {
            Ok(slot as usize)
        } else {
            Err(error(pc, op, "CVNS message slot outside 0..3"))
        }
    }

    fn check_local_sprite(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) < CVNS_LOCAL_SPRITES {
            Ok(slot as usize)
        } else {
            Err(error(pc, op, "CVNS local sprite slot outside 0..19"))
        }
    }

    fn check_local_image(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) < CVNS_LOCAL_IMAGES {
            Ok(slot as usize)
        } else {
            Err(error(pc, op, "CVNS local image outside 0..5"))
        }
    }

    fn check_button_slot(pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        if (slot as usize) < CVNS_INLINE_BUTTONS {
            Ok(slot as usize)
        } else {
            Err(error(pc, op, "CVNS button slot outside 0..11"))
        }
    }

    fn require_win_mut(&mut self, pc: usize, op: u16, slot: u32) -> Result<usize, DecodeError> {
        let idx = Self::check_win_slot(pc, op, slot)?;
        if self.windows[idx].is_none() {
            return Err(error(pc, op, "CVNS window slot is empty"));
        }
        Ok(idx)
    }

    fn require_msg_win_mut(
        &mut self,
        pc: usize,
        op: u16,
        msg_slot: u32,
    ) -> Result<(usize, usize), DecodeError> {
        let idx = Self::check_msg_slot(pc, op, msg_slot)?;
        let Some(msg) = self.messages[idx].as_ref() else {
            return Err(error(pc, op, "CVNS message slot is empty"));
        };
        let win_idx = msg.win_slot;
        if self.windows.get(win_idx).and_then(Option::as_ref).is_none() {
            return Err(error(pc, op, "CVNS message paired window is empty"));
        }
        Ok((idx, win_idx))
    }

    fn require_local_sprite_mut(
        &mut self,
        pc: usize,
        op: u16,
        msg_slot: u32,
        sp_slot: u32,
    ) -> Result<(usize, usize), DecodeError> {
        let (m, _) = self.require_msg_win_mut(pc, op, msg_slot)?;
        let s = Self::check_local_sprite(pc, op, sp_slot)?;
        let msg = self.messages[m].as_ref().expect("checked");
        if msg.local_sprites[s].is_none() {
            return Err(error(pc, op, "CVNS local sprite slot is empty"));
        }
        Ok((m, s))
    }

    fn init_window(
        &mut self,
        win_idx: usize,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        default_face: &[u8],
        screen_size: [i32; 2],
    ) {
        let win = CvnsWindow::default();
        let text = win.new_text(default_face, screen_size);
        self.windows[win_idx] = Some(win);
        texts[win_idx] = Some(text);
    }

    fn delete_window(
        &mut self,
        win_idx: usize,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
    ) {
        self.windows[win_idx] = None;
        texts[win_idx] = None;
    }

    fn sync_win(
        &self,
        win_idx: usize,
        paused: bool,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        default_face: &[u8],
    ) {
        if let (Some(win), Some(text)) = (&self.windows[win_idx], &mut texts[win_idx]) {
            win.sync_metadata_to_text(text, paused, default_face);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_text_to_window(
        &mut self,
        win_idx: usize,
        bytes: &[u8],
        animated: bool,
        instant: bool,
        settings: TextOutputSettings,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        default_face: &[u8],
    ) {
        let Some(win) = self.windows[win_idx].as_mut() else {
            return;
        };
        let Some(text) = texts[win_idx].as_mut() else {
            return;
        };
        let mut ruby_lines = [0u32; 32];
        if win.ruby {
            let _ = layout_line_count(win, bytes, &mut ruby_lines);
            if ruby_lines[0] != 0 || win.ruby_always {
                win.cursor[1] += win.ruby_size;
            }
        }
        let base_step = if animated && !instant {
            match settings.speed {
                0 => 0,
                1 => (win.char_delay_ms as u32) >> 1,
                2 => win.char_delay_ms as u32,
                3 => ((win.char_delay_ms as u32) * 3) >> 1,
                4 => (win.char_delay_ms as u32) * 2,
                _ => win.char_delay_ms as u32,
            }
        } else {
            0
        };
        let limit = win.line_end();
        let line_step = win.glyph_size + win.spacing[1];
        let full_step = win.glyph_size + win.spacing[0];
        let half_step = (win.glyph_size >> 1) + win.spacing[0];
        let face_arc: Arc<[u8]> = if win.face.is_empty() {
            Arc::from(default_face)
        } else {
            Arc::from(win.face.as_slice())
        };
        let mut line_idx = 0usize;
        let mut wc_instant = false;
        let mut total_delay = 0u32;
        let mut i = 0usize;

        while i < bytes.len() && bytes[i] != 0 {
            if bytes[i] == b'\\' {
                match bytes.get(i + 1) {
                    Some(b'n') => {
                        win.cursor[0] = win.area[0];
                        win.cursor[1] += line_step;
                        line_idx += 1;
                        if win.ruby
                            && (ruby_lines.get(line_idx).copied().unwrap_or(0) != 0
                                || win.ruby_always)
                        {
                            win.cursor[1] += win.ruby_size;
                        }
                        i += 2;
                    }
                    Some(b'c') => {
                        text.glyphs.clear();
                        win.cursor = [win.area[0], win.area[1]];
                        i += 2;
                    }
                    Some(b'w') => {
                        match bytes.get(i + 2) {
                            Some(b'c') => {
                                wc_instant = true;
                                total_delay = 0;
                            }
                            Some(b's') => wc_instant = false,
                            _ => {}
                        }
                        i += 3;
                    }
                    _ => i += 1,
                }
                continue;
            }
            if bytes[i] == b'{' {
                let start_x = win.cursor[0];
                let ruby_y = win.cursor[1] - win.ruby_size;
                i += 1;
                while i < bytes.len() && bytes[i] != 0 && bytes[i] != b'/' {
                    if is_sjis_lead(bytes[i]) && i + 1 < bytes.len() && bytes[i + 1] != 0 {
                        let code = (u16::from(bytes[i]) << 8) | u16::from(bytes[i + 1]);
                        push_cvns_glyph(
                            win,
                            text,
                            &face_arc,
                            code,
                            win.cursor,
                            win.glyph_size as u32,
                            self.text_shadow,
                            animated,
                            wc_instant,
                            &mut total_delay,
                            base_step,
                            0,
                        );
                        win.cursor[0] += full_step;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() && bytes[i] == b'/' {
                    i += 1;
                }
                let ruby_start = i;
                let mut ruby_count = 0i32;
                let mut scan = ruby_start;
                while scan < bytes.len() && bytes[scan] != 0 && bytes[scan] != b'}' {
                    if is_sjis_lead(bytes[scan]) && scan + 1 < bytes.len() && bytes[scan + 1] != 0 {
                        ruby_count += 1;
                        scan += 2;
                    } else {
                        scan += 1;
                    }
                }
                if win.ruby && ruby_count > 0 {
                    let ruby_total_fp = (ruby_count * win.ruby_size) << 10;
                    let base_span_fp = (win.cursor[0] - start_x) << 10;
                    let (mut rx_fp, step_fp) = if ruby_total_fp <= base_span_fp {
                        let pad = (base_span_fp - ruby_total_fp) / (2 * ruby_count);
                        (
                            (start_x << 10) + pad,
                            (win.ruby_size << 10) + win.ruby_spacing + 2 * pad,
                        )
                    } else {
                        (
                            (start_x << 10) - ((ruby_total_fp - base_span_fp) >> 1),
                            win.ruby_size << 10,
                        )
                    };
                    let mut r = ruby_start;
                    while r < scan {
                        if is_sjis_lead(bytes[r]) && r + 1 < scan {
                            let code = (u16::from(bytes[r]) << 8) | u16::from(bytes[r + 1]);
                            push_cvns_glyph(
                                win,
                                text,
                                &face_arc,
                                code,
                                [rx_fp >> 10, ruby_y],
                                win.ruby_size as u32,
                                self.text_shadow,
                                animated,
                                wc_instant,
                                &mut total_delay,
                                base_step,
                                0,
                            );
                            rx_fp += step_fp;
                            r += 2;
                        } else {
                            r += 1;
                        }
                    }
                }
                i = scan;
                if i < bytes.len() && bytes[i] == b'}' {
                    i += 1;
                }
                continue;
            }
            if is_sjis_lead(bytes[i]) && i + 1 < bytes.len() && bytes[i + 1] != 0 {
                let code = (u16::from(bytes[i]) << 8) | u16::from(bytes[i + 1]);
                let next_pair = (i + 3 < bytes.len() && is_sjis_lead(bytes[i + 2]))
                    .then(|| (u16::from(bytes[i + 2]) << 8) | u16::from(bytes[i + 3]));
                let pause = if bytes.get(i + 2).copied() == Some(0) {
                    0
                } else {
                    punctuation_pause(code, next_pair)
                };
                i += 2;
                if win.cursor[0] >= limit && (!win.kinsoku || !is_kinsoku_char(code)) {
                    win.cursor[0] = win.area[0];
                    win.cursor[1] += line_step;
                    line_idx += 1;
                    if win.ruby
                        && (ruby_lines.get(line_idx).copied().unwrap_or(0) != 0 || win.ruby_always)
                    {
                        win.cursor[1] += win.ruby_size;
                    }
                }
                push_cvns_glyph(
                    win,
                    text,
                    &face_arc,
                    code,
                    win.cursor,
                    win.glyph_size as u32,
                    self.text_shadow,
                    animated,
                    wc_instant,
                    &mut total_delay,
                    base_step,
                    pause,
                );
                win.cursor[0] += full_step;
            } else {
                let code = u16::from(bytes[i]);
                i += 1;
                if win.cursor[0] >= limit {
                    win.cursor[0] = win.area[0];
                    win.cursor[1] += line_step;
                    line_idx += 1;
                    if win.ruby
                        && (ruby_lines.get(line_idx).copied().unwrap_or(0) != 0 || win.ruby_always)
                    {
                        win.cursor[1] += win.ruby_size;
                    }
                }
                push_cvns_glyph(
                    win,
                    text,
                    &face_arc,
                    code,
                    win.cursor,
                    win.glyph_size as u32,
                    self.text_shadow,
                    animated,
                    wc_instant,
                    &mut total_delay,
                    base_step,
                    0,
                );
                win.cursor[0] += half_step;
            }
        }
        win.sync_metadata_to_text(text, false, default_face);
    }

    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        output_settings: TextOutputSettings,
        default_face: &[u8],
        screen_size: [i32; 2],
    ) -> Result<Flow, DecodeError> {
        let cmd = call.command;
        let op = cmd + 0x1c00;
        match cmd {
            // Window commands 0x20d0..=0x2101, 0x2208..=0x220f
            0x4d0 => {
                let idx = Self::check_win_slot(call.pc, op, call.argument(4, "window slot")?)?;
                call.complete(0x4004)?;
                self.init_window(idx, texts, default_face, screen_size);
            }
            0x4d1 => {
                let idx = Self::check_win_slot(call.pc, op, call.argument(4, "window slot")?)?;
                call.complete(0x4004)?;
                self.delete_window(idx, texts);
            }
            0x4d2 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let x = call.argument(8, "x")? as i32;
                let y = call.argument(12, "y")? as i32;
                call.complete(0x400c)?;
                self.windows[idx].as_mut().expect("checked").position = [x, y];
                self.sync_win(idx, false, texts, default_face);
            }
            0x4d3 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let w = call.argument(8, "width")? as i32;
                let h = call.argument(12, "height")? as i32;
                call.complete(0x400c)?;
                self.windows[idx].as_mut().expect("checked").size = [w, h];
            }
            0x4d4 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let x = call.argument(8, "area x")? as i32;
                let y = call.argument(12, "area y")? as i32;
                let w = call.argument(16, "area w")? as i32;
                let h = call.argument(20, "area h")? as i32;
                call.complete(0x4014)?;
                self.windows[idx]
                    .as_mut()
                    .expect("checked")
                    .apply_area(x, y, w, h);
                self.sync_win(idx, false, texts, default_face);
            }
            0x4d5 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                call.complete(0x4004)?;
                if let (Some(win), Some(text)) = (&mut self.windows[idx], &mut texts[idx]) {
                    win.clear_glyphs(text);
                    win.sync_metadata_to_text(text, false, default_face);
                }
            }
            0x4d6 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let size = call.argument(8, "font size")? as i32;
                let sx = call.argument(12, "spacing x")? as i32;
                let sy = call.argument(16, "line spacing")? as i32;
                call.complete(0x4010)?;
                let win = self.windows[idx].as_mut().expect("checked");
                if size > 0 {
                    win.glyph_size = size;
                }
                if sx >= 0 {
                    win.spacing[0] = sx;
                }
                if sy >= 0 {
                    win.spacing[1] = sy;
                }
                self.sync_win(idx, false, texts, default_face);
            }
            0x4d7 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let face_ref = call.argument(8, "font face")?;
                let face = call.vm.resolve_string(face_ref)?;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").face = face;
                self.sync_win(idx, false, texts, default_face);
            }
            0x4d8 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let top = call.argument(8, "top color")?;
                let bottom = call.argument(12, "bottom color")?;
                call.complete(0x400c)?;
                let win = self.windows[idx].as_mut().expect("checked");
                if top & 0xff00_0000 == 0 {
                    win.colors[0] = top;
                }
                if bottom & 0xff00_0000 == 0 {
                    win.colors[1] = bottom;
                }
                self.sync_win(idx, false, texts, default_face);
            }
            0x4d9 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let enable = call.argument(8, "kinsoku")? != 0;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").kinsoku = enable;
            }
            0x4dc => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let mode = call.argument(8, "reveal mode")? as i32;
                let dur = call.argument(12, "reveal duration")? as i32;
                call.complete(0x400c)?;
                self.windows[idx].as_mut().expect("checked").reveal = [mode, dur];
                self.sync_win(idx, false, texts, default_face);
            }
            0x4dd => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let delay = call.argument(8, "char delay")? as i32;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").char_delay_ms = delay;
                self.sync_win(idx, false, texts, default_face);
            }
            0x4de => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let x = call.argument(8, "cursor x")? as i32;
                let y = call.argument(12, "cursor y")? as i32;
                call.complete(0x400c)?;
                self.windows[idx].as_mut().expect("checked").cursor = [x, y];
                self.sync_win(idx, false, texts, default_face);
            }
            0x4df => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let txt_ref = call.argument(8, "text")?;
                let bytes = call.vm.resolve_string(txt_ref)?;
                call.complete(0x4008)?;
                self.write_text_to_window(
                    idx,
                    &bytes,
                    false,
                    true,
                    output_settings,
                    texts,
                    default_face,
                );
            }
            0x500 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let dir = call.argument(8, "gradient dir")?;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").gradient_dir = dir;
                self.sync_win(idx, false, texts, default_face);
            }
            0x501 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let z = call.argument(8, "z priority")? as i32;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").z = z;
                self.sync_win(idx, false, texts, default_face);
            }
            0x608 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let enable = call.argument(8, "ruby enable")? != 0;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").ruby = enable;
                self.sync_win(idx, false, texts, default_face);
            }
            0x609 => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let size = call.argument(8, "ruby size")? as i32;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").ruby_size = size;
                self.sync_win(idx, false, texts, default_face);
            }
            0x60a => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let sp = call.argument(8, "ruby spacing")? as i32;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").ruby_spacing = sp;
                self.sync_win(idx, false, texts, default_face);
            }
            0x60b => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let always = call.argument(8, "ruby always")? != 0;
                call.complete(0x4008)?;
                self.windows[idx].as_mut().expect("checked").ruby_always = always;
                self.sync_win(idx, false, texts, default_face);
            }
            0x60f => {
                let idx = self.require_win_mut(call.pc, op, call.argument(4, "window slot")?)?;
                let txt_ref = call.argument(8, "text")?;
                let bytes = call.vm.resolve_string(txt_ref)?;
                call.complete(0x4008)?;
                let mut ruby_lines = [0u32; 32];
                let count = layout_line_count(
                    self.windows[idx].as_ref().expect("checked"),
                    &bytes,
                    &mut ruby_lines,
                );
                call.vm
                    .state_mut()
                    .set_system_integer(0, count)
                    .expect("slot 0");
            }

            // Message box commands 0x2110..=0x2127, 0x2230
            0x510 => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                let w_idx = Self::check_win_slot(call.pc, op, call.argument(8, "window slot")?)?;
                let x = call.argument(12, "x")? as i32;
                let y = call.argument(16, "y")? as i32;
                let w = call.argument(20, "width")? as i32;
                let h = call.argument(24, "height")? as i32;
                call.complete(0x4018)?;
                self.init_window(w_idx, texts, default_face, screen_size);
                let win = self.windows[w_idx].as_mut().expect("just created");
                win.size = [w, h];
                win.position = [x, y];
                win.apply_area(0, 0, w, h);
                win.glyph_size = 24;
                win.colors = [0x00ff_ffff, 0x00ff_ffff];
                win.gradient_dir = 0;
                win.char_delay_ms = 30;
                win.reveal = [0, 0];
                win.kinsoku = true;
                win.z = 1;
                if let Some(text) = texts[w_idx].as_mut() {
                    win.sync_metadata_to_text(text, false, default_face);
                }
                self.messages[m] = Some(CvnsMessage::new(w_idx, x, y, w, h));
            }
            0x511 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let show = call.argument(8, "show")? != 0;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_mut().expect("checked");
                if msg.visible != show {
                    let win = self.windows[w_idx].as_mut().expect("checked");
                    msg.show(win, show);
                }
                if let Some(msg) = self.messages[m].as_mut() {
                    msg.paused = false;
                }
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x513 => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                call.complete(0x4004)?;
                if let Some(msg) = self.messages[m].take()
                    && msg.win_slot < CVNS_WINDOWS
                {
                    self.delete_window(msg.win_slot, texts);
                }
            }
            0x514 => {
                let (_m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let size = call.argument(8, "font size")? as i32;
                call.complete(0x4008)?;
                if size > 0 {
                    self.windows[w_idx].as_mut().expect("checked").glyph_size = size;
                }
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x515 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let slot =
                    Self::check_local_sprite(call.pc, op, call.argument(8, "local sprite")?)?;
                let attach = call.argument(12, "attach")? != 0;
                call.complete(0x400c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.attached_flags[slot] = attach;
                if attach && let Some(sp) = msg.local_sprites[slot].as_mut() {
                    if !msg.visible || msg.paused {
                        sp.visible = false;
                    } else {
                        sp.x = msg.x + i32::from(msg.attached_offsets[slot][0]);
                        sp.y = msg.y + i32::from(msg.attached_offsets[slot][1]);
                        sp.visible = true;
                    }
                    msg.rebuild_cached_objects();
                }
            }
            0x516 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let slot =
                    Self::check_local_sprite(call.pc, op, call.argument(8, "local sprite")?)?;
                let dx = call.argument(12, "dx")? as i16;
                let dy = call.argument(16, "dy")? as i16;
                call.complete(0x4010)?;
                self.messages[m].as_mut().expect("checked").attached_offsets[slot] = [dx, dy];
            }
            0x517 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let txt_ref = call.argument(8, "text")?;
                let bytes = call.vm.resolve_string(txt_ref)?;
                call.complete(0x4008)?;
                self.write_text_to_window(
                    w_idx,
                    &bytes,
                    false,
                    true,
                    output_settings,
                    texts,
                    default_face,
                );
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x518 => {
                let (_m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let sx = call.argument(8, "spacing x")? as i32;
                let sy = call.argument(12, "line spacing")? as i32;
                call.complete(0x400c)?;
                let win = self.windows[w_idx].as_mut().expect("checked");
                if sx >= 0 {
                    win.spacing[0] = sx;
                }
                if sy >= 0 {
                    win.spacing[1] = sy;
                }
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x519 => {
                let (_m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let delay = call.argument(8, "char delay")? as i32;
                call.complete(0x4008)?;
                self.windows[w_idx].as_mut().expect("checked").char_delay_ms = delay;
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x51a => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let mode = call.argument(8, "cursor mode")? as u16;
                let ox = call.argument(12, "ox")? as i16;
                let oy = call.argument(16, "oy")? as i16;
                call.complete(0x4010)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.wait_cursor_mode = mode;
                msg.wait_cursor_offset = if mode == 1 { [ox, oy] } else { [0, 0] };
            }
            0x51b => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let paused = call.argument(8, "paused")? != 0;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_mut().expect("checked");
                if msg.visible && self.windows[w_idx].is_some() {
                    msg.paused = paused;
                    self.sync_win(w_idx, paused, texts, default_face);
                }
            }
            0x51d => {
                let (_m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let x = call.argument(8, "area x")? as i32;
                let y = call.argument(12, "area y")? as i32;
                let w = call.argument(16, "area w")? as i32;
                let h = call.argument(20, "area h")? as i32;
                call.complete(0x4014)?;
                self.windows[w_idx]
                    .as_mut()
                    .expect("checked")
                    .apply_area(x, y, w, h);
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x51e => {
                let (_m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let top = call.argument(8, "top color")?;
                let bottom = call.argument(12, "bottom color")?;
                call.complete(0x400c)?;
                let win = self.windows[w_idx].as_mut().expect("checked");
                if top & 0xff00_0000 == 0 {
                    win.colors[0] = top;
                }
                if bottom & 0xff00_0000 == 0 {
                    win.colors[1] = bottom;
                }
                self.sync_win(w_idx, false, texts, default_face);
            }
            0x51f => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                call.complete(0x4004)?;
                let paused = self.messages[m].as_ref().is_some_and(|msg| msg.paused);
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(paused))
                    .expect("slot 0");
            }
            0x521 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                call.complete(0x4004)?;
                let paused = self.messages[m].as_ref().expect("checked").paused;
                if let (Some(win), Some(text)) = (&mut self.windows[w_idx], &mut texts[w_idx]) {
                    win.clear_glyphs(text);
                    win.sync_metadata_to_text(text, paused, default_face);
                }
            }
            0x522 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let x = call.argument(8, "cursor x")? as i32;
                let y = call.argument(12, "cursor y")? as i32;
                call.complete(0x400c)?;
                self.windows[w_idx].as_mut().expect("checked").cursor = [x, y];
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x523 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let mode = call.argument(8, "reveal mode")? as i32;
                let dur = call.argument(12, "reveal duration")? as i32;
                call.complete(0x400c)?;
                self.windows[w_idx].as_mut().expect("checked").reveal = [mode, dur];
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x524 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let dir = call.argument(8, "gradient dir")?;
                call.complete(0x4008)?;
                self.windows[w_idx].as_mut().expect("checked").gradient_dir = dir;
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x525 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let z = call.argument(8, "z priority")? as i32;
                call.complete(0x4008)?;
                self.windows[w_idx].as_mut().expect("checked").z = z;
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x526 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let mode = call.argument(8, "rel mode")?;
                let x = call.argument(12, "x")? as i32;
                let y = call.argument(16, "y")? as i32;
                call.complete(0x4010)?;
                let msg = self.messages[m].as_mut().expect("checked");
                let (nx, ny, dx, dy) = match mode {
                    0 => (x, y, x - msg.x, y - msg.y),
                    1 => (msg.x + x, msg.y + y, x, y),
                    _ => (msg.x, msg.y, 0, 0),
                };
                msg.x = nx;
                msg.y = ny;
                for sp in msg.local_sprites.iter_mut().flatten() {
                    sp.x += dx;
                    sp.y += dy;
                }
                for btn in &mut msg.buttons {
                    if btn.enabled {
                        btn.hit[0] += dx;
                        btn.hit[1] += dy;
                    }
                }
                msg.rebuild_cached_objects();
                self.windows[w_idx].as_mut().expect("checked").position = [nx, ny];
                let paused = self.messages[m].as_ref().expect("checked").paused;
                self.sync_win(w_idx, paused, texts, default_face);
            }
            0x527 => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                call.complete(0x4004)?;
                let active = u32::from(self.messages[m].is_some());
                call.vm
                    .state_mut()
                    .set_system_integer(0, active)
                    .expect("slot 0");
            }
            0x630 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                call.complete(0x4004)?;
                let msg = self.messages[m].as_ref().expect("checked");
                let state = call.vm.state_mut();
                state.set_system_integer(1, msg.x as u32).expect("slot 1");
                state.set_system_integer(2, msg.y as u32).expect("slot 2");
                state.set_system_integer(3, msg.w as u32).expect("slot 3");
                state.set_system_integer(4, msg.h as u32).expect("slot 4");
            }

            // Message local images / sprites / inline buttons 0x2131, 0x2134..=0x2157
            0x531 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let img = Self::check_local_image(call.pc, op, call.argument(8, "local image")?)?;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_images[img] = None;
                msg.rebuild_cached_objects();
            }
            0x534 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let img = Self::check_local_image(call.pc, op, call.argument(8, "local image")?)?;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_ref().expect("checked");
                let state = call.vm.state_mut();
                if let Some(layer) = msg.local_images[img].as_ref() {
                    state.set_system_integer(0, 1).expect("slot 0");
                    state.set_system_integer(1, layer.width()).expect("slot 1");
                    state.set_system_integer(2, layer.height()).expect("slot 2");
                } else {
                    state.set_system_integer(0, 0).expect("slot 0");
                }
            }
            0x535 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let img = Self::check_local_image(call.pc, op, call.argument(8, "local image")?)?;
                let layer_idx = call.argument(12, "layer")?;
                let src_c = call.argument(16, "src color")?;
                let dst_c = call.argument(20, "dst color")?;
                call.complete(0x4014)?;
                let msg = self.messages[m].as_mut().expect("checked");
                if (layer_idx == 0 || layer_idx >= 16)
                    && let Some(layer) = msg.local_images[img].as_mut()
                {
                    crate::scene::cvns::apply_hsl_tint(layer, src_c, dst_c);
                    msg.rebuild_cached_objects();
                }
            }
            0x538 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let sp = Self::check_local_sprite(call.pc, op, call.argument(8, "local sprite")?)?;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp] = Some(CvnsSprite {
                    visible: false,
                    ..Default::default()
                });
                msg.rebuild_cached_objects();
            }
            0x539 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let sp = Self::check_local_sprite(call.pc, op, call.argument(8, "local sprite")?)?;
                call.complete(0x4008)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp] = None;
                msg.rebuild_cached_objects();
            }
            0x53a => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let img = call.argument(12, "local image")?;
                if (img as usize) >= CVNS_LOCAL_IMAGES {
                    return Err(error(call.pc, op, "CVNS local image outside 0..5"));
                }
                let sx = call.argument(16, "sx")? as i32;
                let sy = call.argument(20, "sy")? as i32;
                let sw = call.argument(24, "sw")? as i32;
                let sh = call.argument(28, "sh")? as i32;
                let msg = self.messages[m].as_mut().expect("checked");
                let Some(layer) = msg.local_images[img as usize].as_ref() else {
                    return Err(error(call.pc, op, "CVNS local image slot is empty"));
                };
                let dims = (layer.width() as i32, layer.height() as i32);
                call.complete(0x401c)?;
                msg.local_sprites[sp]
                    .as_mut()
                    .expect("checked")
                    .set_src_rect(img as i16, sx, sy, sw, sh, dims, true, true);
                msg.rebuild_cached_objects();
            }
            0x53c => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let x = call.argument(12, "x")? as i32;
                let y = call.argument(16, "y")? as i32;
                call.complete(0x4010)?;
                let msg = self.messages[m].as_mut().expect("checked");
                let sprite = msg.local_sprites[sp].as_mut().expect("checked");
                sprite.x = x;
                sprite.y = y;
                msg.rebuild_cached_objects();
            }
            0x53d => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let z = call.argument(12, "z priority")? as i32;
                call.complete(0x400c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                if (0..128).contains(&z) {
                    msg.local_sprites[sp].as_mut().expect("checked").z = z;
                }
                msg.rebuild_cached_objects();
            }
            0x53e => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let alpha = call.argument(12, "alpha")?.min(256) as u16;
                call.complete(0x400c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp].as_mut().expect("checked").alpha = alpha;
                msg.rebuild_cached_objects();
            }
            0x540 => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let ox = call.argument(12, "origin x")? as i32;
                let oy = call.argument(16, "origin y")? as i32;
                call.complete(0x4010)?;
                let msg = self.messages[m].as_mut().expect("checked");
                let sprite = msg.local_sprites[sp].as_mut().expect("checked");
                sprite.origin_x = ox;
                sprite.origin_y = oy;
                msg.rebuild_cached_objects();
            }
            0x543 => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let visible = call.argument(12, "visible")? != 0;
                call.complete(0x400c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp].as_mut().expect("checked").visible = visible;
                msg.rebuild_cached_objects();
            }
            0x544 | 0x545 => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let frame = call.argument(12, "frame")? as usize;
                let next_frame = call.argument(16, "next frame")? as i16;
                let duration_ms = call.argument(20, "duration ms")? as i16;
                let image_slot = call.argument(24, "image slot")? as i16;
                let sx = call.argument(28, "sx")? as i16;
                let sy = call.argument(32, "sy")? as i16;
                let sw = call.argument(36, "sw")? as i16;
                let sh = call.argument(40, "sh")? as i16;
                let ox = call.argument(44, "ox")? as i16;
                let oy = call.argument(48, "oy")? as i16;
                let (playlist, stack_bytes) = if cmd == 0x544 {
                    (call.argument(52, "playlist track")? as i32, 0x4034)
                } else {
                    (-1, 0x4030)
                };
                if frame >= CVNS_ANIM_FRAMES {
                    return Err(error(call.pc, op, "CVNS animation frame outside 0..63"));
                }
                call.complete(stack_bytes)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp]
                    .as_mut()
                    .expect("checked")
                    .set_anim_frame(
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
            0x548 => {
                let (m, sp) = self.require_local_sprite_mut(
                    call.pc,
                    op,
                    call.argument(4, "message slot")?,
                    call.argument(8, "local sprite")?,
                )?;
                let img = call.argument(12, "local image")?;
                let sx = call.argument(16, "sx")? as i16;
                let sy = call.argument(20, "sy")? as i16;
                let sw = call.argument(24, "sw")? as i16;
                let sh = call.argument(28, "sh")? as i16;
                call.complete(0x401c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.local_sprites[sp]
                    .as_mut()
                    .expect("checked")
                    .set_src_rect(
                        img as i16,
                        i32::from(sx),
                        i32::from(sy),
                        i32::from(sw),
                        i32::from(sh),
                        (0, 0),
                        false,
                        true,
                    );
                msg.rebuild_cached_objects();
            }
            0x54f => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                let sp = Self::check_local_sprite(call.pc, op, call.argument(8, "local sprite")?)?;
                call.complete(0x4008)?;
                let exists = self.messages[m]
                    .as_ref()
                    .is_some_and(|msg| msg.local_sprites[sp].is_some());
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(exists))
                    .expect("slot 0");
            }
            0x550 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                let state_idx = call.argument(12, "state index")? as usize;
                let sp_slot =
                    Self::check_local_sprite(call.pc, op, call.argument(16, "local sprite")?)?;
                let img_slot = call.argument(20, "local image")? as i16;
                let sx = call.argument(24, "sx")? as i16;
                let sy = call.argument(28, "sy")? as i16;
                let sw = call.argument(32, "sw")? as i16;
                let sh = call.argument(36, "sh")? as i16;
                let dx = call.argument(40, "dx")? as i16;
                let dy = call.argument(44, "dy")? as i16;
                if state_idx >= CVNS_BUTTON_STATES {
                    return Err(error(call.pc, op, "CVNS button state outside 0..5"));
                }
                call.complete(0x402c)?;
                let btn = &mut self.messages[m].as_mut().expect("checked").buttons[btn_idx];
                btn.sprite_slot = sp_slot as u16;
                btn.states[state_idx] = CvnsButtonState {
                    image_slot: img_slot,
                    sx,
                    sy,
                    sw,
                    sh,
                    dx,
                    dy,
                };
            }
            0x551 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                let x = call.argument(12, "hit x")? as i16 as i32;
                let y = call.argument(16, "hit y")? as i16 as i32;
                let w = call.argument(20, "hit w")? as i16 as i32;
                let h = call.argument(24, "hit h")? as i16 as i32;
                call.complete(0x4018)?;
                let msg = self.messages[m].as_mut().expect("checked");
                msg.buttons[btn_idx].hit = [msg.x + x, msg.y + y, w, h];
            }
            0x554 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                call.complete(0x4008)?;
                let enabled = self.messages[m].as_ref().expect("checked").buttons[btn_idx].enabled;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(enabled))
                    .expect("slot 0");
            }
            0x555 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                let init_state = call.argument(12, "init state")? as u16;
                call.complete(0x400c)?;
                let btn = &mut self.messages[m].as_mut().expect("checked").buttons[btn_idx];
                btn.enabled = true;
                btn.prev_state = init_state;
            }
            0x556 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                let state_idx = call.argument(12, "state index")? as u16;
                call.complete(0x400c)?;
                let msg = self.messages[m].as_mut().expect("checked");
                if btn_idx < CVNS_INLINE_BUTTONS {
                    let btn = &mut msg.buttons[btn_idx];
                    btn.cur_state = state_idx;
                    if btn.prev_state != btn.cur_state
                        && let Some(st) = btn.states.get(usize::from(btn.cur_state)).copied()
                        && let Some(sp) = msg
                            .local_sprites
                            .get_mut(usize::from(btn.sprite_slot))
                            .and_then(Option::as_mut)
                    {
                        let dims = msg
                            .local_images
                            .get(st.image_slot as usize)
                            .and_then(Option::as_ref)
                            .map_or((i32::from(st.sw), i32::from(st.sh)), |l| {
                                (l.width() as i32, l.height() as i32)
                            });
                        sp.set_src_rect(
                            st.image_slot,
                            i32::from(st.sx),
                            i32::from(st.sy),
                            i32::from(st.sw),
                            i32::from(st.sh),
                            dims,
                            false,
                            true,
                        );
                    }
                    msg.buttons[btn_idx].prev_state = state_idx;
                    msg.rebuild_cached_objects();
                }
            }
            0x557 => {
                let (m, _) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let btn_idx =
                    Self::check_button_slot(call.pc, op, call.argument(8, "button slot")?)?;
                call.complete(0x4008)?;
                self.messages[m].as_mut().expect("checked").buttons[btn_idx].enabled = false;
            }
            _ => return Err(error(call.pc, op, "unhandled CVNS message command")),
        }
        Ok(Flow::Done)
    }

    pub(crate) fn handle_image(
        &mut self,
        call: &mut Call<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let op = call.command + 0x1c00;
        let (m, _) = self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
        let slot = call.argument(8, "local image")? as usize;
        if slot >= CVNS_LOCAL_IMAGES {
            return Err(error(call.pc, op, "CVNS local image outside 0..5").into());
        }
        if call.command == 0x531 {
            call.complete(0x4008)?;
            let msg = self.messages[m].as_mut().expect("checked");
            msg.local_images[slot] = None;
            msg.rebuild_cached_objects();
            return Ok(());
        }
        let name_ref = call.argument(12, "image name")?;
        let use_alpha = call.argument(16, "alpha flag")? != 0;
        let name = call.vm.resolve_string(name_ref)?;
        let decoded = image::load(source, &name)?
            .ok_or_else(|| error(call.pc, op, "failed to load CVNS message image"))?;
        call.complete(0x4010)?;
        let mut layer =
            CvnsImageLayer::from_image(name, Vec::new(), decoded.pixels, 0, 0, use_alpha);
        if !use_alpha {
            let mut px = layer.pixels.pixels().to_vec();
            for p in px.as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
            layer.pixels = crate::raster::RgbaImage::new(layer.width(), layer.height(), px)?;
        }
        let msg = self.messages[m].as_mut().expect("checked");
        msg.local_images[slot] = Some(layer);
        msg.rebuild_cached_objects();
        Ok(())
    }

    pub(crate) fn handle_pointer_or_buttons(
        &mut self,
        call: &mut Call<'_>,
        input: &mut ActionInput,
    ) -> Result<(), DecodeError> {
        let op = call.command + 0x1c00;
        let (m, _) = self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
        match call.command {
            0x553 => {
                call.complete(0x4004)?;
                let (hit_idx, clicked) = self.messages[m]
                    .as_ref()
                    .expect("checked")
                    .poll_buttons(input, true);
                let state = call.vm.state_mut();
                state.set_system_integer(4, hit_idx as u32).expect("slot 4");
                state
                    .set_system_integer(5, u32::from(clicked))
                    .expect("slot 5");
            }
            0x631 => {
                let x = call.argument(8, "x")? as i32;
                let y = call.argument(12, "y")? as i32;
                let w = call.argument(16, "w")? as i32;
                let h = call.argument(20, "h")? as i32;
                call.complete(0x4014)?;
                let msg = self.messages[m].as_ref().expect("checked");
                let [px, py] = input.pointer();
                let rx = msg.x + x;
                let ry = msg.y + y;
                let inside = px >= rx && py >= ry && px < rx + w && py < ry + h;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(inside))
                    .expect("slot 0");
            }
            _ => return Err(error(call.pc, op, "unhandled CVNS message input command")),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_wait(
        &mut self,
        call: &mut Call<'_>,
        texts: &mut [Option<TextObject>; crate::message::TEXT_OBJECTS],
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
        output_settings: TextOutputSettings,
        default_face: &[u8],
    ) -> Result<Flow, DecodeError> {
        let cmd = call.command;
        let op = cmd + 0x1c00;
        match cmd {
            0x512 | 0x610 => {
                let m = Self::check_msg_slot(call.pc, op, call.argument(4, "message slot")?)?;
                let (stack_bytes, finish_flags, msg_id, txt_ref) = if cmd == 0x512 {
                    (
                        12usize,
                        0xc00c_u16,
                        call.argument(8, "read identifier")? as i32,
                        call.argument(12, "text")?,
                    )
                } else {
                    (8usize, 0xc008_u16, -1, call.argument(8, "text")?)
                };
                let bytes = call.vm.resolve_string(txt_ref)?;
                let is_first = call.vm.repeat_count() == 0;
                let msg = self.messages[m]
                    .as_ref()
                    .ok_or_else(|| error(call.pc, op, "CVNS message slot is empty"))?;
                let w_idx = msg.win_slot;
                let paused = msg.paused;
                let _win = self
                    .windows
                    .get(w_idx)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| error(call.pc, op, "CVNS message paired window is empty"))?;

                let skip_enabled = if cmd == 0x512 && self.read_skip && msg_id >= 0 {
                    context.read_eligible && (flags.skip() || flags.cvns_read_skipping())
                } else {
                    flags.skip() || flags.cvns_read_skipping()
                };
                let skipping = skip_enabled || input.cvns_skip_held() || flags.bypass();
                if is_first {
                    self.write_text_to_window(
                        w_idx,
                        &bytes,
                        true,
                        skipping,
                        output_settings,
                        texts,
                        default_face,
                    );
                    self.sync_win(w_idx, paused, texts, default_face);
                }
                if paused {
                    return call.wait(stack_bytes);
                }
                let active = texts[w_idx]
                    .as_ref()
                    .is_some_and(|t| t.glyphs.iter().any(|g| g.active));
                let mut finish_now = !active;
                let mut consume = false;
                if active && cmd == 0x512 {
                    if skipping {
                        finish_now = true;
                    } else if input.mouse(0)[2]
                        || input.pressed(0)
                        || input.pressed(20)
                        || input.cvns_wait_interrupt((1 << 5) | (1 << 6) | 2, false)
                    {
                        finish_now = true;
                        consume = true;
                    }
                }
                if finish_now {
                    if active && let Some(text) = texts[w_idx].as_mut() {
                        for g in &mut text.glyphs {
                            g.active = false;
                            g.shown = true;
                            g.delay_ms = 0;
                            g.opacity = 255;
                            g.reveal_width = g.size + 4;
                        }
                    }
                    if consume {
                        input.consume_mouse_press(0);
                        input.consume(0);
                        input.consume(20);
                        input.cvns_wait_interrupt((1 << 5) | (1 << 6) | 2, true);
                    }
                    return call.finish(finish_flags);
                }
                call.wait(stack_bytes)
            }
            0x520 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let _msg_id = call.argument(8, "read identifier")? as i32;
                let voice = call.argument(12, "voice channel")? as i32;
                if self.messages[m].as_ref().expect("checked").paused {
                    return call.wait(12);
                }
                let skip_now = input.cvns_skip_held()
                    || flags.bypass()
                    || ((flags.skip() || flags.cvns_read_skipping()) && context.read_eligible);
                let mut done = skip_now;
                let mut consume_click = false;
                if !done {
                    let (hover, _) = self.messages[m]
                        .as_ref()
                        .expect("checked")
                        .poll_buttons(input, false);
                    if hover < 0
                        && (input.mouse(0)[2]
                            || input.pressed(0)
                            || input.pressed(20)
                            || input.cvns_wait_interrupt((1 << 5) | (1 << 6) | 2, false))
                    {
                        done = true;
                        consume_click = true;
                    }
                }
                if !done && flags.auto_advance() {
                    let glyphs = texts[w_idx].as_ref().map_or(0, |t| t.glyphs.len() as u32);
                    done =
                        auto_advance_ready(&mut self.click_wait_elapsed_ms, voice, glyphs, context);
                }
                if done {
                    self.click_wait_elapsed_ms = 0;
                    self.messages[m]
                        .as_mut()
                        .expect("checked")
                        .set_wait_cursor(false);
                    if consume_click {
                        input.consume_mouse_press(0);
                        input.consume(0);
                        input.consume(20);
                        input.cvns_wait_interrupt((1 << 5) | (1 << 6) | 2, true);
                    }
                    return call.finish(0x400c);
                }
                self.messages[m]
                    .as_mut()
                    .expect("checked")
                    .set_wait_cursor(true);
                call.wait(12)
            }
            0x611 => {
                let (m, w_idx) =
                    self.require_msg_win_mut(call.pc, op, call.argument(4, "message slot")?)?;
                let voice = call.argument(8, "voice channel")? as i32;
                if self.messages[m].as_ref().expect("checked").paused {
                    return call.wait(8);
                }
                let glyphs = texts[w_idx].as_ref().map_or(0, |t| t.glyphs.len() as u32);
                if auto_advance_ready(&mut self.auto_wait_elapsed_ms, voice, glyphs, context) {
                    self.auto_wait_elapsed_ms = 0;
                    self.messages[m]
                        .as_mut()
                        .expect("checked")
                        .set_wait_cursor(false);
                    return call.finish(0x4008);
                }
                self.messages[m]
                    .as_mut()
                    .expect("checked")
                    .set_wait_cursor(true);
                call.wait(8)
            }
            _ => Err(error(call.pc, op, "unhandled CVNS message wait command")),
        }
    }
}

fn auto_advance_ready(
    elapsed_ms: &mut u32,
    voice: i32,
    glyphs: u32,
    context: AutoAdvanceContext<'_>,
) -> bool {
    *elapsed_ms = elapsed_ms.saturating_add(context.delta_ms);
    if voice >= 0 && context.settings.sync_voice {
        let ch = ((voice as u32) & 3) as u8;
        if context.audio.voice_position(ch).ok().flatten().is_some() {
            *elapsed_ms = 0;
            false
        } else {
            *elapsed_ms
                >= context
                    .settings
                    .voice_wait
                    .saturating_mul(100)
                    .saturating_add(250)
        }
    } else {
        let target = context.settings.text_delay_ms(glyphs as usize).unwrap_or(0);
        *elapsed_ms >= target
    }
}

#[allow(clippy::too_many_arguments)]
fn push_cvns_glyph(
    win: &CvnsWindow,
    text: &mut TextObject,
    face: &Arc<[u8]>,
    code: u16,
    pos: [i32; 2],
    size: u32,
    shadow: bool,
    animated: bool,
    wc_instant: bool,
    total_delay: &mut u32,
    base_step: u32,
    pause_mult: usize,
) {
    let is_anim = animated && !wc_instant && base_step > 0;
    let delay_ms = if is_anim { *total_delay } else { 0 };
    if is_anim {
        let mut next = base_step;
        if pause_mult > 0 {
            next = next.saturating_add((pause_mult as u32) * 100);
        }
        *total_delay = total_delay.saturating_add(next);
    }
    let outline = if shadow { [2, 0] } else { [0, 0] };
    let reveal_mode = if is_anim { win.reveal[0] as u32 } else { 0 };
    let duration_ms = if is_anim {
        win.reveal[1].max(0) as u32
    } else {
        0
    };
    let shown = !is_anim || delay_ms == 0;
    let opacity = if !is_anim || reveal_mode != 1 { 255 } else { 0 };
    let reveal_width = if !is_anim || reveal_mode != 2 {
        size + 4
    } else {
        0
    };
    text.glyphs.push(RecordedGlyph {
        snapshot_indices: None,
        face: face.clone(),
        bold: false,
        revision_alpha: false,
        code,
        position: pos,
        size,
        colors: win.colors,
        gradient: win.gradient_dir.min(3),
        outline,
        delay_ms,
        elapsed_ms: 0,
        duration_ms,
        active: is_anim,
        shown,
        opacity,
        reveal_mode,
        reveal_width,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::{AudioBackend, AudioRequest},
        evaluate::State,
        image::DecodeThreads,
        input::{CodeInput, KeyBindings},
        message::MessageHost,
        ps3::Script,
        scene::{
            SceneHost,
            cvns::{CvnsImageBuffer, CvnsSpriteKind},
        },
        scene_render,
        settings::AutoAdvanceSettings,
        viewport::ScreenSize,
    };
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    enum Arg<'a> {
        Int(u32),
        Str(&'a [u8]),
    }

    struct MemImages(HashMap<Vec<u8>, Vec<u8>>);

    impl ImageSource for MemImages {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
            Ok(self.0.get(name).cloned())
        }
        fn decode_threads(&self) -> DecodeThreads {
            DecodeThreads::default()
        }
    }

    struct MockVoiceAudio {
        pos_ms: Option<u32>,
    }

    impl AudioBackend for MockVoiceAudio {
        fn play(&mut self, _request: &AudioRequest) -> Result<bool, Box<dyn Error + Send + Sync>> {
            Ok(true)
        }
        fn voice_position(
            &self,
            _channel: u8,
        ) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
            Ok(self.pos_ms)
        }
    }

    fn new_cvns_msg_host(w: u32, h: u32) -> MessageHost {
        let mut host = MessageHost::snapshot_preview(
            std::array::from_fn(|_| None),
            std::array::from_fn(|_| None),
        );
        host.enable_cvns();
        host.set_screen_size(ScreenSize::new([w, h]).unwrap());
        host
    }

    fn pb2_type1_rgba(w: u16, h: u16, rgba: [u8; 4]) -> Vec<u8> {
        assert!(w <= 8 && h <= 8);
        let n = usize::from(w) * usize::from(h);
        let mut planes = Vec::with_capacity(n * 4);
        for lane in [rgba[2], rgba[1], rgba[0], rgba[3]] {
            planes.extend(std::iter::repeat_n(lane, n));
        }
        let flag_len = planes.len().div_ceil(8);
        let flags = vec![0x00u8; flag_len];
        let flags_off = 32u32;
        let data_off = flags_off + flags.len() as u32;
        let tail = [0u8; 31];
        let total = 32 + flags.len() + planes.len() + tail.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"PB2A");
        out.extend((total as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(w.to_le_bytes());
        out.extend(h.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend(flags_off.to_le_bytes());
        out.extend(data_off.to_le_bytes());
        out.extend(&flags);
        out.extend(&planes);
        out.extend(&tail);
        out
    }

    fn build_sys_script(op: u16, args_sp_first: &[Arg<'_>]) -> Script {
        let mut code = Vec::new();
        let mut text = Vec::new();
        for arg in args_sp_first.iter().rev() {
            let val = match *arg {
                Arg::Int(v) => v,
                Arg::Str(bytes) => {
                    let off = text.len() as u32;
                    text.extend_from_slice(bytes);
                    text.push(0);
                    off
                }
            };
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(val.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
        code.extend(0x2000u16.to_le_bytes());
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(&code);
        payload.extend(&text);
        let mut raw = b"PSS0".to_vec();
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
        raw.extend(&payload);
        Script::decode(&raw).unwrap()
    }

    fn exec_msg(host: &mut MessageHost, cvns_op: u16, args: &[Arg<'_>]) -> State {
        let script = build_sys_script(cvns_op + 0x1c00, args);
        let mut vm = crate::machine::Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        let cmd = cvns_op;
        let mut call = Call::new(&mut vm, pc, cmd);
        let flow = host.handle_cvns(&mut call).unwrap();
        assert_eq!(flow, Flow::Done);
        vm.state().clone()
    }

    fn exec_img(
        host: &mut MessageHost,
        images: &mut dyn ImageSource,
        cvns_op: u16,
        args: &[Arg<'_>],
    ) {
        let script = build_sys_script(cvns_op + 0x1c00, args);
        let mut vm = crate::machine::Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        let mut call = Call::new(&mut vm, pc, cvns_op);
        host.handle_cvns_image(&mut call, images).unwrap();
    }

    fn exec_ptr(
        host: &mut MessageHost,
        input: &mut ActionInput,
        cvns_op: u16,
        args: &[Arg<'_>],
    ) -> State {
        let script = build_sys_script(cvns_op + 0x1c00, args);
        let mut vm = crate::machine::Machine::new(&script).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        let mut call = Call::new(&mut vm, pc, cvns_op);
        host.handle_cvns_input(&mut call, input).unwrap();
        vm.state().clone()
    }

    #[test]
    fn cvns_windows_layout_ruby_escapes_and_kinsoku() {
        let mut host = new_cvns_msg_host(800, 600);

        exec_msg(&mut host, 0x4d0, &[Arg::Int(0)]);
        assert_eq!(host.text(0).unwrap().logical_size, [800, 600]);
        assert_eq!(host.text(0).unwrap().scale_bits, 1.0f32.to_bits());

        // Area width = 48 + 4 + 24 => limit = 10 + 76 - (4 + 24) = 58.
        // Glyph at x=10 -> 38 -> 66 (>= 58!).
        exec_msg(&mut host, 0x4d2, &[Arg::Int(0), Arg::Int(20), Arg::Int(30)]);
        exec_msg(
            &mut host,
            0x4d3,
            &[Arg::Int(0), Arg::Int(200), Arg::Int(120)],
        );
        exec_msg(
            &mut host,
            0x4d4,
            &[
                Arg::Int(0),
                Arg::Int(10),
                Arg::Int(12),
                Arg::Int(76),
                Arg::Int(100),
            ],
        );
        exec_msg(
            &mut host,
            0x4d6,
            &[Arg::Int(0), Arg::Int(24), Arg::Int(4), Arg::Int(6)],
        );
        exec_msg(&mut host, 0x4d7, &[Arg::Int(0), Arg::Str(b"MS Gothic")]);
        exec_msg(
            &mut host,
            0x4d8,
            &[Arg::Int(0), Arg::Int(0x0011_2233), Arg::Int(0x0044_5566)],
        );
        exec_msg(&mut host, 0x4d9, &[Arg::Int(0), Arg::Int(1)]); // kinsoku on
        exec_msg(&mut host, 0x4dc, &[Arg::Int(0), Arg::Int(1), Arg::Int(250)]);
        exec_msg(&mut host, 0x4dd, &[Arg::Int(0), Arg::Int(25)]);
        exec_msg(&mut host, 0x500, &[Arg::Int(0), Arg::Int(2)]); // gradient mode
        exec_msg(&mut host, 0x501, &[Arg::Int(0), Arg::Int(5)]); // z=5
        exec_msg(&mut host, 0x608, &[Arg::Int(0), Arg::Int(1)]); // ruby enabled
        exec_msg(&mut host, 0x609, &[Arg::Int(0), Arg::Int(10)]); // ruby size
        exec_msg(&mut host, 0x60a, &[Arg::Int(0), Arg::Int(2)]); // ruby spacing
        exec_msg(&mut host, 0x60b, &[Arg::Int(0), Arg::Int(0)]);

        // Shift-JIS: `ああ、` (0x82a0 0x82a0 0x8141) with kinsoku=1 stays on line 0 (`、` doesn't wrap!),
        // then `\n` moves to line 1 with `{漢/かん}` ruby.
        let line_sample = b"\x82\xa0\x82\xa0\x81\x41\\n{\x8a\xbf/\x82\xa9\x82\xf1}".to_vec();
        let st = exec_msg(&mut host, 0x60f, &[Arg::Int(0), Arg::Str(&line_sample)]);
        assert_eq!(st.system_integer(0), Some(2));

        // Turn kinsoku off -> `、` wraps onto a 3rd line!
        exec_msg(&mut host, 0x4d9, &[Arg::Int(0), Arg::Int(0)]);
        let st2 = exec_msg(&mut host, 0x60f, &[Arg::Int(0), Arg::Str(&line_sample)]);
        assert_eq!(st2.system_integer(0), Some(3));
        exec_msg(&mut host, 0x4d9, &[Arg::Int(0), Arg::Int(1)]);

        // Draw immediate text starting with `Z\c` (to verify `\c` clears), `\wc`, `\ws`, ruby & kinsoku.
        let mut draw_bytes = b"Z\\c".to_vec();
        draw_bytes.extend_from_slice(&line_sample);
        draw_bytes.extend_from_slice(b"\\wcA\\wsB");
        exec_msg(&mut host, 0x4df, &[Arg::Int(0), Arg::Str(&draw_bytes)]);
        let text = host.text(0).unwrap();
        assert_eq!(text.layer, 5);
        assert_eq!(text.decoration, 2);
        // Line 0 chars `あ`,`あ`,`、` share y=12 (no ruby on line 0 when ruby_always=false),
        // Line 1 has base `漢` at y = 12 + (24+6) + 10(ruby) = 52 and ruby `か`,`ん` at y = 42.
        assert_eq!(text.glyphs[0].code, 0x82a0);
        assert_eq!(text.glyphs[0].position[1], 12);
        assert_eq!(text.glyphs[2].code, 0x8141);
        assert_eq!(text.glyphs[2].position[1], 12);
        assert_eq!(text.glyphs[3].code, 0x8abf); // 漢
        assert_eq!(text.glyphs[3].position[1], 52);
        assert_eq!(text.glyphs[4].code, 0x82a9); // か (ruby)
        assert_eq!(text.glyphs[4].position[1], 42);
        assert_eq!(text.glyphs[4].size, 10);

        exec_msg(&mut host, 0x4de, &[Arg::Int(0), Arg::Int(15), Arg::Int(20)]);
        assert_eq!(host.text(0).unwrap().cursor, [15, 20]);
        exec_msg(&mut host, 0x4d5, &[Arg::Int(0)]);
        assert!(host.text(0).unwrap().glyphs.is_empty());
        exec_msg(&mut host, 0x4d1, &[Arg::Int(0)]);
        assert!(host.text(0).is_none());
    }

    #[test]
    fn cvns_messages_local_sprites_inline_buttons_and_bounds() {
        let mut map = HashMap::new();
        map.insert(b"ui".to_vec(), pb2_type1_rgba(4, 4, [20, 40, 200, 255]));
        let mut images = MemImages(map);

        let mut host = new_cvns_msg_host(800, 600);

        exec_msg(
            &mut host,
            0x510,
            &[
                Arg::Int(0),
                Arg::Int(1),
                Arg::Int(100),
                Arg::Int(400),
                Arg::Int(600),
                Arg::Int(150),
            ],
        );
        assert_eq!(
            exec_msg(&mut host, 0x527, &[Arg::Int(0)]).system_integer(0),
            Some(1)
        );
        assert!(!host.window_visible(0));
        assert!(!host.window_visible(1));
        assert_eq!(
            exec_msg(&mut host, 0x51f, &[Arg::Int(0)]).system_integer(0),
            Some(0)
        );

        let bounds = exec_msg(&mut host, 0x630, &[Arg::Int(0)]);
        assert_eq!(bounds.system_integer(1), Some(100));
        assert_eq!(bounds.system_integer(2), Some(400));
        assert_eq!(bounds.system_integer(3), Some(600));
        assert_eq!(bounds.system_integer(4), Some(150));

        // Load local image 0 & bind local sprites 0 (bg), 1 (button), 2 (attached cursor).
        exec_img(
            &mut host,
            &mut images,
            0x530,
            &[Arg::Int(0), Arg::Int(0), Arg::Str(b"ui"), Arg::Int(1)],
        );
        exec_msg(&mut host, 0x538, &[Arg::Int(0), Arg::Int(0)]);
        exec_msg(
            &mut host,
            0x53a,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(4),
                Arg::Int(4),
            ],
        );
        exec_msg(&mut host, 0x53d, &[Arg::Int(0), Arg::Int(0), Arg::Int(10)]);
        exec_msg(&mut host, 0x538, &[Arg::Int(0), Arg::Int(1)]);
        exec_msg(&mut host, 0x53d, &[Arg::Int(0), Arg::Int(1), Arg::Int(5)]);
        exec_msg(
            &mut host,
            0x540,
            &[Arg::Int(0), Arg::Int(1), Arg::Int(1), Arg::Int(2)],
        );
        assert_eq!(
            exec_msg(&mut host, 0x54f, &[Arg::Int(0), Arg::Int(1)]).system_integer(0),
            Some(1)
        );

        // Inline button 2 -> local sprite 1, state 0 normal / state 1 hover / state 2 active.
        exec_msg(
            &mut host,
            0x550,
            &[
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(0),
                Arg::Int(1),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(4),
                Arg::Int(4),
                Arg::Int(10),
                Arg::Int(20),
            ],
        );
        exec_msg(
            &mut host,
            0x550,
            &[
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(1),
                Arg::Int(1),
                Arg::Int(0),
                Arg::Int(1),
                Arg::Int(1),
                Arg::Int(3),
                Arg::Int(3),
                Arg::Int(12),
                Arg::Int(22),
            ],
        );
        exec_msg(
            &mut host,
            0x550,
            &[
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(2),
                Arg::Int(1),
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(2),
                Arg::Int(2),
                Arg::Int(2),
                Arg::Int(14),
                Arg::Int(24),
            ],
        );
        exec_msg(
            &mut host,
            0x551,
            &[
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(50),
                Arg::Int(60),
                Arg::Int(40),
                Arg::Int(20),
            ],
        );
        exec_msg(&mut host, 0x555, &[Arg::Int(0), Arg::Int(2), Arg::Int(0)]);
        exec_msg(&mut host, 0x511, &[Arg::Int(0), Arg::Int(1)]);
        assert!(host.window_visible(0));
        assert!(!host.window_visible(1));
        assert_eq!(
            exec_msg(&mut host, 0x554, &[Arg::Int(0), Arg::Int(2)]).system_integer(0),
            Some(1)
        );

        // Pointer inside screen hit rect (100+50..140, 400+60..420) = (160, 465).
        let mut input = ActionInput::default();
        input.set_pointer(160, 465);
        assert_eq!(
            exec_ptr(
                &mut host,
                &mut input,
                0x631,
                &[
                    Arg::Int(0),
                    Arg::Int(50),
                    Arg::Int(60),
                    Arg::Int(40),
                    Arg::Int(20)
                ]
            )
            .system_integer(0),
            Some(1)
        );
        exec_msg(&mut host, 0x556, &[Arg::Int(0), Arg::Int(2), Arg::Int(1)]);
        assert_eq!(
            host.cvns().unwrap().messages[0]
                .as_ref()
                .unwrap()
                .local_sprites[1]
                .as_ref()
                .unwrap()
                .src_rect,
            [1, 1, 4, 4]
        );
        let hover = exec_ptr(&mut host, &mut input, 0x553, &[Arg::Int(0)]);
        assert_eq!(hover.system_integer(4), Some(2));
        assert_eq!(hover.system_integer(5), Some(0));

        // Press left button -> poll returns hit button 2 and clicked=1 in slots 4/5!
        let mut now = [false; 128];
        now[1] = true; // VK_LBUTTON in MOUSE_CODES
        let bindings = KeyBindings::default();
        let mut tracker = CodeInput::default();
        tracker.update(&bindings, &now, &mut input);
        let click = exec_ptr(&mut host, &mut input, 0x553, &[Arg::Int(0)]);
        assert_eq!(click.system_integer(4), Some(2));
        assert_eq!(click.system_integer(5), Some(1));

        // Attach local sprite 2 (set offset first, then attach), pause, move box, and verify pause & destroy.
        exec_msg(&mut host, 0x538, &[Arg::Int(0), Arg::Int(2)]);
        exec_msg(
            &mut host,
            0x516,
            &[Arg::Int(0), Arg::Int(2), Arg::Int(15), Arg::Int(25)],
        );
        exec_msg(&mut host, 0x515, &[Arg::Int(0), Arg::Int(2), Arg::Int(1)]);
        exec_msg(
            &mut host,
            0x526,
            &[Arg::Int(0), Arg::Int(0), Arg::Int(200), Arg::Int(450)],
        );
        assert_eq!(host.text(1).unwrap().position, [200, 450]);
        assert_eq!(
            host.cvns().unwrap().messages[0]
                .as_ref()
                .unwrap()
                .local_sprites[2]
                .as_ref()
                .unwrap()
                .x,
            215
        );

        exec_msg(&mut host, 0x51b, &[Arg::Int(0), Arg::Int(1)]);
        assert_eq!(
            exec_msg(&mut host, 0x51f, &[Arg::Int(0)]).system_integer(0),
            Some(1)
        );
        assert!(!host.text(1).unwrap().visible);

        exec_msg(&mut host, 0x557, &[Arg::Int(0), Arg::Int(2)]);
        assert_eq!(
            exec_msg(&mut host, 0x554, &[Arg::Int(0), Arg::Int(2)]).system_integer(0),
            Some(0)
        );
        exec_img(&mut host, &mut images, 0x531, &[Arg::Int(0), Arg::Int(0)]);
        exec_msg(&mut host, 0x513, &[Arg::Int(0)]);
        assert!(host.text(1).is_none());
        assert_eq!(
            exec_msg(&mut host, 0x527, &[Arg::Int(0)]).system_integer(0),
            Some(0)
        );
    }

    #[test]
    fn cvns_message_reveal_click_and_voice_wait_completion() {
        let mut host = new_cvns_msg_host(800, 600);
        host.set_text_settings(TextOutputSettings {
            speed: 2,
            ..TextOutputSettings::default()
        });
        exec_msg(
            &mut host,
            0x510,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(400),
                Arg::Int(200),
            ],
        );

        // 0x512 (`0x2112`): slot 0, read_id 10, string "AB" -> reveals over 2 glyphs * 30ms.
        let s512 = build_sys_script(0x2112, &[Arg::Int(0), Arg::Int(10), Arg::Str(b"AB")]);
        let mut vm = crate::machine::Machine::new(&s512).unwrap();
        while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc = vm.pc();
        let mut input = ActionInput::default();
        let flags = MessageFlags::default();
        let mut audio = MockVoiceAudio { pos_ms: None };
        let ctx = AutoAdvanceContext {
            settings: AutoAdvanceSettings::default(),
            delta_ms: 0,
            gesture_result: false,
            read_eligible: false,
            audio: &audio,
        };
        assert_eq!(
            host.handle_cvns_wait(&mut Call::new(&mut vm, pc, 0x512), &mut input, flags, ctx)
                .unwrap(),
            Flow::Wait
        );
        // Glyph 0 shown at 0ms, glyph 1 hidden until 30ms.
        assert_eq!(host.text(0).unwrap().glyphs.len(), 2);
        assert!(host.text(0).unwrap().glyphs[0].shown);
        assert!(!host.text(0).unwrap().glyphs[1].shown);

        host.advance_frame(35);
        assert!(host.text(0).unwrap().glyphs[1].shown);
        assert_eq!(
            host.handle_cvns_wait(&mut Call::new(&mut vm, pc, 0x512), &mut input, flags, ctx)
                .unwrap(),
            Flow::Yield
        );

        // 0x611 (`0x2211`): auto/voice wait stays Wait while voice is playing, completes 250ms after voice ends.
        let s611 = build_sys_script(0x2211, &[Arg::Int(0), Arg::Int(3)]);
        let mut vm2 = crate::machine::Machine::new(&s611).unwrap();
        while !matches!(vm2.step().unwrap(), crate::machine::Step::System { .. }) {}
        let pc2 = vm2.pc();
        audio.pos_ms = Some(120);
        assert_eq!(
            host.handle_cvns_wait(
                &mut Call::new(&mut vm2, pc2, 0x611),
                &mut input,
                flags,
                AutoAdvanceContext {
                    settings: AutoAdvanceSettings::default(),
                    delta_ms: 200,
                    gesture_result: false,
                    read_eligible: false,
                    audio: &audio,
                }
            )
            .unwrap(),
            Flow::Wait
        );
        audio.pos_ms = None;
        assert_eq!(
            host.handle_cvns_wait(
                &mut Call::new(&mut vm2, pc2, 0x611),
                &mut input,
                flags,
                AutoAdvanceContext {
                    settings: AutoAdvanceSettings::default(),
                    delta_ms: 150,
                    gesture_result: false,
                    read_eligible: false,
                    audio: &audio,
                }
            )
            .unwrap(),
            Flow::Wait
        );
        assert_eq!(
            host.handle_cvns_wait(
                &mut Call::new(&mut vm2, pc2, 0x611),
                &mut input,
                flags,
                AutoAdvanceContext {
                    settings: AutoAdvanceSettings::default(),
                    delta_ms: 120,
                    gesture_result: false,
                    read_eligible: false,
                    audio: &audio,
                }
            )
            .unwrap(),
            Flow::Done
        );
    }

    #[test]
    fn cvns_compositor_orders_glyphs_local_sprites_and_global_sprites() {
        let mut map = HashMap::new();
        map.insert(b"local".to_vec(), pb2_type1_rgba(4, 4, [0, 220, 0, 255])); // green
        map.insert(b"global".to_vec(), pb2_type1_rgba(4, 4, [220, 0, 0, 255])); // red
        let mut images = MemImages(map);

        let mut scene = SceneHost::default();
        let mut msg = new_cvns_msg_host(8, 8);

        // Local sprite 0 at z=2 (green), glyph proxy at z=1 (blue), global sprite 0 at z=1 (red).
        exec_msg(
            &mut msg,
            0x510,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(8),
                Arg::Int(8),
            ],
        );
        exec_img(
            &mut msg,
            &mut images,
            0x530,
            &[Arg::Int(0), Arg::Int(0), Arg::Str(b"local"), Arg::Int(1)],
        );
        exec_msg(&mut msg, 0x538, &[Arg::Int(0), Arg::Int(0)]);
        exec_msg(
            &mut msg,
            0x53a,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(4),
                Arg::Int(4),
            ],
        );
        exec_msg(&mut msg, 0x53d, &[Arg::Int(0), Arg::Int(0), Arg::Int(2)]);
        exec_msg(&mut msg, 0x511, &[Arg::Int(0), Arg::Int(1)]);

        // Global sprite 0 at (2,0) with z=3 (behind z=2 green!) and global sprite 1 at (3,0) with z=1 (in front of z=1 glyph!).
        let gimg = crate::image::load(&mut images, b"global").unwrap().unwrap();
        let cvns = scene.cvns_mut();
        let mut sp0 = CvnsSprite {
            visible: true,
            z: 3,
            x: 2,
            kind: CvnsSpriteKind::Image,
            image_slot: 0,
            ..Default::default()
        };
        sp0.set_src_rect(0, 0, 0, 2, 2, (2, 2), false, true);
        let mut sp1 = sp0.clone();
        sp1.z = 1;
        sp1.x = 3;
        let mut layers = vec![None; crate::scene::cvns::CVNS_LAYERS];
        layers[0] = Some(CvnsImageLayer::from_image(
            b"global".to_vec(),
            Vec::new(),
            gimg.pixels,
            0,
            0,
            true,
        ));
        cvns.images[0] = Some(CvnsImageBuffer {
            layers,
            composited: None,
            mask: None,
        });
        cvns.sprites[0] = Some(sp0);
        cvns.sprites[1] = Some(sp1);

        let glyph_px = crate::raster::RgbaImage::new(4, 2, [0, 0, 220, 255].repeat(8)).unwrap();
        let mut glyph_obj = SceneObject {
            shown: true,
            image: Some(ObjectImage {
                filter: None,
                name: b"glyph".to_vec(),
                pixels: glyph_px,
                alpha: true,
                surface: false,
            }),
            ..Default::default()
        };
        let mut draw = DrawState::default();
        draw.words[4] = 4;
        draw.words[5] = 2;
        draw.words[8] = 1; // x=1
        draw.words[9] = 0; // y=0
        draw.words[10] = 1; // z=1 (same z as global sp1)
        draw.words[17] = 256;
        glyph_obj.draw = Some(draw);

        let mut additional: Vec<&SceneObject> = msg.visible_window_objects().collect();
        additional.push(&glyph_obj);
        let frame = scene_render::render_scene(&scene, None, &additional, 8, 8).unwrap();
        let px = |x: usize| &frame.pixels()[(x) * 4..(x) * 4 + 4];
        // x=0: local z=2 green overwrote global z=3 red!
        assert_eq!(px(0), &[0, 220, 0, 255]);
        // x=1..2: z=1 blue glyph overwrote z=2 green!
        assert_eq!(px(1), &[0, 0, 220, 255]);
        // x=3..4: z=1 red global sprite overwrote z=1 blue glyph!
        assert_eq!(px(3), &[220, 0, 0, 255]);

        // Font dialog / refresh_default_fonts updates CVNS windows using default face and keeps custom faces.
        msg.set_default_face(b"OldDefault");
        exec_msg(&mut msg, 0x4d0, &[Arg::Int(0)]);
        exec_msg(&mut msg, 0x4d0, &[Arg::Int(1)]);
        exec_msg(&mut msg, 0x4d7, &[Arg::Int(1), Arg::Str(b"CustomFace")]);
        assert_eq!(msg.text(0).unwrap().face.as_ref(), b"OldDefault");
        assert_eq!(msg.text(1).unwrap().face.as_ref(), b"CustomFace");
        msg.set_default_face(b"NewDefault");
        msg.refresh_default_fonts();
        assert_eq!(msg.text(0).unwrap().face.as_ref(), b"NewDefault");
        assert_eq!(msg.text(1).unwrap().face.as_ref(), b"CustomFace");
    }

    #[test]
    fn cvns_local_message_size_hsl_alpha_and_raw_source() {
        let mut map = HashMap::new();
        map.insert(b"btn".to_vec(), pb2_type1_rgba(6, 4, [200, 40, 40, 255]));
        let mut images = MemImages(map);
        let mut msg = new_cvns_msg_host(64, 64);

        exec_msg(
            &mut msg,
            0x510,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(64),
                Arg::Int(64),
            ],
        );
        // Empty local image slot queries as 0 without changing size slots.
        let vm_empty = exec_msg(&mut msg, 0x534, &[Arg::Int(0), Arg::Int(1)]);
        assert_eq!(vm_empty.system_integer(0), Some(0));

        // Populate slot 0, query size via 0x534 -> (1, 6, 4).
        exec_img(
            &mut msg,
            &mut images,
            0x530,
            &[Arg::Int(0), Arg::Int(0), Arg::Str(b"btn"), Arg::Int(1)],
        );
        let vm_size = exec_msg(&mut msg, 0x534, &[Arg::Int(0), Arg::Int(0)]);
        assert_eq!(vm_size.system_integer(0), Some(1));
        assert_eq!(vm_size.system_integer(1), Some(6));
        assert_eq!(vm_size.system_integer(2), Some(4));

        // 0x535 HSL tint shifts red -> blue/green cleanly and records hsl_from/hsl_to.
        exec_msg(
            &mut msg,
            0x535,
            &[
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0),
                Arg::Int(0x00c8_2828),
                Arg::Int(0x0028_c828),
            ],
        );
        let tinted = msg.cvns().unwrap().messages[0]
            .as_ref()
            .unwrap()
            .local_images[0]
            .as_ref()
            .unwrap();
        assert_eq!(tinted.hsl_from, 0x00c8_2828);
        assert_eq!(tinted.hsl_to, 0x0028_c828);
        let px = &tinted.pixels.pixels()[..4];
        assert!(px[2] > px[0] && px[2] > px[1]);

        // 0x538 create sprite, 0x53e alpha clamp (>256 -> 256), 0x548 raw src_rect without clamping negatives.
        exec_msg(&mut msg, 0x538, &[Arg::Int(0), Arg::Int(2)]);
        exec_msg(&mut msg, 0x53e, &[Arg::Int(0), Arg::Int(2), Arg::Int(400)]);
        assert_eq!(
            msg.cvns().unwrap().messages[0]
                .as_ref()
                .unwrap()
                .local_sprites[2]
                .as_ref()
                .unwrap()
                .alpha,
            256
        );
        exec_msg(&mut msg, 0x53e, &[Arg::Int(0), Arg::Int(2), Arg::Int(120)]);
        exec_msg(
            &mut msg,
            0x548,
            &[
                Arg::Int(0),
                Arg::Int(2),
                Arg::Int(0),
                Arg::Int((-2i32) as u32),
                Arg::Int(1),
                Arg::Int(5),
                Arg::Int(3),
            ],
        );
        let sp = msg.cvns().unwrap().messages[0]
            .as_ref()
            .unwrap()
            .local_sprites[2]
            .as_ref()
            .unwrap();
        assert_eq!(sp.alpha, 120);
        assert_eq!(sp.src_rect, [-2, 1, 3, 4]);
        assert_eq!((sp.w, sp.h), (5, 3));
        assert!(sp.dirty);

        // 0x543 toggles local sprite visibility (`visible != 0`).
        exec_msg(&mut msg, 0x543, &[Arg::Int(0), Arg::Int(2), Arg::Int(1)]);
        assert!(
            msg.cvns().unwrap().messages[0]
                .as_ref()
                .unwrap()
                .local_sprites[2]
                .as_ref()
                .unwrap()
                .visible
        );
        exec_msg(&mut msg, 0x543, &[Arg::Int(0), Arg::Int(2), Arg::Int(0)]);
        assert!(
            !msg.cvns().unwrap().messages[0]
                .as_ref()
                .unwrap()
                .local_sprites[2]
                .as_ref()
                .unwrap()
                .visible
        );

        // 0x600 loads an MSK0 glyph sheet; 0x601 extracts tile_index for a Shift-JIS code.
        let mut msk = b"MSK0".to_vec();
        msk.extend(16u32.to_le_bytes());
        msk.extend(4u32.to_le_bytes());
        msk.extend(4u32.to_le_bytes());
        // Two rows of 2x2 tiles across width 4; tile 1 sits at columns 2..4, rows 0..2.
        msk.extend([
            1, 2, 10, 11, // row 0
            3, 4, 12, 13, // row 1
            5, 6, 14, 15, // row 2
            7, 8, 16, 17, // row 3
        ]);
        images.0.insert(b"font2ex.msk".to_vec(), msk);
        exec_img(
            &mut msg,
            &mut images,
            0x600,
            &[Arg::Str(b"font2ex.msk"), Arg::Int(2)],
        );
        exec_msg(&mut msg, 0x601, &[Arg::Int(1), Arg::Int(0x8197)]);
        assert_eq!(
            msg.external_glyphs
                .glyph(0x8197, 2)
                .map(GlyphMask::coverage),
            Some([10, 11, 12, 13].as_slice())
        );
        for index in [4, 5, u32::MAX] {
            exec_msg(&mut msg, 0x601, &[Arg::Int(index), Arg::Int(0x8198)]);
            assert!(msg.external_glyphs.glyph(0x8198, 2).is_none());
        }
        exec_msg(&mut msg, 0x601, &[Arg::Int(3), Arg::Int(0x8198)]);
        assert_eq!(
            msg.external_glyphs
                .glyph(0x8198, 2)
                .map(GlyphMask::coverage),
            Some([14, 15, 16, 17].as_slice())
        );
    }
}

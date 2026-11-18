//! Engine snapshot serialization for CVNS (`cvns-20090203` / `CSP1`).

pub(crate) mod native;

/// Explicit schema version stored at the start of the `0xa00` CVNS snapshot header record.
/// Increment whenever `0xa00..=0xa09` record layouts change.
pub const CVNS_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
use super::{
    SnapshotLayout, SnapshotRecord,
    engine::Writer,
    parse_records, unique_body,
    visual::{Reader, text},
};
use crate::{
    FormatError,
    evaluate::State,
    image::{self, ImageSource},
    lzss,
    message::{
        TextObject,
        cvns::{
            CVNS_INLINE_BUTTONS, CVNS_LOCAL_IMAGES, CVNS_LOCAL_SPRITES, CVNS_MESSAGES,
            CVNS_WINDOWS, CvnsButtonState, CvnsInlineButton, CvnsMessage, CvnsMessageHost,
            CvnsWindow,
        },
    },
    raster::RgbaImage,
    scene::cvns::{
        CVNS_ANIM_FRAMES, CVNS_IMAGES, CVNS_LAYERS, CVNS_PLAYLISTS, CVNS_SPRITES, CvnsAnimFrame,
        CvnsCurtain, CvnsCurtainTile, CvnsFade, CvnsImageBuffer, CvnsImageLayer, CvnsMaskLayer,
        CvnsMaskTransition, CvnsMoveTween, CvnsQuake, CvnsScene, CvnsSprite, CvnsSpriteAnim,
        CvnsSpriteKind,
    },
    system_save::PersistentStarts,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
};

const MAX_DIM: u32 = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvnsLocalVariables {
    pub starts: PersistentStarts,
    pub integers: Vec<i32>,
    pub bits: Vec<u8>,
    pub strings: Vec<Vec<u8>>,
}

impl CvnsLocalVariables {
    pub fn capture(state: &State, starts: PersistentStarts) -> Result<Self, FormatError> {
        starts.checked()?;
        let mut integers = Vec::with_capacity(starts.integer as usize);
        for i in 0..starts.integer {
            integers.push(
                state
                    .integer(i)
                    .ok_or(FormatError("CVNS snapshot reads unknown local integer"))?,
            );
        }
        let bit_bytes = (starts.bit >> 3) as usize;
        let mut bits = vec![0u8; bit_bytes];
        for i in 0..bit_bytes * 8 {
            let v = state
                .bit(i as u32)
                .ok_or(FormatError("CVNS snapshot reads unknown local bit"))?;
            bits[i / 8] |= u8::from(v) << (i % 8);
        }
        if !bits.is_empty() {
            bits[0] |= 1;
        }
        let mut strings = Vec::with_capacity(starts.string as usize);
        for i in 0..starts.string {
            let s = state
                .string(i)
                .ok_or(FormatError("CVNS snapshot reads unknown local string"))?;
            if s.len() >= 1024 || s.contains(&0) {
                return Err(FormatError("CVNS snapshot local string invalid"));
            }
            strings.push(s.to_vec());
        }
        Ok(Self {
            starts,
            integers,
            bits,
            strings,
        })
    }

    pub fn apply_to_state(
        &self,
        state: &mut State,
        expected: PersistentStarts,
    ) -> Result<(), FormatError> {
        if self.starts != expected {
            return Err(FormatError(
                "CVNS snapshot persistent starts disagree with startup script",
            ));
        }
        for (i, &v) in self.integers.iter().enumerate() {
            state.set_integer(i as u32, v).map_err(FormatError)?;
        }
        for (byte_idx, &byte) in self.bits.iter().enumerate() {
            for bit in 0..8 {
                state
                    .set_bit((byte_idx * 8 + bit) as u32, byte & (1 << bit) != 0)
                    .map_err(FormatError)?;
            }
        }
        for (i, s) in self.strings.iter().enumerate() {
            state.set_string(i as u32, s).map_err(FormatError)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum SavedLayerPixels {
    BlankFill {
        width: u32,
        height: u32,
        color: u32,
    },
    Asset {
        name: Vec<u8>,
        part: Vec<u8>,
        frame: Option<u32>,
    },
    RawLzss {
        width: u32,
        height: u32,
        packed: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct SavedCvnsLayer {
    pub source: SavedLayerPixels,
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

impl SavedCvnsLayer {
    pub fn from_layer(layer: &CvnsImageLayer) -> Result<Self, FormatError> {
        let source = if let Some(color) = layer.fill_color {
            SavedLayerPixels::BlankFill {
                width: layer.width(),
                height: layer.height(),
                color,
            }
        } else if !layer.captured && !layer.name.is_empty() {
            SavedLayerPixels::Asset {
                name: layer.name.clone(),
                part: layer.part.clone(),
                frame: layer.frame,
            }
        } else {
            SavedLayerPixels::RawLzss {
                width: layer.width(),
                height: layer.height(),
                packed: lzss::encode(layer.pixels.pixels()),
            }
        };
        Ok(Self {
            source,
            use_alpha: layer.use_alpha,
            dx: layer.dx,
            dy: layer.dy,
            filter_mode: layer.filter_mode,
            filter_arg: layer.filter_arg,
            monochrome: layer.monochrome,
            hsl_from: layer.hsl_from,
            hsl_to: layer.hsl_to,
            rgb_mul: layer.rgb_mul,
        })
    }

    pub fn load(
        &self,
        images: &mut dyn ImageSource,
        budget: &mut usize,
        force_opaque: bool,
    ) -> Result<CvnsImageLayer, Box<dyn Error + Send + Sync>> {
        let (name, part, frame, fill_color, captured, pixels) = match &self.source {
            SavedLayerPixels::BlankFill {
                width,
                height,
                color,
            } => {
                charge_pixels(budget, *width, *height)?;
                let img = CvnsImageLayer::empty_rgba(*width, *height, *color)?;
                (b" ".to_vec(), b" ".to_vec(), None, Some(*color), false, img)
            }
            SavedLayerPixels::Asset { name, part, frame } => {
                let decoded = if let Some(idx) = frame {
                    image::load_frame(images, name, *idx as usize)?
                } else if !part.is_empty() {
                    image::load_part(images, name, part)?
                } else {
                    image::load(images, name)?
                }
                .ok_or("failed to reload CVNS snapshot image asset")?;
                charge_pixels(budget, decoded.pixels.width(), decoded.pixels.height())?;
                let pixels = if force_opaque && !self.use_alpha {
                    force_opaque_alpha(decoded.pixels)?
                } else {
                    decoded.pixels
                };
                (name.clone(), part.clone(), *frame, None, false, pixels)
            }
            SavedLayerPixels::RawLzss {
                width,
                height,
                packed,
            } => {
                let bytes = charge_pixels(budget, *width, *height)?;
                let raw = lzss::decode(packed, bytes)?;
                let img = RgbaImage::new(*width, *height, raw)?;
                (b" ".to_vec(), b" ".to_vec(), None, None, true, img)
            }
        };
        let raw_unfiltered = (captured && self.filter_mode != 0).then(|| pixels.clone());
        let mut layer =
            CvnsImageLayer::from_image(name, part, pixels, self.dx, self.dy, self.use_alpha);
        layer.frame = frame;
        layer.fill_color = fill_color;
        layer.captured = captured;
        layer.filter_mode = self.filter_mode;
        layer.filter_arg = self.filter_arg;
        layer.monochrome = self.monochrome;
        layer.hsl_from = self.hsl_from;
        layer.hsl_to = self.hsl_to;
        layer.rgb_mul = self.rgb_mul;
        layer.apply_saved_effects()?;
        // Raw captured layers already hold tinted pixels; avoid double-tinting base pixels.
        if let Some(unfiltered) = raw_unfiltered {
            let mut reset = CvnsImageLayer::from_image(
                layer.name.clone(),
                layer.part.clone(),
                unfiltered,
                self.dx,
                self.dy,
                self.use_alpha,
            );
            reset.captured = true;
            reset.monochrome = self.monochrome;
            reset.hsl_from = self.hsl_from;
            reset.hsl_to = self.hsl_to;
            reset.rgb_mul = self.rgb_mul;
            reset.filter_mode = self.filter_mode;
            reset.filter_arg = self.filter_arg;
            reset.filtered = layer.filtered;
            return Ok(reset);
        }
        Ok(layer)
    }
}

fn force_opaque_alpha(img: RgbaImage) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
    let w = img.width();
    let h = img.height();
    let mut px = img.pixels().to_vec();
    for p in px.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    Ok(RgbaImage::new(w, h, px)?)
}

fn charge_pixels(budget: &mut usize, width: u32, height: u32) -> Result<usize, FormatError> {
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(FormatError("CVNS snapshot image dimensions out of bounds"));
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(4))
        .ok_or(FormatError("CVNS snapshot image byte size overflow"))?;
    *budget = budget
        .checked_sub(bytes)
        .ok_or(FormatError("CVNS snapshot image allocation exceeds budget"))?;
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub enum SavedMaskSource {
    Asset(Vec<u8>),
    RawLzss {
        width: u32,
        height: u32,
        packed: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct SavedCvnsImageBuffer {
    pub layers: Vec<Option<SavedCvnsLayer>>,
    pub has_composite: bool,
    pub mask: Option<SavedMaskSource>,
}

impl SavedCvnsImageBuffer {
    pub fn from_buffer(buf: &CvnsImageBuffer) -> Result<Self, FormatError> {
        let mut layers = Vec::with_capacity(CVNS_LAYERS);
        for layer in &buf.layers {
            layers.push(layer.as_ref().map(SavedCvnsLayer::from_layer).transpose()?);
        }
        let mask = buf.mask.as_ref().map(|m| {
            if !m.name.is_empty() {
                SavedMaskSource::Asset(m.name.clone())
            } else {
                SavedMaskSource::RawLzss {
                    width: m.width,
                    height: m.height,
                    packed: lzss::encode(&m.data),
                }
            }
        });
        Ok(Self {
            layers,
            has_composite: buf.composited.is_some(),
            mask,
        })
    }

    pub fn load(
        &self,
        images: &mut dyn ImageSource,
        budget: &mut usize,
    ) -> Result<CvnsImageBuffer, Box<dyn Error + Send + Sync>> {
        let mut layers = Vec::with_capacity(CVNS_LAYERS);
        for slot in &self.layers {
            layers.push(
                slot.as_ref()
                    .map(|l| l.load(images, budget, false))
                    .transpose()?,
            );
        }
        let mask = match &self.mask {
            None => None,
            Some(SavedMaskSource::Asset(name)) => {
                let dec = image::load(images, name)?
                    .ok_or("failed to reload CVNS snapshot image mask")?;
                charge_pixels(budget, dec.pixels.width(), dec.pixels.height())?;
                Some(CvnsImageLayer::make_mask(name.clone(), &dec.pixels))
            }
            Some(SavedMaskSource::RawLzss {
                width,
                height,
                packed,
            }) => {
                let px_count = charge_pixels(budget, *width, *height)? / 4;
                let data = lzss::decode(packed, px_count)?;
                Some(CvnsMaskLayer {
                    name: Vec::new(),
                    width: *width,
                    height: *height,
                    data,
                })
            }
        };
        let mut out = CvnsImageBuffer {
            layers,
            composited: None,
            mask,
        };
        if self.has_composite {
            out.rebuild_composite()?;
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct SavedCvnsMaskTransition {
    pub sprite_slot: u16,
    pub reveal: bool,
    pub duration_ms: u32,
    pub ramp_ms: u32,
    pub elapsed_ms: u32,
    pub width: u32,
    pub height: u32,
    pub mask_packed: Vec<u8>,
}

impl SavedCvnsMaskTransition {
    pub fn from_transition(tr: &CvnsMaskTransition) -> Self {
        Self {
            sprite_slot: tr.sprite_slot,
            reveal: tr.reveal,
            duration_ms: tr.duration_ms,
            ramp_ms: tr.ramp_ms,
            elapsed_ms: tr.elapsed_ms,
            width: tr.width,
            height: tr.height,
            mask_packed: lzss::encode(&tr.mask),
        }
    }

    pub fn restore(&self, budget: &mut usize) -> Result<CvnsMaskTransition, FormatError> {
        let len = charge_pixels(budget, self.width, self.height)? / 4;
        let mask = lzss::decode(&self.mask_packed, len)?;
        Ok(CvnsMaskTransition {
            sprite_slot: self.sprite_slot,
            reveal: self.reveal,
            duration_ms: self.duration_ms,
            ramp_ms: self.ramp_ms,
            elapsed_ms: self.elapsed_ms,
            width: self.width,
            height: self.height,
            mask,
            thresholds: CvnsMaskTransition::compute_thresholds(self.duration_ms),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SavedCvnsMessage {
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
    pub local_images: Vec<Option<SavedCvnsLayer>>,
    pub local_sprites: Vec<Option<CvnsSprite>>,
    pub buttons: [CvnsInlineButton; CVNS_INLINE_BUTTONS],
}

impl SavedCvnsMessage {
    pub fn from_message(msg: &CvnsMessage) -> Result<Self, FormatError> {
        let mut local_images = Vec::with_capacity(CVNS_LOCAL_IMAGES);
        for slot in &msg.local_images {
            local_images.push(slot.as_ref().map(SavedCvnsLayer::from_layer).transpose()?);
        }
        Ok(Self {
            win_slot: msg.win_slot,
            x: msg.x,
            y: msg.y,
            w: msg.w,
            h: msg.h,
            visible: msg.visible,
            paused: msg.paused,
            wait_cursor_mode: msg.wait_cursor_mode,
            wait_cursor_offset: msg.wait_cursor_offset,
            attached_flags: msg.attached_flags,
            attached_offsets: msg.attached_offsets,
            local_images,
            local_sprites: msg.local_sprites.clone(),
            buttons: msg.buttons,
        })
    }

    pub fn load(
        &self,
        images: &mut dyn ImageSource,
        budget: &mut usize,
    ) -> Result<CvnsMessage, Box<dyn Error + Send + Sync>> {
        let mut local_images = Vec::with_capacity(CVNS_LOCAL_IMAGES);
        for slot in &self.local_images {
            local_images.push(
                slot.as_ref()
                    .map(|l| l.load(images, budget, true))
                    .transpose()?,
            );
        }
        Ok(CvnsMessage::from_saved(
            crate::message::cvns::RestoredCvnsMessage {
                win_slot: self.win_slot,
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                visible: self.visible,
                paused: self.paused,
                wait_cursor_mode: self.wait_cursor_mode,
                wait_cursor_offset: self.wait_cursor_offset,
                attached_flags: self.attached_flags,
                attached_offsets: self.attached_offsets,
                local_images,
                local_sprites: self.local_sprites.clone(),
                buttons: self.buttons,
            },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct CvnsSnapshot {
    pub save_title: Vec<u8>,
    pub scene_title: Vec<u8>,
    pub script_names: [Vec<u8>; 4],
    pub script_sizes: [u32; 4],
    pub active_slot: u32,
    pub pc: u32,
    pub repeat_count: u32,
    pub timers: [u32; 10],
    pub popup: u32,
    pub callbacks: Vec<u8>,
    pub stack_pointer: u32,
    pub frame_depth: u32,
    pub frames: [u32; 64],
    pub stack: Vec<u8>,
    pub system_results: [u32; 11],
    pub script_data: Vec<u8>,
    pub variables: CvnsLocalVariables,
    pub clear_color: u32,
    pub clear_depth: bool,
    pub backdrop_disabled: bool,
    pub layer_fade_alpha: Option<u16>,
    pub fade: Option<CvnsFade>,
    pub mask_transition: Option<SavedCvnsMaskTransition>,
    pub quake: Option<CvnsQuake>,
    pub cg_seen: BTreeSet<Vec<u8>>,
    pub images: BTreeMap<u16, SavedCvnsImageBuffer>,
    pub sprites: BTreeMap<u16, CvnsSprite>,
    pub text_shadow: bool,
    pub read_skip: bool,
    pub auto_skip: bool,
    pub auto_wait_elapsed_ms: u32,
    pub click_wait_elapsed_ms: u32,
    pub windows: BTreeMap<u16, CvnsWindow>,
    pub texts: BTreeMap<u16, TextObject>,
    pub messages: BTreeMap<u16, SavedCvnsMessage>,
    pub audio: Vec<u8>,
    pub history: Vec<u8>,
}

pub(crate) struct CvnsSceneCapture<'a> {
    pub scene: &'a CvnsScene,
    pub msg_host: Option<&'a CvnsMessageHost>,
    pub texts: &'a [Option<TextObject>],
    pub clear_color: u32,
    pub clear_depth: bool,
    pub backdrop_disabled: bool,
    pub layer_fade_alpha: Option<u16>,
}

pub(crate) struct CvnsVmCapture {
    pub save_title: Vec<u8>,
    pub scene_title: Vec<u8>,
    pub script_names: [Vec<u8>; 4],
    pub script_sizes: [u32; 4],
    pub active_slot: u32,
    pub pc: u32,
    pub repeat_count: u32,
    pub timers: [u32; 10],
    pub popup: u32,
    pub callbacks: Vec<u8>,
    pub stack_pointer: u32,
    pub frame_depth: u32,
    pub frames: [u32; 64],
    pub stack: Vec<u8>,
    pub system_results: [u32; 11],
    pub script_data: Vec<u8>,
    pub variables: CvnsLocalVariables,
    pub audio: Vec<u8>,
    pub history: Vec<u8>,
}

impl CvnsSnapshot {
    pub(crate) fn from_scene_and_messages(
        visual: CvnsSceneCapture<'_>,
        vm: CvnsVmCapture,
    ) -> Result<Self, FormatError> {
        let mut images = BTreeMap::new();
        for (i, slot) in visual.scene.images.iter().enumerate() {
            if let Some(buf) = slot {
                images.insert(i as u16, SavedCvnsImageBuffer::from_buffer(buf)?);
            }
        }
        let mut sprites = BTreeMap::new();
        for (i, slot) in visual.scene.sprites.iter().enumerate() {
            if let Some(sp) = slot {
                sprites.insert(i as u16, sp.clone());
            }
        }
        let mut windows = BTreeMap::new();
        let mut messages = BTreeMap::new();
        let (text_shadow, read_skip, auto_skip, auto_wait_elapsed_ms, click_wait_elapsed_ms) =
            if let Some(cvns) = visual.msg_host {
                for (i, w) in cvns.windows.iter().enumerate() {
                    if let Some(win) = w {
                        windows.insert(i as u16, win.clone());
                    }
                }
                for (i, m) in cvns.messages.iter().enumerate() {
                    if let Some(msg) = m {
                        messages.insert(i as u16, SavedCvnsMessage::from_message(msg)?);
                    }
                }
                (
                    cvns.text_shadow,
                    cvns.read_skip,
                    cvns.auto_skip,
                    cvns.auto_wait_elapsed_ms,
                    cvns.click_wait_elapsed_ms,
                )
            } else {
                (false, false, false, 0, 0)
            };
        let mut texts = BTreeMap::new();
        for (i, t) in visual.texts.iter().take(CVNS_WINDOWS).enumerate() {
            if let Some(text) = t {
                texts.insert(i as u16, text.clone());
            }
        }
        Ok(Self {
            save_title: vm.save_title,
            scene_title: vm.scene_title,
            script_names: vm.script_names,
            script_sizes: vm.script_sizes,
            active_slot: vm.active_slot,
            pc: vm.pc,
            repeat_count: vm.repeat_count,
            timers: vm.timers,
            popup: vm.popup,
            callbacks: vm.callbacks,
            stack_pointer: vm.stack_pointer,
            frame_depth: vm.frame_depth,
            frames: vm.frames,
            stack: vm.stack,
            system_results: vm.system_results,
            script_data: vm.script_data,
            variables: vm.variables,
            clear_color: visual.clear_color,
            clear_depth: visual.clear_depth,
            backdrop_disabled: visual.backdrop_disabled,
            layer_fade_alpha: visual.layer_fade_alpha,
            fade: visual.scene.fade,
            mask_transition: visual
                .scene
                .mask_transition
                .as_ref()
                .map(SavedCvnsMaskTransition::from_transition),
            quake: visual.scene.quake,
            cg_seen: visual.scene.cg_seen.clone(),
            images,
            sprites,
            text_shadow,
            read_skip,
            auto_skip,
            auto_wait_elapsed_ms,
            click_wait_elapsed_ms,
            windows,
            texts,
            messages,
            audio: vm.audio,
            history: vm.history,
        })
    }

    /// Whether `bytes` begins with `cmvs-rs` `0xa00` CVNS snapshot records (rather than an
    /// original `cvns.exe` `0x100..` payload inside a `CSP1` container).
    pub fn has_cmvs_rs_records(bytes: &[u8]) -> bool {
        bytes.len() >= 10 && u16::from_le_bytes([bytes[0], bytes[1]]) == 0xa00
    }

    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = Writer::default();
        out.record(0xa00, None, &self.encode_vm_block()?)?;
        out.record(0xa01, None, &self.encode_scene_meta()?)?;
        for (&slot, buf) in &self.images {
            if usize::from(slot) >= CVNS_IMAGES {
                return Err(FormatError("CVNS image slot out of bounds"));
            }
            out.record(0xa02, Some(slot), &encode_image_buffer(buf)?)?;
        }
        for (&slot, sp) in &self.sprites {
            if usize::from(slot) >= CVNS_SPRITES {
                return Err(FormatError("CVNS sprite slot out of bounds"));
            }
            let mut body = Vec::new();
            write_sprite(&mut body, sp);
            out.record(0xa03, Some(slot), &body)?;
        }
        out.record(0xa04, None, &self.encode_message_meta())?;
        for (&slot, win) in &self.windows {
            if usize::from(slot) >= CVNS_WINDOWS {
                return Err(FormatError("CVNS window slot out of bounds"));
            }
            out.record(0xa05, Some(slot), &encode_window(win)?)?;
        }
        for (&slot, obj) in &self.texts {
            if usize::from(slot) >= CVNS_WINDOWS {
                return Err(FormatError("CVNS text slot out of bounds"));
            }
            out.record(
                0xa06,
                Some(slot),
                &text::encode_with_layout(obj, SnapshotLayout::CvnsCsp1)?,
            )?;
        }
        for (&slot, msg) in &self.messages {
            if usize::from(slot) >= CVNS_MESSAGES {
                return Err(FormatError("CVNS message slot out of bounds"));
            }
            out.record(0xa07, Some(slot), &encode_message(msg)?)?;
        }
        out.record(0xa08, None, &self.audio)?;
        out.record(0xa09, None, &self.history)?;
        Ok(out.0)
    }

    /// Decodes the `cmvs-rs` schema. Original CSP1 imports use the game-bound runtime loader.
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let records = parse_records(bytes)?;
        let vm_body = unique_body(bytes, &records, 0xa00).map_err(|err| {
            if records.iter().any(|r| (0x100..0xa00).contains(&r.tag)) {
                FormatError("native cvns.exe CSP1 snapshot payload is not supported")
            } else {
                err
            }
        })?;
        let vm = decode_vm_block(vm_body)?;
        let scene_meta = decode_scene_meta(unique_body(bytes, &records, 0xa01)?)?;
        let msg_meta = decode_message_meta(unique_body(bytes, &records, 0xa04)?)?;
        let audio = unique_body(bytes, &records, 0xa08)?.to_vec();
        let history = unique_body(bytes, &records, 0xa09)?.to_vec();

        let mut images = BTreeMap::new();
        let mut sprites = BTreeMap::new();
        let mut windows = BTreeMap::new();
        let mut texts = BTreeMap::new();
        let mut messages = BTreeMap::new();

        for record in &records {
            let body = &bytes[record.body.clone()];
            match record.tag {
                0xa00 | 0xa01 | 0xa04 | 0xa08 | 0xa09 => {}
                0xa02 => {
                    let slot = require_slot(record, CVNS_IMAGES)?;
                    insert_unique(&mut images, slot, decode_image_buffer(body)?)?;
                }
                0xa03 => {
                    let slot = require_slot(record, CVNS_SPRITES)?;
                    let mut r = Reader::new(body);
                    let sp = read_sprite(&mut r)?;
                    r.finish()?;
                    insert_unique(&mut sprites, slot, sp)?;
                }
                0xa05 => {
                    let slot = require_slot(record, CVNS_WINDOWS)?;
                    insert_unique(&mut windows, slot, decode_window(body)?)?;
                }
                0xa06 => {
                    let slot = require_slot(record, CVNS_WINDOWS)?;
                    let obj = text::decode_with_layout(body, SnapshotLayout::CvnsCsp1)?;
                    insert_unique(&mut texts, slot, obj)?;
                }
                0xa07 => {
                    let slot = require_slot(record, CVNS_MESSAGES)?;
                    insert_unique(&mut messages, slot, decode_message(body)?)?;
                }
                _ => return Err(FormatError("unexpected record tag in CVNS snapshot")),
            }
        }

        Ok(Self {
            save_title: vm.save_title,
            scene_title: vm.scene_title,
            script_names: vm.script_names,
            script_sizes: vm.script_sizes,
            active_slot: vm.active_slot,
            pc: vm.pc,
            repeat_count: vm.repeat_count,
            timers: vm.timers,
            popup: vm.popup,
            callbacks: vm.callbacks,
            stack_pointer: vm.stack_pointer,
            frame_depth: vm.frame_depth,
            frames: vm.frames,
            stack: vm.stack,
            system_results: vm.system_results,
            script_data: vm.script_data,
            variables: vm.variables,
            clear_color: scene_meta.clear_color,
            clear_depth: scene_meta.clear_depth,
            backdrop_disabled: scene_meta.backdrop_disabled,
            layer_fade_alpha: scene_meta.layer_fade_alpha,
            fade: scene_meta.fade,
            mask_transition: scene_meta.mask_transition,
            quake: scene_meta.quake,
            cg_seen: scene_meta.cg_seen,
            images,
            sprites,
            text_shadow: msg_meta.0,
            read_skip: msg_meta.1,
            auto_skip: msg_meta.2,
            auto_wait_elapsed_ms: msg_meta.3,
            click_wait_elapsed_ms: msg_meta.4,
            windows,
            texts,
            messages,
            audio,
            history,
        })
    }

    fn encode_vm_block(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = Vec::new();
        out.extend_from_slice(&CVNS_SNAPSHOT_SCHEMA_VERSION.to_le_bytes());
        put_bytes(&mut out, &self.save_title);
        put_bytes(&mut out, &self.scene_title);
        for name in &self.script_names {
            put_bytes(&mut out, name);
        }
        for s in self.script_sizes {
            out.extend_from_slice(&s.to_le_bytes());
        }
        for w in [
            self.active_slot,
            self.pc,
            self.repeat_count,
            self.popup,
            self.stack_pointer,
            self.frame_depth,
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for t in self.timers {
            out.extend_from_slice(&t.to_le_bytes());
        }
        put_bytes(&mut out, &self.callbacks);
        for f in self.frames {
            out.extend_from_slice(&f.to_le_bytes());
        }
        put_bytes(&mut out, &self.stack);
        for r in self.system_results {
            out.extend_from_slice(&r.to_le_bytes());
        }
        put_bytes(&mut out, &self.script_data);
        out.extend_from_slice(&self.variables.starts.bit.to_le_bytes());
        out.extend_from_slice(&self.variables.starts.integer.to_le_bytes());
        out.extend_from_slice(&self.variables.starts.string.to_le_bytes());
        for &v in &self.variables.integers {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.variables.bits);
        for s in &self.variables.strings {
            if s.len() >= 1024 || s.contains(&0) {
                return Err(FormatError("CVNS snapshot variable string invalid"));
            }
            put_bytes(&mut out, s);
        }
        Ok(out)
    }

    fn encode_scene_meta(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.clear_color.to_le_bytes());
        out.push(u8::from(self.clear_depth));
        out.push(u8::from(self.backdrop_disabled));
        match self.layer_fade_alpha {
            Some(a) => {
                out.push(1);
                out.extend_from_slice(&a.to_le_bytes());
            }
            None => out.push(0),
        }
        match self.fade {
            Some(f) => {
                out.push(1);
                out.push(u8::from(f.to_normal));
                out.extend_from_slice(&f.color.to_le_bytes());
                out.extend_from_slice(&f.duration_ms.to_le_bytes());
                out.extend_from_slice(&f.elapsed_ms.to_le_bytes());
            }
            None => out.push(0),
        }
        match &self.mask_transition {
            Some(tr) => {
                out.push(1);
                out.extend_from_slice(&tr.sprite_slot.to_le_bytes());
                out.push(u8::from(tr.reveal));
                out.extend_from_slice(&tr.duration_ms.to_le_bytes());
                out.extend_from_slice(&tr.ramp_ms.to_le_bytes());
                out.extend_from_slice(&tr.elapsed_ms.to_le_bytes());
                out.extend_from_slice(&tr.width.to_le_bytes());
                out.extend_from_slice(&tr.height.to_le_bytes());
                put_bytes(&mut out, &tr.mask_packed);
            }
            None => out.push(0),
        }
        match self.quake {
            Some(q) => {
                out.push(1);
                out.extend_from_slice(&q.kind.to_le_bytes());
                out.extend_from_slice(&q.amplitude.to_le_bytes());
                out.extend_from_slice(&q.interval_ms.to_le_bytes());
                out.extend_from_slice(&q.count.to_le_bytes());
                out.extend_from_slice(&q.phase.to_le_bytes());
                out.push(u8::from(q.paused));
                out.extend_from_slice(&q.elapsed_ms.to_le_bytes());
                out.extend_from_slice(&q.step_count.to_le_bytes());
                out.extend_from_slice(&q.offset_x.to_le_bytes());
                out.extend_from_slice(&q.offset_y.to_le_bytes());
                out.extend_from_slice(&q.rng().to_le_bytes());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.cg_seen.len() as u32).to_le_bytes());
        for name in &self.cg_seen {
            put_bytes(&mut out, name);
        }
        Ok(out)
    }

    fn encode_message_meta(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(u8::from(self.text_shadow));
        out.push(u8::from(self.read_skip));
        out.push(u8::from(self.auto_skip));
        out.extend_from_slice(&self.auto_wait_elapsed_ms.to_le_bytes());
        out.extend_from_slice(&self.click_wait_elapsed_ms.to_le_bytes());
        out
    }
}

struct DecodedVmBlock {
    save_title: Vec<u8>,
    scene_title: Vec<u8>,
    script_names: [Vec<u8>; 4],
    script_sizes: [u32; 4],
    active_slot: u32,
    pc: u32,
    repeat_count: u32,
    timers: [u32; 10],
    popup: u32,
    callbacks: Vec<u8>,
    stack_pointer: u32,
    frame_depth: u32,
    frames: [u32; 64],
    stack: Vec<u8>,
    system_results: [u32; 11],
    script_data: Vec<u8>,
    variables: CvnsLocalVariables,
}

struct DecodedSceneMeta {
    clear_color: u32,
    clear_depth: bool,
    backdrop_disabled: bool,
    layer_fade_alpha: Option<u16>,
    fade: Option<CvnsFade>,
    mask_transition: Option<SavedCvnsMaskTransition>,
    quake: Option<CvnsQuake>,
    cg_seen: BTreeSet<Vec<u8>>,
}

fn require_slot(record: &SnapshotRecord, limit: usize) -> Result<u16, FormatError> {
    let slot = record
        .index
        .ok_or(FormatError("CVNS indexed record lacks slot"))?;
    if usize::from(slot) >= limit {
        return Err(FormatError("CVNS snapshot slot exceeds table limit"));
    }
    Ok(slot)
}

fn insert_unique<T>(map: &mut BTreeMap<u16, T>, key: u16, value: T) -> Result<(), FormatError> {
    if map.insert(key, value).is_some() {
        return Err(FormatError("duplicate CVNS snapshot slot record"));
    }
    Ok(())
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn read_bytes<'a>(r: &mut Reader<'a>, limit: usize) -> Result<Vec<u8>, FormatError> {
    let len = r.u32()? as usize;
    if len > limit {
        return Err(FormatError("CVNS snapshot byte field exceeds limit"));
    }
    Ok(r.take(len)?.to_vec())
}

fn read_byte(r: &mut Reader<'_>) -> Result<u8, FormatError> {
    Ok(r.take(1)?[0])
}

fn read_bool(r: &mut Reader<'_>) -> Result<bool, FormatError> {
    match read_byte(r)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FormatError("invalid CVNS snapshot boolean")),
    }
}

fn read_i16(r: &mut Reader<'_>) -> Result<i16, FormatError> {
    Ok(i16::from_le_bytes(r.take(2)?.try_into().unwrap()))
}

fn read_i32(r: &mut Reader<'_>) -> Result<i32, FormatError> {
    Ok(i32::from_le_bytes(r.take(4)?.try_into().unwrap()))
}

fn decode_vm_block(bytes: &[u8]) -> Result<DecodedVmBlock, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u32()? != CVNS_SNAPSHOT_SCHEMA_VERSION {
        return Err(FormatError("unsupported CVNS snapshot VM block version"));
    }
    let save_title = read_bytes(&mut r, 256)?;
    let scene_title = read_bytes(&mut r, 256)?;
    let script_names = [
        read_bytes(&mut r, 256)?,
        read_bytes(&mut r, 256)?,
        read_bytes(&mut r, 256)?,
        read_bytes(&mut r, 256)?,
    ];
    let script_sizes = r.words::<4>()?;
    let [
        active_slot,
        pc,
        repeat_count,
        popup,
        stack_pointer,
        frame_depth,
    ] = r.words::<6>()?;
    let timers = r.words::<10>()?;
    let callbacks = read_bytes(&mut r, 1796)?;
    if callbacks.len() != 1796 {
        return Err(FormatError("CVNS snapshot callback table length mismatch"));
    }
    let frames = r.words::<64>()?;
    let stack = read_bytes(&mut r, 0x10000)?;
    if stack.len() != 0x10000 {
        return Err(FormatError("CVNS snapshot stack size mismatch"));
    }
    let system_results = r.words::<11>()?;
    let script_data = read_bytes(&mut r, 0x30_0000)?;
    let starts = PersistentStarts {
        bit: r.u32()?,
        integer: r.u32()?,
        string: r.u32()?,
    }
    .checked()?;
    let mut integers = Vec::with_capacity(starts.integer as usize);
    for _ in 0..starts.integer {
        integers.push(read_i32(&mut r)?);
    }
    let bit_bytes = (starts.bit >> 3) as usize;
    let bits = r.take(bit_bytes)?.to_vec();
    let mut strings = Vec::with_capacity(starts.string as usize);
    for _ in 0..starts.string {
        let s = read_bytes(&mut r, 1023)?;
        if s.contains(&0) {
            return Err(FormatError("CVNS snapshot variable string contains NUL"));
        }
        strings.push(s);
    }
    r.finish()?;
    Ok(DecodedVmBlock {
        save_title,
        scene_title,
        script_names,
        script_sizes,
        active_slot,
        pc,
        repeat_count,
        timers,
        popup,
        callbacks,
        stack_pointer,
        frame_depth,
        frames,
        stack,
        system_results,
        script_data,
        variables: CvnsLocalVariables {
            starts,
            integers,
            bits,
            strings,
        },
    })
}

fn decode_scene_meta(bytes: &[u8]) -> Result<DecodedSceneMeta, FormatError> {
    let mut r = Reader::new(bytes);
    let clear_color = r.u32()?;
    let clear_depth = read_bool(&mut r)?;
    let backdrop_disabled = read_bool(&mut r)?;
    let layer_fade_alpha = if read_bool(&mut r)? {
        Some(r.u16()?.min(256))
    } else {
        None
    };
    let fade = if read_bool(&mut r)? {
        Some(CvnsFade {
            to_normal: read_bool(&mut r)?,
            color: r.u32()?,
            duration_ms: r.u32()?,
            elapsed_ms: r.u32()?,
        })
    } else {
        None
    };
    let mask_transition = if read_bool(&mut r)? {
        let sprite_slot = r.u16()?;
        if usize::from(sprite_slot) >= CVNS_SPRITES {
            return Err(FormatError("CVNS mask transition sprite out of range"));
        }
        Some(SavedCvnsMaskTransition {
            sprite_slot,
            reveal: read_bool(&mut r)?,
            duration_ms: r.u32()?,
            ramp_ms: r.u32()?,
            elapsed_ms: r.u32()?,
            width: r.u32()?,
            height: r.u32()?,
            mask_packed: read_bytes(&mut r, 0x30_0000)?,
        })
    } else {
        None
    };
    let quake = if read_bool(&mut r)? {
        Some(CvnsQuake::from_saved(
            r.u32()?,
            read_i32(&mut r)?,
            r.u32()?,
            read_i32(&mut r)?,
            r.u32()?,
            read_bool(&mut r)?,
            r.u32()?,
            read_i32(&mut r)?,
            read_i32(&mut r)?,
            read_i32(&mut r)?,
            r.u32()?,
        ))
    } else {
        None
    };
    let cg_count = r.u32()? as usize;
    if cg_count > 65536 {
        return Err(FormatError("CVNS snapshot CG count exceeds bounds"));
    }
    let mut cg_seen = BTreeSet::new();
    for _ in 0..cg_count {
        cg_seen.insert(read_bytes(&mut r, 256)?);
    }
    r.finish()?;
    Ok(DecodedSceneMeta {
        clear_color,
        clear_depth,
        backdrop_disabled,
        layer_fade_alpha,
        fade,
        mask_transition,
        quake,
        cg_seen,
    })
}

fn decode_message_meta(bytes: &[u8]) -> Result<(bool, bool, bool, u32, u32), FormatError> {
    let mut r = Reader::new(bytes);
    let res = (
        read_bool(&mut r)?,
        read_bool(&mut r)?,
        read_bool(&mut r)?,
        r.u32()?,
        r.u32()?,
    );
    r.finish()?;
    Ok(res)
}

fn write_layer(out: &mut Vec<u8>, l: &SavedCvnsLayer) {
    match &l.source {
        SavedLayerPixels::BlankFill {
            width,
            height,
            color,
        } => {
            out.push(0);
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            out.extend_from_slice(&color.to_le_bytes());
        }
        SavedLayerPixels::Asset { name, part, frame } => {
            out.push(1);
            put_bytes(out, name);
            put_bytes(out, part);
            match frame {
                Some(f) => {
                    out.push(1);
                    out.extend_from_slice(&f.to_le_bytes());
                }
                None => out.push(0),
            }
        }
        SavedLayerPixels::RawLzss {
            width,
            height,
            packed,
        } => {
            out.push(2);
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            put_bytes(out, packed);
        }
    }
    out.push(u8::from(l.use_alpha));
    for v in [
        l.dx,
        l.dy,
        l.filter_mode as i32,
        l.filter_arg,
        l.monochrome as i32,
        l.hsl_from,
        l.hsl_to,
        l.rgb_mul,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn read_layer(r: &mut Reader<'_>) -> Result<SavedCvnsLayer, FormatError> {
    let source = match read_byte(r)? {
        0 => SavedLayerPixels::BlankFill {
            width: r.u32()?,
            height: r.u32()?,
            color: r.u32()?,
        },
        1 => {
            let name = read_bytes(r, 256)?;
            let part = read_bytes(r, 256)?;
            let frame = if read_bool(r)? { Some(r.u32()?) } else { None };
            SavedLayerPixels::Asset { name, part, frame }
        }
        2 => SavedLayerPixels::RawLzss {
            width: r.u32()?,
            height: r.u32()?,
            packed: read_bytes(r, 0x30_0000)?,
        },
        _ => return Err(FormatError("unknown CVNS snapshot image source tag")),
    };
    let use_alpha = read_bool(r)?;
    Ok(SavedCvnsLayer {
        source,
        use_alpha,
        dx: read_i32(r)?,
        dy: read_i32(r)?,
        filter_mode: r.u32()?,
        filter_arg: read_i32(r)?,
        monochrome: r.u32()?,
        hsl_from: read_i32(r)?,
        hsl_to: read_i32(r)?,
        rgb_mul: read_i32(r)?,
    })
}

fn encode_image_buffer(buf: &SavedCvnsImageBuffer) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for (idx, layer) in buf.layers.iter().enumerate() {
        if let Some(l) = layer {
            out.push(idx as u8);
            write_layer(&mut out, l);
        }
    }
    out.push(0xff);
    out.push(u8::from(buf.has_composite));
    match &buf.mask {
        None => out.push(0),
        Some(SavedMaskSource::Asset(name)) => {
            out.push(1);
            put_bytes(&mut out, name);
        }
        Some(SavedMaskSource::RawLzss {
            width,
            height,
            packed,
        }) => {
            out.push(2);
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            put_bytes(&mut out, packed);
        }
    }
    Ok(out)
}

fn decode_image_buffer(bytes: &[u8]) -> Result<SavedCvnsImageBuffer, FormatError> {
    let mut r = Reader::new(bytes);
    let mut layers: Vec<Option<SavedCvnsLayer>> = vec![None; CVNS_LAYERS];
    loop {
        let idx = read_byte(&mut r)?;
        if idx == 0xff {
            break;
        }
        if usize::from(idx) >= CVNS_LAYERS {
            return Err(FormatError("CVNS snapshot image layer outside 0..15"));
        }
        if layers[usize::from(idx)].is_some() {
            return Err(FormatError("duplicate CVNS snapshot image layer"));
        }
        layers[usize::from(idx)] = Some(read_layer(&mut r)?);
    }
    let has_composite = read_bool(&mut r)?;
    let mask = match read_byte(&mut r)? {
        0 => None,
        1 => Some(SavedMaskSource::Asset(read_bytes(&mut r, 256)?)),
        2 => Some(SavedMaskSource::RawLzss {
            width: r.u32()?,
            height: r.u32()?,
            packed: read_bytes(&mut r, 0x30_0000)?,
        }),
        _ => return Err(FormatError("invalid CVNS snapshot mask kind")),
    };
    r.finish()?;
    Ok(SavedCvnsImageBuffer {
        layers,
        has_composite,
        mask,
    })
}

fn write_sprite(out: &mut Vec<u8>, sp: &CvnsSprite) {
    match sp.kind {
        CvnsSpriteKind::Image => out.push(0),
        CvnsSpriteKind::Video => out.push(1),
        CvnsSpriteKind::SolidRect { color } => {
            out.push(2);
            out.extend_from_slice(&color.to_le_bytes());
        }
    }
    out.push(u8::from(sp.visible));
    out.push(u8::from(sp.finished_delete));
    out.push(u8::from(sp.dirty));
    out.extend_from_slice(&sp.image_slot.to_le_bytes());
    for v in sp.src_rect {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [
        sp.x,
        sp.y,
        sp.w,
        sp.h,
        sp.origin_x,
        sp.origin_y,
        sp.scale_x,
        sp.scale_y,
        sp.rotation,
        sp.z,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&sp.blend_mode.to_le_bytes());
    out.extend_from_slice(&sp.color_strength.to_le_bytes());
    out.extend_from_slice(&sp.color.to_le_bytes());
    out.extend_from_slice(&sp.alpha.to_le_bytes());
    out.extend_from_slice(&sp.flags.to_le_bytes());
    out.extend_from_slice(&sp.mask_mode.to_le_bytes());
    match sp.anim {
        Some(a) => {
            out.push(1);
            out.extend_from_slice(&a.track.to_le_bytes());
            out.extend_from_slice(&a.current_frame.to_le_bytes());
            out.extend_from_slice(&a.remaining_ms.to_le_bytes());
        }
        None => out.push(0),
    }
    for p in sp.playlists {
        out.extend_from_slice(&p.to_le_bytes());
    }
    let default_frame = CvnsAnimFrame::default();
    let non_default: Vec<(u16, &CvnsAnimFrame)> = sp
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| **f != default_frame)
        .map(|(i, f)| (i as u16, f))
        .collect();
    out.extend_from_slice(&(non_default.len() as u16).to_le_bytes());
    for (idx, f) in non_default {
        out.extend_from_slice(&idx.to_le_bytes());
        for w in [
            f.image_slot,
            f.sx,
            f.sy,
            f.sw,
            f.sh,
            f.ox,
            f.oy,
            f.duration_ms,
            f.next_frame,
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    match sp.move_tween {
        Some(t) => {
            out.push(1);
            out.push(u8::from(t.active));
            for v in [
                t.start_x,
                t.start_y,
                t.start_alpha,
                t.dst_x,
                t.dst_y,
                t.ctrl_x,
                t.ctrl_y,
                t.dst_alpha,
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for u in [t.flags, t.duration_ms, t.elapsed_ms, t.delay_ms] {
                out.extend_from_slice(&u.to_le_bytes());
            }
        }
        None => out.push(0),
    }
    match &sp.curtain {
        Some(c) => {
            out.push(1);
            for u in [
                c.mode,
                u32::from(c.delete_on_finish),
                c.cols,
                c.rows,
                c.interval_ms,
                c.steps,
                c.band_size as u32,
                c.delay_step,
                c.speed as u32,
                c.elapsed_ms,
                c.wave_phase,
                c.tiles.len() as u32,
            ] {
                out.extend_from_slice(&u.to_le_bytes());
            }
            for tile in &c.tiles {
                for v in [tile.sx, tile.sy, tile.ex, tile.ey, tile.dx, tile.dy] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                out.extend_from_slice(&tile.alpha.to_le_bytes());
            }
        }
        None => out.push(0),
    }
}

fn read_sprite(r: &mut Reader<'_>) -> Result<CvnsSprite, FormatError> {
    let kind = match read_byte(r)? {
        0 => CvnsSpriteKind::Image,
        1 => CvnsSpriteKind::Video,
        2 => CvnsSpriteKind::SolidRect { color: r.u32()? },
        _ => return Err(FormatError("unknown CVNS snapshot sprite kind")),
    };
    let visible = read_bool(r)?;
    let finished_delete = read_bool(r)?;
    let dirty = read_bool(r)?;
    let image_slot = read_i16(r)?;
    let mut src_rect = [0i32; 4];
    for v in &mut src_rect {
        *v = read_i32(r)?;
    }
    let x = read_i32(r)?;
    let y = read_i32(r)?;
    let w = read_i32(r)?;
    let h = read_i32(r)?;
    let origin_x = read_i32(r)?;
    let origin_y = read_i32(r)?;
    let scale_x = read_i32(r)?;
    let scale_y = read_i32(r)?;
    let rotation = read_i32(r)?;
    let z = read_i32(r)?;
    let blend_mode = read_i16(r)?;
    let color_strength = read_i16(r)?;
    let color = r.u32()?;
    let alpha = r.u16()?;
    let flags = r.u32()?;
    let mask_mode = read_i16(r)?;
    let anim = if read_bool(r)? {
        Some(CvnsSpriteAnim {
            track: read_i16(r)?,
            current_frame: read_i16(r)?,
            remaining_ms: r.u32()?,
        })
    } else {
        None
    };
    let mut playlists = [-1i16; CVNS_PLAYLISTS];
    for p in &mut playlists {
        *p = read_i16(r)?;
    }
    let mut frames = vec![CvnsAnimFrame::default(); CVNS_ANIM_FRAMES];
    let n_frames = r.u16()? as usize;
    if n_frames > CVNS_ANIM_FRAMES {
        return Err(FormatError(
            "CVNS sprite animation frame count out of range",
        ));
    }
    for _ in 0..n_frames {
        let idx = r.u16()? as usize;
        if idx >= CVNS_ANIM_FRAMES {
            return Err(FormatError(
                "CVNS sprite animation frame index out of range",
            ));
        }
        frames[idx] = CvnsAnimFrame {
            image_slot: read_i16(r)?,
            sx: read_i16(r)?,
            sy: read_i16(r)?,
            sw: read_i16(r)?,
            sh: read_i16(r)?,
            ox: read_i16(r)?,
            oy: read_i16(r)?,
            duration_ms: read_i16(r)?,
            next_frame: read_i16(r)?,
        };
    }
    let move_tween = if read_bool(r)? {
        Some(CvnsMoveTween {
            active: read_bool(r)?,
            start_x: read_i32(r)?,
            start_y: read_i32(r)?,
            start_alpha: read_i32(r)?,
            dst_x: read_i32(r)?,
            dst_y: read_i32(r)?,
            ctrl_x: read_i32(r)?,
            ctrl_y: read_i32(r)?,
            dst_alpha: read_i32(r)?,
            flags: r.u32()?,
            duration_ms: r.u32()?,
            elapsed_ms: r.u32()?,
            delay_ms: r.u32()?,
        })
    } else {
        None
    };
    let curtain = if read_bool(r)? {
        let mode = r.u32()?;
        let delete_on_finish = r.u32()? != 0;
        let cols = r.u32()?;
        let rows = r.u32()?;
        let interval_ms = r.u32()?;
        let steps = r.u32()?;
        let band_size = read_i32(r)?;
        let delay_step = r.u32()?;
        let speed = read_i32(r)?;
        let elapsed_ms = r.u32()?;
        let wave_phase = r.u32()?;
        let tile_count = r.u32()? as usize;
        if tile_count > 65536 {
            return Err(FormatError("CVNS sprite curtain tile count exceeds limit"));
        }
        let mut tiles = Vec::with_capacity(tile_count);
        for _ in 0..tile_count {
            tiles.push(CvnsCurtainTile {
                sx: read_i32(r)?,
                sy: read_i32(r)?,
                ex: read_i32(r)?,
                ey: read_i32(r)?,
                dx: read_i32(r)?,
                dy: read_i32(r)?,
                alpha: r.u16()?,
            });
        }
        Some(CvnsCurtain {
            mode,
            delete_on_finish,
            cols,
            rows,
            interval_ms,
            steps,
            band_size,
            delay_step,
            speed,
            elapsed_ms,
            wave_phase,
            tiles,
        })
    } else {
        None
    };
    Ok(CvnsSprite {
        kind,
        visible,
        finished_delete,
        dirty,
        image_slot,
        src_rect,
        x,
        y,
        w,
        h,
        origin_x,
        origin_y,
        scale_x,
        scale_y,
        rotation,
        z,
        blend_mode,
        color_strength,
        color,
        alpha,
        flags,
        mask_mode,
        anim,
        playlists,
        frames,
        move_tween,
        curtain,
    })
}

fn encode_window(win: &CvnsWindow) -> Result<Vec<u8>, FormatError> {
    if win.face.len() >= 64 || win.face.contains(&0) {
        return Err(FormatError("CVNS window font face invalid"));
    }
    let mut out = Vec::new();
    for v in [
        win.position[0],
        win.position[1],
        win.size[0],
        win.size[1],
        win.area[0],
        win.area[1],
        win.area[2],
        win.area[3],
        win.cursor[0],
        win.cursor[1],
        win.glyph_size,
        win.spacing[0],
        win.spacing[1],
        win.colors[0] as i32,
        win.colors[1] as i32,
        win.gradient_dir as i32,
        win.char_delay_ms,
        win.reveal[0],
        win.reveal[1],
        win.z,
        win.ruby_size,
        win.ruby_spacing,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(u8::from(win.kinsoku));
    out.push(u8::from(win.visible));
    out.push(u8::from(win.ruby));
    out.push(u8::from(win.ruby_always));
    put_bytes(&mut out, &win.face);
    Ok(out)
}

fn decode_window(bytes: &[u8]) -> Result<CvnsWindow, FormatError> {
    let mut r = Reader::new(bytes);
    let win = CvnsWindow {
        position: [read_i32(&mut r)?, read_i32(&mut r)?],
        size: [read_i32(&mut r)?, read_i32(&mut r)?],
        area: [
            read_i32(&mut r)?,
            read_i32(&mut r)?,
            read_i32(&mut r)?,
            read_i32(&mut r)?,
        ],
        cursor: [read_i32(&mut r)?, read_i32(&mut r)?],
        glyph_size: read_i32(&mut r)?,
        spacing: [read_i32(&mut r)?, read_i32(&mut r)?],
        colors: [r.u32()?, r.u32()?],
        gradient_dir: r.u32()?,
        char_delay_ms: read_i32(&mut r)?,
        reveal: [read_i32(&mut r)?, read_i32(&mut r)?],
        z: read_i32(&mut r)?,
        ruby_size: read_i32(&mut r)?,
        ruby_spacing: read_i32(&mut r)?,
        kinsoku: read_bool(&mut r)?,
        visible: read_bool(&mut r)?,
        ruby: read_bool(&mut r)?,
        ruby_always: read_bool(&mut r)?,
        face: read_bytes(&mut r, 63)?,
    };
    r.finish()?;
    Ok(win)
}

fn encode_message(msg: &SavedCvnsMessage) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for v in [msg.win_slot as i32, msg.x, msg.y, msg.w, msg.h] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(u8::from(msg.visible));
    out.push(u8::from(msg.paused));
    out.extend_from_slice(&msg.wait_cursor_mode.to_le_bytes());
    out.extend_from_slice(&msg.wait_cursor_offset[0].to_le_bytes());
    out.extend_from_slice(&msg.wait_cursor_offset[1].to_le_bytes());
    for i in 0..CVNS_LOCAL_SPRITES {
        out.push(u8::from(msg.attached_flags[i]));
        out.extend_from_slice(&msg.attached_offsets[i][0].to_le_bytes());
        out.extend_from_slice(&msg.attached_offsets[i][1].to_le_bytes());
    }
    for (idx, slot) in msg.local_images.iter().enumerate() {
        if let Some(l) = slot {
            out.push(idx as u8);
            write_layer(&mut out, l);
        }
    }
    out.push(0xff);
    for (idx, slot) in msg.local_sprites.iter().enumerate() {
        if let Some(sp) = slot {
            out.push(idx as u8);
            write_sprite(&mut out, sp);
        }
    }
    out.push(0xff);
    for b in &msg.buttons {
        out.push(u8::from(b.enabled));
        out.extend_from_slice(&b.sprite_slot.to_le_bytes());
        out.extend_from_slice(&b.cur_state.to_le_bytes());
        out.extend_from_slice(&b.prev_state.to_le_bytes());
        for h in b.hit {
            out.extend_from_slice(&h.to_le_bytes());
        }
        for s in &b.states {
            for w in [s.image_slot, s.sx, s.sy, s.sw, s.sh, s.dx, s.dy] {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
    }
    Ok(out)
}

fn decode_message(bytes: &[u8]) -> Result<SavedCvnsMessage, FormatError> {
    let mut r = Reader::new(bytes);
    let win_slot = r.u32()? as usize;
    if win_slot >= CVNS_WINDOWS {
        return Err(FormatError("CVNS message window index outside 0..7"));
    }
    let x = read_i32(&mut r)?;
    let y = read_i32(&mut r)?;
    let w = read_i32(&mut r)?;
    let h = read_i32(&mut r)?;
    let visible = read_bool(&mut r)?;
    let paused = read_bool(&mut r)?;
    let wait_cursor_mode = r.u16()?;
    let wait_cursor_offset = [read_i16(&mut r)?, read_i16(&mut r)?];
    let mut attached_flags = [false; CVNS_LOCAL_SPRITES];
    let mut attached_offsets = [[0i16; 2]; CVNS_LOCAL_SPRITES];
    for i in 0..CVNS_LOCAL_SPRITES {
        attached_flags[i] = read_bool(&mut r)?;
        attached_offsets[i] = [read_i16(&mut r)?, read_i16(&mut r)?];
    }
    let mut local_images: Vec<Option<SavedCvnsLayer>> = vec![None; CVNS_LOCAL_IMAGES];
    loop {
        let idx = read_byte(&mut r)?;
        if idx == 0xff {
            break;
        }
        if usize::from(idx) >= CVNS_LOCAL_IMAGES {
            return Err(FormatError("CVNS local image index outside 0..5"));
        }
        if local_images[usize::from(idx)].is_some() {
            return Err(FormatError("duplicate CVNS local image index"));
        }
        local_images[usize::from(idx)] = Some(read_layer(&mut r)?);
    }
    let mut local_sprites: Vec<Option<CvnsSprite>> = vec![None; CVNS_LOCAL_SPRITES];
    loop {
        let idx = read_byte(&mut r)?;
        if idx == 0xff {
            break;
        }
        if usize::from(idx) >= CVNS_LOCAL_SPRITES {
            return Err(FormatError("CVNS local sprite index outside 0..19"));
        }
        if local_sprites[usize::from(idx)].is_some() {
            return Err(FormatError("duplicate CVNS local sprite index"));
        }
        local_sprites[usize::from(idx)] = Some(read_sprite(&mut r)?);
    }
    let mut buttons = [CvnsInlineButton::default(); CVNS_INLINE_BUTTONS];
    for b in &mut buttons {
        b.enabled = read_bool(&mut r)?;
        b.sprite_slot = r.u16()?;
        b.cur_state = r.u16()?;
        b.prev_state = r.u16()?;
        for h in &mut b.hit {
            *h = read_i32(&mut r)?;
        }
        for s in &mut b.states {
            *s = CvnsButtonState {
                image_slot: read_i16(&mut r)?,
                sx: read_i16(&mut r)?,
                sy: read_i16(&mut r)?,
                sw: read_i16(&mut r)?,
                sh: read_i16(&mut r)?,
                dx: read_i16(&mut r)?,
                dy: read_i16(&mut r)?,
            };
        }
    }
    r.finish()?;
    Ok(SavedCvnsMessage {
        win_slot,
        x,
        y,
        w,
        h,
        visible,
        paused,
        wait_cursor_mode,
        wait_cursor_offset,
        attached_flags,
        attached_offsets,
        local_images,
        local_sprites,
        buttons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::{AudioHost, AudioRequest, RecordingAudio, SavedCvnsAudio},
        image::ImageSource,
        scene::cvns::CvnsSpriteAnim,
    };
    use std::sync::Arc;

    struct TestImages(BTreeMap<Vec<u8>, Vec<u8>>);

    impl ImageSource for TestImages {
        fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
            Ok(self.0.get(name).cloned())
        }
    }

    fn make_msk0(w: u32, h: u32, fill: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + (w * h) as usize);
        out.extend_from_slice(b"MSK0");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.resize(16 + (w * h) as usize, fill);
        out
    }

    fn empty_state() -> State {
        let mut state = State::new(4096, 262_144);
        state.set_string_slots(256);
        state.fill_bits(0, 262_144, false).unwrap();
        state.fill_integers(0, 4096, 0).unwrap();
        state.fill_strings(0, 256, b"").unwrap();
        state
    }

    #[test]
    fn cvns_local_variables_respect_persistent_starts_and_preserve_tail() {
        let starts = PersistentStarts {
            bit: 16,
            integer: 8,
            string: 3,
        };
        let mut live = empty_state();
        live.set_bit(0, false).unwrap();
        live.set_bit(9, true).unwrap();
        live.set_bit(16, true).unwrap();
        live.set_integer(3, -42).unwrap();
        live.set_integer(8, 999).unwrap();
        live.set_string(1, b"local_name").unwrap();
        live.set_string(3, b"persist_tail").unwrap();

        let captured = CvnsLocalVariables::capture(&live, starts).unwrap();
        // Bit 0 is set only in the serialized snapshot image (matching 0x140069270 / CVNS).
        assert_eq!(captured.bits, vec![0x01, 0x02]);
        assert_eq!(captured.integers.len(), 8);
        assert_eq!(captured.integers[3], -42);
        assert_eq!(captured.strings.len(), 3);
        assert_eq!(captured.strings[1], b"local_name");

        let mut target = empty_state();
        target.set_bit(9, false).unwrap();
        target.set_bit(16, false).unwrap();
        target.set_bit(200, true).unwrap();
        target.set_integer(3, 123).unwrap();
        target.set_integer(8, 7777).unwrap();
        target.set_string(1, b"overwritten").unwrap();
        target.set_string(3, b"keep_me").unwrap();

        captured.apply_to_state(&mut target, starts).unwrap();
        assert_eq!(target.bit(0), Some(true));
        assert_eq!(target.bit(9), Some(true));
        assert_eq!(target.bit(16), Some(false));
        assert_eq!(target.bit(200), Some(true));
        assert_eq!(target.integer(3), Some(-42));
        assert_eq!(target.integer(8), Some(7777));
        assert_eq!(target.string(1), Some(b"local_name".as_slice()));
        assert_eq!(target.string(3), Some(b"keep_me".as_slice()));

        assert!(
            captured
                .apply_to_state(
                    &mut target,
                    PersistentStarts {
                        integer: 9,
                        ..starts
                    }
                )
                .is_err()
        );
        assert!(
            CvnsLocalVariables::capture(
                &live,
                PersistentStarts {
                    string: 300,
                    ..starts
                }
            )
            .is_err()
        );
    }

    #[test]
    fn cvns_snapshot_roundtrips_scene_messages_tweens_curtains_and_audio() {
        let mut images_src = BTreeMap::new();
        images_src.insert(b"asset_mask".to_vec(), make_msk0(4, 2, 0x5a));
        images_src.insert(b"wipe_mask".to_vec(), make_msk0(2, 2, 0xc3));
        let mut src = TestImages(images_src);

        let mut scene = CvnsScene::default();
        let mut layer0 = CvnsImageLayer::from_image(
            b" ".to_vec(),
            b" ".to_vec(),
            CvnsImageLayer::empty_rgba(4, 2, 0x11223344).unwrap(),
            0,
            0,
            false,
        );
        layer0.fill_color = Some(0x11223344);
        let asset_pixels = crate::image::load(&mut src, b"asset_mask")
            .unwrap()
            .unwrap()
            .pixels;
        let mut layer1 = CvnsImageLayer::from_image(
            b"asset_mask".to_vec(),
            Vec::new(),
            asset_pixels,
            1,
            -2,
            true,
        );
        layer1.monochrome = 0x0080_4020;
        let mut layer2 = CvnsImageLayer::from_image(
            b"cap".to_vec(),
            Vec::new(),
            RgbaImage::new(2, 2, [10, 20, 30, 255].repeat(4)).unwrap(),
            0,
            0,
            false,
        );
        layer2.captured = true;

        let mut layers = vec![None; CVNS_LAYERS];
        layers[0] = Some(layer0);
        layers[1] = Some(layer1);
        layers[2] = Some(layer2);
        let mut buf = CvnsImageBuffer {
            layers,
            composited: None,
            mask: Some(CvnsMaskLayer {
                name: Vec::new(),
                width: 2,
                height: 2,
                data: vec![1, 2, 3, 4],
            }),
        };
        buf.rebuild_composite().unwrap();
        scene.images[0] = Some(buf);

        let mut sp = CvnsSprite {
            kind: CvnsSpriteKind::SolidRect { color: 0xff00_1122 },
            visible: true,
            x: 10,
            y: 20,
            w: 30,
            h: 40,
            z: 5,
            blend_mode: 2,
            alpha: 200,
            anim: Some(CvnsSpriteAnim {
                track: 1,
                current_frame: 3,
                remaining_ms: 45,
            }),
            move_tween: Some(CvnsMoveTween {
                active: true,
                start_x: 0,
                start_y: 0,
                start_alpha: 100,
                dst_x: 100,
                dst_y: 50,
                ctrl_x: 40,
                ctrl_y: -10,
                dst_alpha: 256,
                flags: 0x1000_0001,
                duration_ms: 500,
                elapsed_ms: 120,
                delay_ms: 10,
            }),
            curtain: Some(CvnsCurtain {
                mode: 1,
                delete_on_finish: true,
                cols: 2,
                rows: 1,
                interval_ms: 30,
                steps: 4,
                band_size: 2,
                delay_step: 5,
                speed: 10,
                elapsed_ms: 15,
                wave_phase: 3,
                tiles: vec![CvnsCurtainTile {
                    sx: 0,
                    sy: 0,
                    ex: 8,
                    ey: 8,
                    dx: 1,
                    dy: 2,
                    alpha: 180,
                }],
            }),
            ..Default::default()
        };
        sp.set_anim_frame(3, 4, 80, 0, 1, 2, 3, 4, 5, 6, 1);
        scene.sprites[7] = Some(sp);
        scene.fade = Some(CvnsFade {
            to_normal: true,
            color: 0x0010_2030,
            duration_ms: 300,
            elapsed_ms: 75,
        });
        scene.mask_transition = Some(CvnsMaskTransition {
            sprite_slot: 7,
            reveal: true,
            duration_ms: 400,
            ramp_ms: 40,
            elapsed_ms: 20,
            width: 2,
            height: 2,
            mask: vec![10, 20, 30, 40],
            thresholds: CvnsMaskTransition::compute_thresholds(400),
        });
        scene.quake = Some(CvnsQuake::from_saved(
            2,
            12,
            33,
            5,
            1,
            true,
            17,
            2,
            -4,
            3,
            0xdead_beef,
        ));
        scene.cg_seen.insert(b"ev_cg01".to_vec());

        let mut msg_host = CvnsMessageHost {
            text_shadow: true,
            read_skip: false,
            auto_skip: true,
            auto_wait_elapsed_ms: 120,
            click_wait_elapsed_ms: 45,
            ..Default::default()
        };
        msg_host.windows[0] = Some(CvnsWindow {
            position: [12, 24],
            size: [640, 160],
            area: [16, 8, 600, 140],
            cursor: [32, 8],
            glyph_size: 26,
            spacing: [2, 4],
            face: b"TestGothic".to_vec(),
            colors: [0x00ff_ff00, 0x0000_00ff],
            gradient_dir: 1,
            char_delay_ms: 25,
            reveal: [10, 5],
            kinsoku: false,
            visible: true,
            z: 4,
            ruby: true,
            ruby_size: 13,
            ruby_spacing: 1,
            ruby_always: true,
        });
        let mut local_images = vec![None; CVNS_LOCAL_IMAGES];
        local_images[0] = Some(CvnsImageLayer::from_image(
            b"asset_mask".to_vec(),
            Vec::new(),
            crate::image::load(&mut src, b"asset_mask")
                .unwrap()
                .unwrap()
                .pixels,
            0,
            0,
            true,
        ));
        let mut local_sprites = vec![None; CVNS_LOCAL_SPRITES];
        local_sprites[0] = Some(CvnsSprite {
            kind: CvnsSpriteKind::Image,
            image_slot: 0,
            visible: true,
            ..Default::default()
        });
        let mut buttons = [CvnsInlineButton::default(); CVNS_INLINE_BUTTONS];
        buttons[0].enabled = true;
        buttons[0].sprite_slot = 0;
        buttons[0].hit = [1, 2, 30, 40];
        buttons[0].states[0] = CvnsButtonState {
            image_slot: 0,
            sx: 0,
            sy: 0,
            sw: 4,
            sh: 2,
            dx: 1,
            dy: 2,
        };
        msg_host.messages[0] = Some(CvnsMessage::from_saved(
            crate::message::cvns::RestoredCvnsMessage {
                win_slot: 0,
                x: 5,
                y: 6,
                w: 400,
                h: 120,
                visible: true,
                paused: true,
                wait_cursor_mode: 2,
                wait_cursor_offset: [3, -4],
                attached_flags: [false; CVNS_LOCAL_SPRITES],
                attached_offsets: [[0; 2]; CVNS_LOCAL_SPRITES],
                local_images,
                local_sprites,
                buttons,
            },
        ));
        let texts_in: Vec<Option<TextObject>> = vec![Some(TextObject {
            font_face: b"TestGothic".to_vec(),
            face: Arc::from(b"TestGothic".as_slice()),
            bold: false,
            revision_alpha: false,
            scale_bits: 1.0f32.to_bits(),
            logical_size: [800, 600],
            position: [12, 24],
            area: [16, 8, 600, 140],
            cursor: [32, 8],
            ruby_lines: [0; 32],
            layout_scratch: [0; 2],
            glyphs: Vec::new(),
            glyph_size: 26,
            spacing: [2, 4],
            color: 0x00ff_ff00,
            decoration_color: 0x0000_00ff,
            outline_mode: 2,
            glyph_parameter: 0,
            decoration: 1,
            reveal: [10, 5],
            char_delay: 25,
            layer: 0,
            visible: true,
            ruby: true,
            ruby_every_line: true,
            ruby_size: 13,
            ruby_spacing: 1,
            line_break: None,
            glyph_capacity: 4096,
        })];

        let mut audio = AudioHost::default();
        audio.enable_cvns();
        let mut rec = RecordingAudio::default();
        for (kind, val) in [(0, 80), (1, 90), (2, 70)] {
            let (pct, settings) = audio.stage_cvns_volume(kind, val, &mut rec).unwrap();
            audio.commit_cvns_volume(kind, pct, settings);
        }
        let mut state = empty_state();
        let starts = PersistentStarts {
            bit: 16,
            integer: 4,
            string: 2,
        };
        state.set_integer(1, 55).unwrap();
        state.set_string(1, b"chapter1").unwrap();
        let vars = CvnsLocalVariables::capture(&state, starts).unwrap();

        let snap = CvnsSnapshot::from_scene_and_messages(
            CvnsSceneCapture {
                scene: &scene,
                msg_host: Some(&msg_host),
                texts: &texts_in,
                clear_color: 0x0102_0304,
                clear_depth: true,
                backdrop_disabled: true,
                layer_fade_alpha: Some(128),
            },
            CvnsVmCapture {
                save_title: b"Title".to_vec(),
                scene_title: b"Scene".to_vec(),
                script_names: [b"a.ps3\0".to_vec(), Vec::new(), Vec::new(), Vec::new()],
                script_sizes: [12, 0, 0, 0],
                active_slot: 0,
                pc: 0x120,
                repeat_count: 3,
                timers: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                popup: 1,
                callbacks: vec![1; 1796],
                stack_pointer: 16,
                frame_depth: 1,
                frames: [0; 64],
                stack: vec![0xaa; 0x10000],
                system_results: [9; 11],
                script_data: vec![0x55; 32],
                variables: vars,
                audio: audio.saved_cvns(),
                history: vec![7, 8, 9],
            },
        )
        .unwrap();

        let encoded = snap.encode().unwrap();
        let decoded = CvnsSnapshot::decode(&encoded).unwrap();
        assert_eq!(decoded.save_title, b"Title");
        assert_eq!(decoded.scene_title, b"Scene");
        assert_eq!(decoded.pc, 0x120);
        assert_eq!(decoded.clear_color, 0x0102_0304);
        assert!(decoded.clear_depth);
        assert!(decoded.backdrop_disabled);
        assert_eq!(decoded.layer_fade_alpha, Some(128));
        assert!(decoded.cg_seen.contains(b"ev_cg01".as_slice()));
        assert_eq!(decoded.quake.unwrap().rng(), 0xdead_beef);
        assert!(decoded.sprites[&7].move_tween.unwrap().active);
        assert_eq!(decoded.sprites[&7].curtain.as_ref().unwrap().tiles.len(), 1);
        assert_eq!(decoded.windows[&0].face, b"TestGothic");
        assert_eq!(decoded.audio, audio.saved_cvns());
        assert_eq!(decoded.history, vec![7, 8, 9]);

        let mut budget = crate::MAX_PAYLOAD;
        let restored_buf = decoded.images[&0].load(&mut src, &mut budget).unwrap();
        assert!(restored_buf.composited.is_some());
        assert!(restored_buf.layers[2].as_ref().unwrap().captured);
        let restored_mask_tr = decoded
            .mask_transition
            .as_ref()
            .unwrap()
            .restore(&mut budget)
            .unwrap();
        assert_eq!(restored_mask_tr.mask, vec![10, 20, 30, 40]);
        let restored_msg = decoded.messages[&0].load(&mut src, &mut budget).unwrap();
        assert!(restored_msg.visible);
        assert_eq!(restored_msg.buttons[0].sprite_slot, 0);

        let mut tight = 4;
        assert!(decoded.images[&0].load(&mut src, &mut tight).is_err());

        // Check SavedCvnsAudio decoding with seeded BGM/SE/Aux payload.
        let mut raw_audio = Vec::new();
        for name in [b"bgm01.ogg".as_slice(), b"bgm02.ogg".as_slice()] {
            raw_audio.extend_from_slice(&(name.len() as u32).to_le_bytes());
            raw_audio.extend_from_slice(name);
        }
        raw_audio.extend_from_slice(&44100u32.to_le_bytes());
        for i in 0..crate::audio::CVNS_SE_CHANNELS {
            if i == 3 {
                raw_audio.push(1);
                raw_audio.extend_from_slice(&(b"se03.ogg".len() as u32).to_le_bytes());
                raw_audio.extend_from_slice(b"se03.ogg");
                raw_audio.push(1);
                raw_audio.extend_from_slice(&60i32.to_le_bytes());
                raw_audio.push(1);
            } else {
                raw_audio.push(0);
            }
            raw_audio.extend_from_slice(&(if i == 3 { 50u32 } else { 100 }).to_le_bytes());
            raw_audio.extend_from_slice(&(if i == 3 { -25i32 } else { 0 }).to_le_bytes());
        }
        for i in 0..crate::audio::AUX_SE_CHANNELS {
            if i == 1 {
                raw_audio.push(1);
                raw_audio.extend_from_slice(&(b"ax.ogg".len() as u32).to_le_bytes());
                raw_audio.extend_from_slice(b"ax.ogg");
                raw_audio.push(1);
                raw_audio.extend_from_slice(&75i32.to_le_bytes());
                raw_audio.push(0);
            } else {
                raw_audio.push(0);
            }
        }
        let saved_audio = SavedCvnsAudio::decode(&raw_audio).unwrap();
        audio.restore_cvns(saved_audio, &mut rec).unwrap();
        assert!(rec.requests().iter().any(|r| matches!(
            r,
            AudioRequest::PlayBgmLoopFrom { primary, loop_sample: 44100, .. }
                if primary == b"bgm01.ogg"
        )));
        assert!(rec.requests().iter().any(|r| matches!(
            r,
            AudioRequest::PlaySe { channel: 3, name, .. } if name == b"se03.ogg"
        )));
        assert!(rec.requests().iter().any(|r| matches!(
            r,
            AudioRequest::PlayAuxSe { channel: 1, name, .. } if name == b"ax.ogg"
        )));

        assert!(CvnsSnapshot::has_cmvs_rs_records(&encoded));
        let mut wrong_version = encoded.clone();
        wrong_version[6..10].copy_from_slice(&(CVNS_SNAPSHOT_SCHEMA_VERSION + 1).to_le_bytes());
        assert_eq!(
            CvnsSnapshot::decode(&wrong_version).unwrap_err(),
            FormatError("unsupported CVNS snapshot VM block version")
        );

        // Native cvns.exe CSP1 snapshots start with 0x100 short records and lack 0xa00.
        let native_payload = [0x00, 0x01, 0x02, 0x00, 0xaa, 0xbb];
        assert!(!CvnsSnapshot::has_cmvs_rs_records(&native_payload));
        assert_eq!(
            CvnsSnapshot::decode(&native_payload).unwrap_err(),
            FormatError("native cvns.exe CSP1 snapshot payload is not supported")
        );

        let mut bad = encoded.clone();
        bad[0] ^= 0xff;
        assert!(CvnsSnapshot::decode(&bad).is_err());
    }
}

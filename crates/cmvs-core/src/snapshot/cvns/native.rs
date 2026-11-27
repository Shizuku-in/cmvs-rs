//! Original CSP1 records for the cvns-20090203 build (writer 0x44ebf0, reader 0x451e60).
//! Convert the on-disk records to the existing CVNS runtime snapshot model.

use super::{
    CVNS_IMAGES, CVNS_INLINE_BUTTONS, CVNS_LAYERS, CVNS_LOCAL_IMAGES, CVNS_LOCAL_SPRITES,
    CVNS_MESSAGES, CVNS_SPRITES, CVNS_WINDOWS, CvnsButtonState, CvnsInlineButton,
    CvnsLocalVariables, CvnsSnapshot, CvnsSprite, CvnsWindow, SavedCvnsImageBuffer, SavedCvnsLayer,
    SavedCvnsMessage, SavedLayerPixels, SavedMaskSource, insert_unique, put_bytes,
};
use crate::{
    FormatError,
    audio::{AUX_SE_CHANNELS, CVNS_SE_CHANNELS},
    history::{HISTORY_CAPACITY, MAX_HISTORY_TEXT},
    message::{RecordedGlyph, TextObject, cvns::CvnsMessageHost},
    save::Csv2Save,
    scene::cvns::{CVNS_ANIM_FRAMES, CVNS_PLAYLISTS, CvnsAnimFrame, CvnsSpriteAnim},
    script_control::CALLBACK_RECORD,
    snapshot::{MAX_SNAPSHOT, visual::Reader},
    system_save::PersistentStarts,
};
use std::collections::BTreeMap;

pub(crate) fn decode(
    save: &Csv2Save,
    starts: PersistentStarts,
    message_settings: &CvnsMessageHost,
    logical_size: [i32; 2],
) -> Result<CvnsSnapshot, FormatError> {
    if save.snapshot.len() > MAX_SNAPSHOT {
        return Err(FormatError(
            "original CVNS snapshot exceeds the 3 MiB buffer",
        ));
    }
    let mut r = Reader::new(&save.snapshot);
    let mut records = BTreeMap::new();
    let mut images = BTreeMap::new();
    let mut sprites = BTreeMap::new();
    let mut windows = BTreeMap::new();
    let mut texts = BTreeMap::new();
    let mut messages: BTreeMap<u16, SavedCvnsMessage> = BTreeMap::new();
    while !r.is_empty() {
        let tag = r.u16()?;
        let slots = match tag {
            0x400 | 0x481 | 0x4c1 => CVNS_MESSAGES,
            0x480 => CVNS_IMAGES,
            0x4c0 => CVNS_SPRITES,
            0x500 => CVNS_WINDOWS,
            _ => 0,
        };
        let index = if slots == 0 { 0 } else { slot(&mut r, slots)? };
        let local_index = match tag {
            0x481 => slot(&mut r, CVNS_LOCAL_IMAGES)?,
            0x4c1 => slot(&mut r, CVNS_LOCAL_SPRITES)?,
            _ => 0,
        };
        let len = match tag {
            0xff | 0x100..=0x10b | 0x200..=0x204 | 0x280 | 0x281 | 0x288 | 0x300 => {
                usize::from(r.u16()?)
            }
            0x282
            | 0x283
            | 0x2a0..=0x2a2
            | 0x301
            | 0x400
            | 0x480
            | 0x481
            | 0x4c0
            | 0x4c1
            | 0x500
            | 0x540
            | 0x900 => r.u32()? as usize,
            // BGM loop sample has a fixed four-byte body and no length.
            0x310 => 4,
            _ => return Err(FormatError("unsupported original CVNS snapshot tag")),
        };
        let body = r.take(len)?;
        match tag {
            0x400 => insert_unique(&mut messages, index, message(body)?)?,
            0x480 => insert_unique(&mut images, index, image_buffer(body)?)?,
            0x481 => {
                let buffer = image_buffer(body)?;
                let mut layers = buffer.layers.into_iter();
                let base = layers
                    .next()
                    .flatten()
                    .ok_or(FormatError("CVNS message image has no base layer"))?;
                if layers.any(|layer| layer.is_some()) || buffer.mask.is_some() {
                    return Err(FormatError("layered CVNS message image is unsupported"));
                }
                let msg = messages
                    .get_mut(&index)
                    .ok_or(FormatError("CVNS local object precedes its message"))?;
                put_slot(&mut msg.local_images[usize::from(local_index)], base)?;
            }
            0x4c0 => insert_unique(&mut sprites, index, sprite(body, CVNS_IMAGES)?)?,
            0x4c1 => {
                let msg = messages
                    .get_mut(&index)
                    .ok_or(FormatError("CVNS local object precedes its message"))?;
                put_slot(
                    &mut msg.local_sprites[usize::from(local_index)],
                    sprite(body, CVNS_LOCAL_IMAGES)?,
                )?;
            }
            0x500 => {
                let (window, text) = window(body, logical_size)?;
                insert_unique(&mut windows, index, window)?;
                insert_unique(&mut texts, index, text)?;
            }
            _ => insert_unique(&mut records, tag, body)?,
        }
    }
    for msg in messages.values() {
        let key = msg.win_slot as u16;
        let text = texts
            .get_mut(&key)
            .ok_or(FormatError("CVNS message has no paired text window"))?;
        text.visible = windows[&key].visible && !msg.paused;
    }
    let required = |tag| {
        records
            .get(&tag)
            .copied()
            .ok_or(FormatError("missing original CVNS snapshot record"))
    };
    let number = |tag| Ok::<_, FormatError>(words::<1>(required(tag)?)?[0]);
    let title = |tag, limit| {
        let mut r = Reader::new(required(tag)?);
        let value = r.string(limit)?.to_vec();
        r.finish()?;
        Ok::<_, FormatError>(value)
    };
    let stack = required(0x283)?;
    let callbacks = required(0x10b)?;
    let main_len = number(0x103)?;
    if stack.len() != 65536
        || callbacks.len() != CALLBACK_RECORD
        || main_len as usize != save.script_image.len()
        || !save.script_image.starts_with(b"PSS0")
    {
        return Err(FormatError("invalid original CVNS VM size"));
    }
    let mut system_results = [0; 11];
    system_results[..7].copy_from_slice(&words::<7>(required(0x288)?)?);
    let variables = local_variables(starts, required(0x2a0)?, required(0x2a1)?, required(0x2a2)?)?;
    // 0x107 is opcode scratch space. 0x10a and 0x203 hold native engine state
    // that the Rust VM and renderer do not expose.
    words::<16>(required(0x107)?)?;
    number(0x10a)?;
    number(0x203)?;
    Ok(CvnsSnapshot {
        save_title: title(0x100, 256)?,
        scene_title: title(0xff, 256)?,
        script_names: [title(0x101, 32)?, title(0x102, 32)?, Vec::new(), Vec::new()],
        script_sizes: [main_len, number(0x104)?, 0, 0],
        active_slot: number(0x105)?,
        pc: number(0x106)?,
        repeat_count: number(0x108)?,
        timers: words(required(0x109)?)?,
        popup: number(0x204)?,
        callbacks: callbacks.to_vec(),
        stack_pointer: number(0x280)?,
        frame_depth: number(0x281)?,
        frames: words(required(0x282)?)?,
        stack: stack.to_vec(),
        system_results,
        script_data: required(0x900)?.to_vec(),
        variables,
        clear_color: number(0x200)?,
        clear_depth: false,
        backdrop_disabled: number(0x202)? == 0,
        layer_fade_alpha: Some(number(0x201)?.min(256) as u16),
        fade: None,
        mask_transition: None,
        quake: None,
        cg_seen: Default::default(),
        images,
        sprites,
        text_shadow: message_settings.text_shadow,
        read_skip: message_settings.read_skip,
        auto_skip: message_settings.auto_skip,
        auto_wait_elapsed_ms: 0,
        click_wait_elapsed_ms: 0,
        windows,
        texts,
        messages,
        audio: audio(&title(0x300, 64)?, number(0x310)?, required(0x301)?)?,
        history: history(required(0x540)?)?,
    })
}

fn slot(r: &mut Reader<'_>, limit: usize) -> Result<u16, FormatError> {
    let index = r.u16()?;
    if usize::from(index) < limit {
        Ok(index)
    } else {
        Err(FormatError("original CVNS snapshot slot out of range"))
    }
}

fn put_slot<T>(target: &mut Option<T>, value: T) -> Result<(), FormatError> {
    if target.is_some() {
        return Err(FormatError("duplicate original CVNS snapshot slot"));
    }
    *target = Some(value);
    Ok(())
}

fn words<const N: usize>(bytes: &[u8]) -> Result<[u32; N], FormatError> {
    let mut r = Reader::new(bytes);
    let result = r.words()?;
    r.finish()?;
    Ok(result)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn i16_at(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn local_variables(
    starts: PersistentStarts,
    integers: &[u8],
    bits: &[u8],
    strings: &[u8],
) -> Result<CvnsLocalVariables, FormatError> {
    starts.checked()?;
    if integers.len() != starts.integer as usize * 4 || bits.len() != (starts.bit >> 3) as usize {
        return Err(FormatError(
            "original CVNS local variable counts differ from the game",
        ));
    }
    let mut r = Reader::new(strings);
    let strings = (0..starts.string)
        .map(|_| Ok(r.string(1024)?.to_vec()))
        .collect::<Result<Vec<_>, FormatError>>()?;
    r.finish()?;
    Ok(CvnsLocalVariables {
        starts,
        integers: integers
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&word| i32::from_le_bytes(word))
            .collect(),
        bits: bits.to_vec(),
        strings,
    })
}

fn image_buffer(bytes: &[u8]) -> Result<SavedCvnsImageBuffer, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 0xfffd {
        return Err(FormatError(
            "unsupported original CVNS image buffer version",
        ));
    }
    let count = usize::from(r.u16()?);
    if count > CVNS_LAYERS {
        return Err(FormatError("invalid original CVNS image layer count"));
    }
    let mut layers = vec![None; CVNS_LAYERS];
    for _ in 0..count {
        let index = usize::from(slot(&mut r, CVNS_LAYERS)?);
        let name = fixed_string(r.take(64)?)?;
        let part = fixed_string(r.take(32)?)?;
        let [
            width,
            height,
            alpha,
            frame,
            dx,
            dy,
            monochrome,
            filter_mode,
            filter_arg,
            hsl_from,
            hsl_to,
            rgb_mul,
        ] = r.words()?;
        let source = if name.is_empty() {
            SavedLayerPixels::BlankFill {
                width,
                height,
                color: 0,
            }
        } else {
            SavedLayerPixels::Asset {
                name: name.to_vec(),
                part: part.to_vec(),
                frame: (part.is_empty() && frame != 0).then_some(frame),
            }
        };
        put_slot(
            &mut layers[index],
            SavedCvnsLayer {
                source,
                use_alpha: alpha != 0,
                dx: dx as i32,
                dy: dy as i32,
                filter_mode,
                filter_arg: filter_arg as i32,
                monochrome,
                hsl_from: hsl_from as i32,
                hsl_to: hsl_to as i32,
                rgb_mul: rgb_mul as i32,
            },
        )?;
    }
    let mask = fixed_string(r.take(64)?)?;
    r.finish()?;
    Ok(SavedCvnsImageBuffer {
        layers,
        has_composite: count >= 2,
        mask: (!mask.is_empty()).then(|| SavedMaskSource::Asset(mask.to_vec())),
    })
}

fn fixed_string(bytes: &[u8]) -> Result<&[u8], FormatError> {
    Reader::new(bytes).string(bytes.len())
}

fn history(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut r = Reader::new(bytes);
    let [version, capacity, count] = r.words()?;
    if version != 0xffff_0001 || capacity != HISTORY_CAPACITY as u32 || count > capacity {
        return Err(FormatError("unsupported original CVNS history layout"));
    }
    let mut out = Vec::new();
    for value in [0xffff_0000, capacity, count] {
        out.extend(value.to_le_bytes());
    }
    for _ in 0..count {
        for limit in [32, 32, MAX_HISTORY_TEXT + 1] {
            out.extend(r.string(limit)?);
            out.push(0);
        }
        out.push(0); // fourth string is only present in CMVS
        out.extend(r.take(16)?);
        out.extend([0; 8]); // CVNS has four history words
    }
    r.finish()?;
    Ok(out)
}

fn audio(bgm: &[u8], loop_sample: u32, se: &[u8]) -> Result<Vec<u8>, FormatError> {
    // The original reader restores 48 SE names from the 0x1110-byte bank;
    // the other four name entries and auxiliary effects are not read on load.
    if se.len() != 0x1110 {
        return Err(FormatError("invalid original CVNS audio bank size"));
    }
    let mut out = Vec::new();
    put_bytes(&mut out, bgm);
    put_bytes(&mut out, &[]);
    out.extend(loop_sample.to_le_bytes());
    for index in 0..CVNS_SE_CHANNELS {
        let name = fixed_string(&se[index * 64..(index + 1) * 64])?;
        let word = |offset| u32_at(se, offset + 4 * index);
        if name.is_empty() {
            out.push(0);
        } else {
            out.push(1);
            put_bytes(&mut out, name);
            let looped = word(3328) != 0;
            let volume = word(3536);
            out.push(u8::from(looped));
            out.extend(if volume == 0 { 100 } else { volume.min(100) }.to_le_bytes());
            out.push(u8::from(looped && word(4160) != 0));
        }
        out.extend(word(3744).min(100).to_le_bytes());
        out.extend(word(3952).to_le_bytes());
    }
    out.extend([0; AUX_SE_CHANNELS]);
    Ok(out)
}

fn window(bytes: &[u8], logical_size: [i32; 2]) -> Result<(CvnsWindow, TextObject), FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 0xffff || r.u16()? != 1 {
        return Err(FormatError("unsupported original CVNS text window version"));
    }
    let glyph_size = i32::from(r.u16()?);
    let _font_quality = r.u16()?;
    let face = r.string(64)?.to_vec();
    let raw = r.take(160)?;
    let glyph_count = usize::from(r.u16()?);
    if glyph_count > 4096 || !(1..=128).contains(&glyph_size) {
        return Err(FormatError("invalid original CVNS glyph atlas fields"));
    }
    let saved_glyphs = r.take(glyph_count * 124)?.as_chunks::<124>().0;
    let ruby = r.take(148)?;
    r.finish()?;
    let field = |offset| u32_at(raw, offset) as i32;
    let win = CvnsWindow {
        position: [field(8), field(12)],
        size: [field(0), field(4)],
        area: [field(16), field(20), field(24), field(28)],
        // The original redraws saved glyphs after resetting the pen.
        cursor: [field(16), field(20)],
        glyph_size: field(32),
        spacing: [field(36), field(40)],
        face,
        colors: [field(112) as u32, field(116) as u32],
        gradient_dir: field(120) as u32,
        char_delay_ms: field(140),
        reveal: [field(128), field(136)],
        kinsoku: field(108) != 0,
        visible: field(148) != 0,
        z: field(148),
        ruby: u32_at(ruby, 0) != 0,
        ruby_size: u32_at(ruby, 12) as i32,
        ruby_spacing: u32_at(ruby, 16) as i32,
        ruby_always: u32_at(ruby, 8) != 0,
    };
    if !(1..=128).contains(&win.glyph_size) {
        return Err(FormatError("invalid original CVNS glyph size"));
    }
    let mut text = win.new_text(&[], logical_size);
    for glyph in saved_glyphs {
        // 0x442c20 redraws from the 44-byte descriptor after the native sprite.
        let descriptor = &glyph[80..];
        let [code, gradient, x, y, size, top, bottom, _, shadow, _, alpha] =
            words::<11>(descriptor)?;
        if !(1..=128).contains(&size) {
            return Err(FormatError("invalid original CVNS saved glyph size"));
        }
        text.glyphs.push(RecordedGlyph {
            snapshot_indices: None,
            face: text.face.clone(),
            bold: false,
            revision_alpha: false,
            code: (code >> 16) as u16,
            position: [x as i32, y as i32],
            size,
            colors: [top, bottom],
            gradient: gradient & 3,
            outline: [u32::from(shadow != 0), 0],
            delay_ms: 0,
            elapsed_ms: 0,
            duration_ms: 0,
            active: false,
            shown: true,
            opacity: alpha.min(255),
            reveal_mode: 0,
            reveal_width: size + 4,
        });
    }
    Ok((win, text))
}

fn message(bytes: &[u8]) -> Result<SavedCvnsMessage, FormatError> {
    let mut r = Reader::new(bytes);
    let win_slot = usize::from(slot(&mut r, CVNS_WINDOWS)?);
    if r.u16()? != 0xfff0 {
        return Err(FormatError("unsupported original CVNS message version"));
    }
    let [window, visible, paused, _, x, y, w, h] = r.words()?;
    if window as usize != win_slot {
        return Err(FormatError("original CVNS message window slots disagree"));
    }
    let wait_cursor_mode = r.u16()?;
    let wait_cursor_offset = [r.u16()? as i16, r.u16()? as i16];
    r.take(2)?;
    let mut buttons = [CvnsInlineButton::default(); CVNS_INLINE_BUTTONS];
    for button in &mut buttons {
        *button = inline_button(r.take(128)?)?;
    }
    r.take(4)?;
    if usize::from(r.u16()?) != CVNS_LOCAL_SPRITES {
        return Err(FormatError("invalid original CVNS attached sprite count"));
    }
    let mut attached_flags = [false; CVNS_LOCAL_SPRITES];
    let mut attached_offsets = [[0; 2]; CVNS_LOCAL_SPRITES];
    for (flag, offsets) in attached_flags.iter_mut().zip(&mut attached_offsets) {
        *flag = r.u32()? != 0;
        *offsets = [r.u16()? as i16, r.u16()? as i16];
    }
    r.finish()?;
    Ok(SavedCvnsMessage {
        win_slot,
        x: x as i32,
        y: y as i32,
        w: w as i32,
        h: h as i32,
        visible: visible != 0,
        paused: paused != 0,
        wait_cursor_mode,
        wait_cursor_offset,
        attached_flags,
        attached_offsets,
        local_images: vec![None; CVNS_LOCAL_IMAGES],
        local_sprites: vec![None; CVNS_LOCAL_SPRITES],
        buttons,
    })
}

fn inline_button(bytes: &[u8]) -> Result<CvnsInlineButton, FormatError> {
    let short = |offset| i16_at(bytes, offset);
    let sprite_slot = short(116) as u16;
    if usize::from(sprite_slot) >= CVNS_LOCAL_SPRITES {
        return Err(FormatError("invalid original CVNS button sprite slot"));
    }
    let x = i32::from(short(118));
    let y = i32::from(short(120));
    let w = i32::from(short(122));
    let h = i32::from(short(124));
    Ok(CvnsInlineButton {
        enabled: u32_at(bytes, 4) != 0,
        sprite_slot,
        cur_state: short(0) as u16,
        prev_state: short(2) as u16,
        hit: [x, y, w, h],
        states: std::array::from_fn(|i| {
            let at = 8 + i * 18;
            CvnsButtonState {
                image_slot: short(at),
                sx: short(at + 2),
                sy: short(at + 4),
                sw: short(at + 6),
                sh: short(at + 8),
                dx: short(at + 10),
                dy: short(at + 12),
            }
        }),
    })
}

fn sprite(bytes: &[u8], image_slots: usize) -> Result<CvnsSprite, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 0xffff {
        return Err(FormatError("unsupported original CVNS sprite version"));
    }
    // The writer copies the sprite's scalar fields and animation table. Raw
    // pointers in this memory block have no meaning in the restoring process.
    let raw = r.take(1336)?;
    if r.u16()? != 0 {
        return Err(FormatError("original CVNS sprite curtain is unsupported"));
    }
    let tween = r.take(80)?;
    r.finish()?;
    if u32_at(tween, 0) != 0 {
        return Err(FormatError("original CVNS sprite tween is unsupported"));
    }
    let field = |offset| u32_at(raw, offset) as i32;
    let short = |offset| i16_at(raw, offset);
    let image_slot = short(42);
    if image_slot < -1 || image_slot >= image_slots as i16 {
        return Err(FormatError("invalid original CVNS sprite image slot"));
    }
    let alpha = short(28) as u16;
    if alpha > 256 {
        return Err(FormatError("invalid original CVNS sprite alpha"));
    }
    let frames = raw[184..1336]
        .as_chunks::<18>()
        .0
        .iter()
        .map(|frame| CvnsAnimFrame {
            image_slot: i16_at(frame, 0),
            sx: i16_at(frame, 2),
            sy: i16_at(frame, 4),
            sw: i16_at(frame, 6),
            sh: i16_at(frame, 8),
            ox: i16_at(frame, 10),
            oy: i16_at(frame, 12),
            duration_ms: i16_at(frame, 14),
            next_frame: i16_at(frame, 16),
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(frames.len(), CVNS_ANIM_FRAMES);
    let anim = if field(160) == 0 {
        None
    } else {
        let track = short(164);
        let current_frame = short(166);
        if !(0..CVNS_PLAYLISTS as i16).contains(&track)
            || !(0..CVNS_ANIM_FRAMES as i16).contains(&current_frame)
        {
            return Err(FormatError("invalid original CVNS sprite animation"));
        }
        Some(CvnsSpriteAnim {
            track,
            current_frame,
            remaining_ms: field(168).max(0) as u32,
        })
    };
    Ok(CvnsSprite {
        visible: field(80) != 0,
        image_slot,
        src_rect: [44, 46, 48, 50].map(|offset| i32::from(short(offset))),
        x: field(88),
        y: field(92),
        w: field(96),
        h: field(100),
        origin_x: field(104),
        origin_y: field(108),
        scale_x: field(112),
        scale_y: field(116),
        rotation: field(120),
        z: field(124),
        blend_mode: short(20),
        color_strength: short(22),
        color: field(24) as u32,
        alpha,
        flags: field(12) as u32,
        mask_mode: short(56),
        anim,
        playlists: std::array::from_fn(|i| short(172 + i * 2)),
        frames,
        ..CvnsSprite::default()
    })
}

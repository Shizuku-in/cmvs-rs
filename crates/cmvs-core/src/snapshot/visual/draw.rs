//! Draw state chunk 0x780 (writer 0x14001eef0, reader 0x14001f080), its 0x793 frame
//! animation and motion tracks. The decoder keeps the saved +84 fade factor; the loader's
//! reset to 256 is applied when hosts are rebuilt.
//!
//! Tracks follow the fields in writer order: 0x790 scale +112 (0x140024b50 / 0x140024ba0),
//! 0x791 rotation +120, 0x792 opacity +128 (0x1400267f0 / 0x140026840), 0x793 frames +136,
//! 0x794 XY +144 (0x140024f30 / 0x140024f80) and 0x795 XYZ +152 (0x140025410 /
//! 0x140025460). Each body is u16 version 1 and the track object's memory image, which the
//! reader copies back unchecked; Rust checks what the setters guarantee instead. Header
//! words a track that never started leaves uninitialized are written as zero.
use super::Reader;
use crate::{
    FormatError,
    scene::{
        Animation, AnimationFrame, DrawState, MotionKey, MotionPlayback, OPACITY_KEYS, OpacityKey,
        OpacityTrack, POSITION_KEYS, PositionKey, PositionMotion, PositionPlayback, ROTATION_KEYS,
        RotationKey, RotationTrack, SCALE_KEYS, ScaleKey, ScaleMotion, ScalePlayback,
        WORLD_MOTION_KEYS, WorldMotion,
    },
};

/// Object sizes: 0x140024b00, 0x1400267a0, 0x140024f00 and 0x1400253e0 allocations.
const SCALE_SIZE: usize = 0xb8;
const OPACITY_SIZE: usize = 0x94;
const POSITION_SIZE: usize = 0x114;
const WORLD_SIZE: usize = 0xdc;

pub(super) fn decode(bytes: &[u8]) -> Result<DrawState, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 4 || r.u16()? != 0 {
        return Err(FormatError("unsupported snapshot draw version"));
    }
    let words = r.words::<22>()?;
    let extra = r.words::<6>()?;
    if extra[1..4] != [0; 3] {
        return Err(FormatError("unsupported snapshot draw extra fields"));
    }
    let mut out = DrawState {
        words,
        texture_mode: extra[0],
        texture_subtract: extra[4],
        depth_bias: extra[5],
        ..DrawState::default()
    };
    if r.peek() == Some(0x790) {
        r.u16()?;
        out.scale_motion = Some(Box::new(scale(r.chunk()?)?));
    }
    if r.peek() == Some(0x791) {
        r.u16()?;
        out.rotation_track = Some(Box::new(rotation(r.chunk()?)?));
    }
    if r.peek() == Some(0x792) {
        r.u16()?;
        out.opacity_track = Some(Box::new(opacity(r.chunk()?)?));
    }
    if r.peek() == Some(0x793) {
        r.u16()?;
        out.animation = Some(Box::new(animation(r.chunk()?)?));
    }
    if r.peek() == Some(0x794) {
        r.u16()?;
        out.position_motion = Some(Box::new(position(r.chunk()?)?));
    }
    if r.peek() == Some(0x795) {
        r.u16()?;
        out.world_motion = Some(Box::new(world(r.chunk()?)?));
    }
    if r.peek().is_some_and(|tag| (0x790..=0x795).contains(&tag)) {
        return Err(FormatError(
            "snapshot draw track is out of order or duplicated",
        ));
    }
    if r.u16()? != 0x7789 {
        return Err(FormatError("invalid snapshot draw terminator"));
    }
    r.finish()?;
    Ok(out)
}

/// The loader 0x14001f080 discards the captured transient fade factor (+84).
pub(super) fn reset_transients(draw: &mut DrawState) {
    draw.words[21] = 256;
}

/// Inverse of `decode`.
pub(crate) fn encode(draw: &DrawState) -> Result<Vec<u8>, FormatError> {
    let mut out = 4u16.to_le_bytes().to_vec();
    out.extend(0u16.to_le_bytes());
    for word in draw.words {
        out.extend(word.to_le_bytes());
    }
    for word in [
        draw.texture_mode,
        0,
        0,
        0,
        draw.texture_subtract,
        draw.depth_bias,
    ] {
        out.extend(word.to_le_bytes());
    }
    if let Some(scale) = &draw.scale_motion {
        let p = scale.playback.as_ref();
        let mut words = vec![
            u32::from(p.is_some()),
            scale.base[0],
            scale.base[1],
            p.map_or(0, |p| p.start),
            p.map_or(0, |p| p.current),
            p.map_or(0, |p| p.elapsed),
        ];
        for k in &scale.keys {
            words.extend([k.next as u32, k.xy[0], k.xy[1], k.mode, k.duration]);
        }
        put_track(&mut out, 0x790, &words);
    }
    if let Some(rotation) = &draw.rotation_track {
        let mut words = vec![
            u32::from(rotation.playing),
            rotation.base as u32,
            rotation.start,
            rotation.current,
            rotation.elapsed,
        ];
        for k in &rotation.keys {
            words.extend([k.next as u32, k.value as u32, k.mode, k.duration]);
        }
        put_track(&mut out, 0x791, &words);
    }
    if let Some(opacity) = &draw.opacity_track {
        let mut words = vec![
            u32::from(opacity.playing),
            opacity.base as u32,
            opacity.start,
            opacity.current,
            opacity.elapsed,
        ];
        for k in &opacity.keys {
            words.extend([k.next as u32, k.value as u32, k.hold, k.duration]);
        }
        put_track(&mut out, 0x792, &words);
    }
    if let Some(animation) = &draw.animation {
        let mut body = 2u16.to_le_bytes().to_vec();
        for word in [
            u32::from(animation.playing),
            animation.start,
            animation.current,
            animation.elapsed,
        ] {
            body.extend(word.to_le_bytes());
        }
        for frame in &animation.frames {
            body.extend(frame.next.to_le_bytes());
            for word in frame.words {
                body.extend(word.to_le_bytes());
            }
            body.extend(frame.duration.to_le_bytes());
        }
        out.extend(0x793u16.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
    }
    let packed = |xy: [i16; 2]| u32::from(xy[0] as u16) | (u32::from(xy[1] as u16) << 16);
    if let Some(position) = &draw.position_motion {
        let p = position.playback.as_ref();
        let mut words = vec![
            u32::from(p.is_some()),
            p.map_or(0, |p| packed(p.base)),
            p.map_or(0, |p| p.start),
            p.map_or(0, |p| p.current),
            p.map_or(0, |p| p.elapsed),
        ];
        for k in &position.keys {
            words.extend([k.next as u32, packed(k.xy), k.mode, k.duration]);
        }
        put_track(&mut out, 0x794, &words);
    }
    if let Some(world) = &draw.world_motion {
        let p = world.playback.as_ref();
        let base = p.map_or([0; 3], |p| p.base);
        let mut words = vec![u32::from(p.is_some()), base[0], base[1], base[2]];
        words.extend([
            p.map_or(0, |p| p.start),
            p.map_or(0, |p| p.current),
            p.map_or(0, |p| p.elapsed),
        ]);
        for k in &world.keys {
            words.extend([
                k.next as u32,
                k.xyz[0],
                k.xyz[1],
                k.xyz[2],
                k.mode,
                k.duration,
            ]);
        }
        put_track(&mut out, 0x795, &words);
    }
    out.extend(0x7789u16.to_le_bytes());
    Ok(out)
}

fn put_track(out: &mut Vec<u8>, tag: u16, words: &[u32]) {
    out.extend(tag.to_le_bytes());
    out.extend((2 + 4 * words.len() as u32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
}

/// A track body of `size` object bytes after its u16 version 1.
fn track(bytes: &[u8], size: usize) -> Result<Reader<'_>, FormatError> {
    if bytes.len() != 2 + size {
        return Err(FormatError(
            "snapshot draw track length differs from its object",
        ));
    }
    let mut r = Reader::new(bytes);
    if r.u16()? != 1 {
        return Err(FormatError("unsupported snapshot draw track version"));
    }
    Ok(r)
}

fn playing(word: u32) -> Result<bool, FormatError> {
    match word {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FormatError("snapshot draw track play flag is not 0 or 1")),
    }
}

/// Key indices a playing track follows; negative `next` ends the track.
fn keys_in_range(next: i32, keys: usize) -> bool {
    next < keys as i32
}

fn active(start: u32, current: u32, keys: usize) -> Result<(), FormatError> {
    if start as usize >= keys || current as usize >= keys {
        return Err(FormatError("snapshot draw track key exceeds bounds"));
    }
    Ok(())
}

fn unpacked(word: u32) -> [i16; 2] {
    [word as u16 as i16, (word >> 16) as u16 as i16]
}

fn scale(bytes: &[u8]) -> Result<ScaleMotion, FormatError> {
    let mut r = track(bytes, SCALE_SIZE)?;
    let [play, x, y, start, current, elapsed] = r.words::<6>()?;
    let mut keys = [ScaleKey::default(); SCALE_KEYS];
    for key in &mut keys {
        let [next, kx, ky, mode, duration] = r.words::<5>()?;
        // 0x21e1 accepts interpolation modes 0 and 1 only.
        if !keys_in_range(next as i32, SCALE_KEYS) || !matches!(mode as u8, 0 | 1) {
            return Err(FormatError("snapshot scale key is not supported"));
        }
        *key = ScaleKey {
            next: next as i32,
            xy: [kx, ky],
            mode,
            duration,
        };
    }
    r.finish()?;
    let playback = if playing(play)? {
        active(start, current, SCALE_KEYS)?;
        Some(ScalePlayback {
            start,
            current,
            elapsed,
        })
    } else {
        None
    };
    Ok(ScaleMotion {
        base: [x, y],
        playback,
        keys,
    })
}

fn rotation(bytes: &[u8]) -> Result<RotationTrack, FormatError> {
    let mut r = track(bytes, OPACITY_SIZE)?;
    let [play, base, start, current, elapsed] = r.words::<5>()?;
    let angle = |v: u32| (-35900..=35900).contains(&(v as i32)) && (v as i32) % 100 == 0;
    if !angle(base) {
        return Err(FormatError("unsupported snapshot rotation base"));
    }
    let mut keys = [RotationKey::default(); ROTATION_KEYS];
    for key in &mut keys {
        let [next, value, mode, duration] = r.words::<4>()?;
        if !keys_in_range(next as i32, ROTATION_KEYS)
            || !angle(value)
            || !RotationTrack::supports(mode)
            || (mode as u8 == 0x80 && duration == 0)
        {
            return Err(FormatError("unsupported snapshot rotation key"));
        }
        *key = RotationKey {
            next: next as i32,
            value: value as i32,
            mode,
            duration,
        };
    }
    r.finish()?;
    let playing = playing(play)?;
    if playing {
        active(start, current, ROTATION_KEYS)?;
    }
    Ok(RotationTrack {
        playing,
        base: base as i32,
        start,
        current,
        elapsed,
        keys,
    })
}

fn opacity(bytes: &[u8]) -> Result<OpacityTrack, FormatError> {
    let mut r = track(bytes, OPACITY_SIZE)?;
    let [play, base, start, current, elapsed] = r.words::<5>()?;
    // 0x21ea / 0x21eb store |value| % 256.
    let magnitude = |value: u32| (value as i32).clamp(0, 255) as u32 == value;
    let mut keys = [OpacityKey::default(); OPACITY_KEYS];
    for key in &mut keys {
        let [next, value, hold, duration] = r.words::<4>()?;
        if !keys_in_range(next as i32, OPACITY_KEYS) || !magnitude(value) {
            return Err(FormatError("snapshot opacity key is not supported"));
        }
        *key = OpacityKey {
            next: next as i32,
            value: value as i32,
            hold,
            duration,
        };
    }
    r.finish()?;
    let playing = playing(play)?;
    if playing {
        active(start, current, OPACITY_KEYS)?;
    }
    if !magnitude(base) {
        return Err(FormatError("snapshot opacity base exceeds 255"));
    }
    // A track that never played keeps its header words: only playing reads them.
    Ok(OpacityTrack {
        playing,
        base: base as i32,
        start,
        current,
        elapsed,
        keys,
    })
}

fn position(bytes: &[u8]) -> Result<PositionMotion, FormatError> {
    let mut r = track(bytes, POSITION_SIZE)?;
    let [play, base, start, current, elapsed] = r.words::<5>()?;
    let mut keys = [PositionKey::default(); POSITION_KEYS];
    for key in &mut keys {
        let [next, xy, mode, duration] = r.words::<4>()?;
        // 0x21f5 checks in Rust: implemented modes, durations the signed division accepts.
        if !keys_in_range(next as i32, POSITION_KEYS)
            || !PositionMotion::supported(mode)
            || duration > i32::MAX as u32
        {
            return Err(FormatError("snapshot position key is not supported"));
        }
        *key = PositionKey {
            next: next as i32,
            xy: unpacked(xy),
            mode,
            duration,
        };
    }
    r.finish()?;
    let playback = if playing(play)? {
        active(start, current, POSITION_KEYS)?;
        Some(PositionPlayback {
            base: unpacked(base),
            start,
            current,
            elapsed,
        })
    } else {
        None
    };
    Ok(PositionMotion { playback, keys })
}

fn world(bytes: &[u8]) -> Result<WorldMotion, FormatError> {
    let mut r = track(bytes, WORLD_SIZE)?;
    let [play, x, y, z, start, current, elapsed] = r.words::<7>()?;
    let mut keys = [MotionKey::default(); WORLD_MOTION_KEYS];
    for key in &mut keys {
        let [next, kx, ky, kz, mode, duration] = r.words::<6>()?;
        if !keys_in_range(next as i32, WORLD_MOTION_KEYS) || !WorldMotion::supports_mode(mode) {
            return Err(FormatError("snapshot world motion key is not supported"));
        }
        *key = MotionKey {
            next: next as i32,
            xyz: [kx, ky, kz],
            mode,
            duration,
        };
    }
    r.finish()?;
    let playback = if playing(play)? {
        active(start, current, WORLD_MOTION_KEYS)?;
        Some(MotionPlayback {
            base: [x, y, z],
            start,
            current,
            elapsed,
        })
    } else {
        None
    };
    Ok(WorldMotion { playback, keys })
}

fn animation(bytes: &[u8]) -> Result<Animation, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 2 {
        return Err(FormatError("unsupported snapshot animation version"));
    }
    let h = r.words::<4>()?;
    if h[0] > 1 {
        return Err(FormatError("unsupported snapshot animation play flag"));
    }
    let mut frames = [AnimationFrame::default(); 64];
    for frame in &mut frames {
        frame.next = r.u32()? as i32;
        for word in &mut frame.words {
            *word = r.u16()? as i16;
        }
        frame.duration = r.u32()?;
        if frame.next >= 64 {
            return Err(FormatError("snapshot animation next frame exceeds bounds"));
        }
    }
    if h[0] != 0 && (h[1] >= 64 || h[2] >= 64) {
        return Err(FormatError(
            "snapshot active animation frame exceeds bounds",
        ));
    }
    r.finish()?;
    Ok(Animation {
        playing: h[0] != 0,
        start: h[1],
        current: h[2],
        elapsed: h[3],
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_subtract_round_trips_raw_word_and_survives_transient_reset() {
        for value in [0, 1, 0x8000_0000, u32::MAX] {
            let draw = DrawState {
                texture_subtract: value,
                ..DrawState::default()
            };
            let bytes = encode(&draw).unwrap();
            assert_eq!(&bytes[108..112], &value.to_le_bytes());
            let mut loaded = decode(&bytes).unwrap();
            assert_eq!(loaded, draw);
            reset_transients(&mut loaded);
            assert_eq!(loaded.texture_subtract, value);
            assert_eq!(encode(&loaded).unwrap(), bytes);
        }
    }

    #[test]
    fn rotation_record_round_trip_resumes_from_saved_progress_and_rejects_bad_keys() {
        let mut keys = [RotationKey::default(); ROTATION_KEYS];
        keys[0] = RotationKey {
            next: -1,
            value: 9000,
            mode: 0,
            duration: 400,
        };
        let mut draw = DrawState::default();
        draw.words[20] = 2250;
        draw.rotation_track = Some(Box::new(RotationTrack {
            playing: true,
            base: 0,
            start: 0,
            current: 0,
            elapsed: 100,
            keys,
        }));
        let bytes = encode(&draw).unwrap();
        assert_eq!(
            bodies(&bytes)
                .iter()
                .map(|&(tag, _, len)| (tag, len))
                .collect::<Vec<_>>(),
            [(0x791, 150)]
        );
        assert_eq!(decode(&bytes).unwrap(), draw);
        let mut loaded = crate::scene::SceneObject {
            draw: Some(decode(&bytes).unwrap()),
            ..Default::default()
        };
        loaded.advance_effects(100);
        assert_eq!(loaded.draw.as_ref().unwrap().words[20], 4500);
        loaded.advance_effects(200);
        assert_eq!(loaded.draw.as_ref().unwrap().words[20], 9000);
        assert!(loaded.draw.as_ref().unwrap().rotation_track.is_none());
        for (offset, value) in [(2, 2u32), (10, 8), (14, 8), (6, 1), (26, 36000), (30, 2)] {
            let mut bad = bytes.clone();
            let at = bodies(&bytes)[0].1 + offset;
            bad[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&bad).is_err(), "offset {offset}");
        }
    }

    /// Scale playing, opacity not yet started, XY playing, XYZ not yet started.
    fn tracked() -> DrawState {
        let mut draw = DrawState::default();
        let mut scale = ScaleMotion {
            base: [1.5f32.to_bits(), 2.0f32.to_bits()],
            playback: Some(ScalePlayback {
                start: 1,
                current: 2,
                elapsed: 30,
            }),
            keys: [ScaleKey::default(); SCALE_KEYS],
        };
        scale.keys[2] = ScaleKey {
            next: -1,
            xy: [3.0f32.to_bits(), 4.0f32.to_bits()],
            mode: 1,
            duration: 500,
        };
        draw.scale_motion = Some(Box::new(scale));
        let mut opacity = OpacityTrack {
            playing: false,
            base: 255,
            start: 0,
            current: 0,
            elapsed: 0,
            keys: [OpacityKey::default(); OPACITY_KEYS],
        };
        opacity.keys[0] = OpacityKey {
            next: -2,
            value: 128,
            hold: 1,
            duration: 100,
        };
        draw.opacity_track = Some(Box::new(opacity));
        let mut position = PositionMotion {
            playback: Some(PositionPlayback {
                base: [-5, 700],
                start: 0,
                current: 3,
                elapsed: 16,
            }),
            ..PositionMotion::default()
        };
        position.keys[3] = PositionKey {
            next: 15,
            xy: [i16::MIN, i16::MAX],
            mode: 0x21 | (3 << 16) | (4 << 24),
            duration: 900,
        };
        draw.position_motion = Some(Box::new(position));
        let mut world = WorldMotion::default();
        world.keys[7] = MotionKey {
            next: 0,
            xyz: [1, 2, 3],
            mode: 0x38 | (100 << 16) | (2 << 28),
            duration: 1000,
        };
        draw.world_motion = Some(Box::new(world));
        draw
    }

    /// Body offsets of the track chunks after the fixed 116-byte fields.
    fn bodies(bytes: &[u8]) -> Vec<(u16, usize, usize)> {
        let mut out = Vec::new();
        let mut at = 116;
        while u16::from_le_bytes([bytes[at], bytes[at + 1]]) != 0x7789 {
            let tag = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            let len = u32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize;
            out.push((tag, at + 6, len));
            at += 6 + len;
        }
        out
    }

    #[test]
    fn motion_tracks_round_trip_in_writer_order_with_object_sizes() {
        let draw = tracked();
        let bytes = encode(&draw).unwrap();
        let chunks: Vec<_> = bodies(&bytes).iter().map(|&(t, _, l)| (t, l)).collect();
        assert_eq!(
            chunks,
            [(0x790, 186), (0x792, 150), (0x794, 278), (0x795, 222)]
        );
        assert_eq!(decode(&bytes).unwrap(), draw);
        // Unstarted XYZ: playing 0 and zeroed header words.
        let (_, world, _) = bodies(&bytes)[3];
        assert_eq!(
            bytes[world..world + 30],
            [[1, 0].as_slice(), &[0; 28]].concat()
        );
    }

    #[test]
    fn track_bodies_are_checked_like_their_setters() {
        let bytes = encode(&tracked()).unwrap();
        let chunks = bodies(&bytes);
        let at = |index: usize, offset: usize| chunks[index].1 + offset;
        let word = |value: u32| value.to_le_bytes().to_vec();
        for (offset, value) in [
            (at(0, 0), vec![2, 0]),              // track version
            (at(0, 2), word(2)),                 // play flag
            (at(0, 2 + 16), word(8)),            // playing current key
            (at(0, 2 + 24), word(8)),            // scale next key
            (at(0, 2 + 24 + 12), word(2)),       // scale mode
            (at(1, 2 + 4), word(256)),           // opacity base
            (at(1, 2 + 20 + 4), word(u32::MAX)), // opacity value
            (at(2, 2 + 20 + 8), word(0x22)),     // XY mode
            (at(2, 2 + 20 + 12), word(1 << 31)), // XY duration
            (at(3, 2 + 28 + 16), word(0x40)),    // XYZ mode
            (bytes.len() - 2, vec![0x88, 0x77]), // terminator
            (chunks[0].1 - 6, vec![0x91, 0x07]), // rotation track
            (chunks[2].1 - 6, vec![0x90, 0x07]), // out of order
        ] {
            let mut bad = bytes.clone();
            bad[offset..offset + value.len()].copy_from_slice(&value);
            assert!(decode(&bad).is_err(), "offset {offset}");
        }
    }
}

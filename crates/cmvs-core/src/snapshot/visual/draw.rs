//! Draw state chunk 0x780 (writer 0x14001eef0, reader 0x14001f080) and its 0x793 frame
//! animation. The decoder keeps the saved +84 fade factor; the loader's reset to 256 is
//! applied when hosts are rebuilt.
use super::Reader;
use crate::{
    FormatError,
    scene::{Animation, AnimationFrame, DrawState},
};

pub(super) fn decode(bytes: &[u8]) -> Result<DrawState, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 4 || r.u16()? != 0 {
        return Err(FormatError("unsupported snapshot draw version"));
    }
    let words = r.words::<22>()?;
    let extra = r.words::<6>()?;
    if extra[1..5] != [0; 4] {
        return Err(FormatError("unsupported snapshot draw extra fields"));
    }
    let mut out = DrawState {
        words,
        texture_mode: extra[0],
        depth_bias: extra[5],
        ..DrawState::default()
    };
    if r.peek() == Some(0x793) {
        r.u16()?;
        out.animation = Some(Box::new(animation(r.chunk()?)?));
    }
    if r.peek().is_some_and(|tag| (0x790..=0x795).contains(&tag)) {
        return Err(FormatError(
            "snapshot draw motion channel is not implemented or duplicated",
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

/// Inverse of `decode` for the supported channels: only frame animation (0x793).
pub(crate) fn encode(draw: &DrawState) -> Result<Vec<u8>, FormatError> {
    if draw.opacity_track.is_some()
        || draw.scale_motion.is_some()
        || draw.position_motion.is_some()
        || draw.world_motion.is_some()
    {
        return Err(FormatError(
            "saving a draw state with a running motion channel is not implemented",
        ));
    }
    let mut out = 4u16.to_le_bytes().to_vec();
    out.extend(0u16.to_le_bytes());
    for word in draw.words {
        out.extend(word.to_le_bytes());
    }
    for word in [draw.texture_mode, 0, 0, 0, 0, draw.depth_bias] {
        out.extend(word.to_le_bytes());
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
    out.extend(0x7789u16.to_le_bytes());
    Ok(out)
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

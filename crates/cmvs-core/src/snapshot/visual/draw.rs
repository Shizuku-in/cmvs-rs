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
    let mut words = r.words::<22>()?;
    // Loader 0x14001f080 explicitly discards the captured transient fade.
    words[21] = 256;
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

fn animation(bytes: &[u8]) -> Result<Animation, FormatError> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 2 {
        return Err(FormatError("unsupported snapshot animation version"));
    }
    let h = r.words::<4>()?;
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

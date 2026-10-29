use super::{Reader, SavedScene, SavedWindow};
use crate::{
    FormatError,
    message::{MessageWindow, WindowButton},
    scene::SceneObject,
};

pub(crate) fn decode(bytes: &[u8], nodes: &mut usize) -> Result<SavedWindow, FormatError> {
    let mut r = Reader::new(bytes);
    let text = usize::from(r.u16()?);
    if text >= 12 || r.u16()? != 3 {
        return Err(FormatError(
            "unsupported snapshot window version or text slot",
        ));
    }
    let logical_size = r.words::<2>()?;
    let raw = r.take(2496)?;
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let half = |at: usize| u16::from_le_bytes(raw[at..at + 2].try_into().unwrap());
    if word(0) as usize != text || raw[34..40] != [0; 6] {
        return Err(FormatError("snapshot window text or padding disagrees"));
    }
    let mut buttons = [WindowButton::default(); 32];
    let mut button_words = [0; 32];
    for (i, button) in buttons.iter_mut().enumerate() {
        let at = 40 + 76 * i;
        button.enabled = word(at) != 0;
        button.states =
            std::array::from_fn(|state| std::array::from_fn(|n| half(at + 4 + 12 * state + 2 * n)));
        button.hit = std::array::from_fn(|n| half(at + 64 + 2 * n));
        button.state_word = word(at + 72);
        button_words[i] = word(at + 72);
    }
    let scene = SavedScene::decode(r.chunk()?, 0, nodes)?;
    r.finish()?;
    Ok(SavedWindow {
        state: MessageWindow {
            text,
            visible: word(4) != 0,
            hidden: word(8) != 0,
            position: [word(12) as i32, word(16) as i32],
            size: [word(20) as i32, word(24) as i32],
            switch_positions: [
                [word(2480) as i32, word(2484) as i32],
                [word(2472) as i32, word(2476) as i32],
            ],
            cursor_mode: half(28),
            cursor_offset: [half(30) as i16, half(32) as i16],
            position_switch: word(2488) != 0,
            object: SceneObject::default(),
            buttons,
            pressed_button: -1,
            switch_state: word(2492),
        },
        logical_size,
        button_words,
        switch_state: word(2492),
        scene,
    })
}

/// Window writer 0x14005d050: the record decoded above, from a window and its scene.
pub(crate) fn encode(window: &SavedWindow) -> Result<Vec<u8>, crate::FormatError> {
    let state = &window.state;
    let mut raw = vec![0u8; 2496];
    let mut word = |at: usize, value: u32| raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
    word(0, state.text as u32);
    word(4, u32::from(state.visible));
    word(8, u32::from(state.hidden));
    word(12, state.position[0] as u32);
    word(16, state.position[1] as u32);
    word(20, state.size[0] as u32);
    word(24, state.size[1] as u32);
    word(2472, state.switch_positions[1][0] as u32);
    word(2476, state.switch_positions[1][1] as u32);
    word(2480, state.switch_positions[0][0] as u32);
    word(2484, state.switch_positions[0][1] as u32);
    word(2488, u32::from(state.position_switch));
    word(2492, window.switch_state);
    for (i, button) in state.buttons.iter().enumerate() {
        let at = 40 + 76 * i;
        word(at, u32::from(button.enabled));
        word(at + 72, window.button_words[i]);
    }
    let mut half = |at: usize, value: u16| raw[at..at + 2].copy_from_slice(&value.to_le_bytes());
    half(28, state.cursor_mode);
    half(30, state.cursor_offset[0] as u16);
    half(32, state.cursor_offset[1] as u16);
    for (i, button) in state.buttons.iter().enumerate() {
        let at = 40 + 76 * i;
        for (s, values) in button.states.iter().enumerate() {
            for (n, &value) in values.iter().enumerate() {
                half(at + 4 + 12 * s + 2 * n, value);
            }
        }
        for (n, &value) in button.hit.iter().enumerate() {
            half(at + 64 + 2 * n, value);
        }
    }
    let mut out = (state.text as u16).to_le_bytes().to_vec();
    out.extend(3u16.to_le_bytes());
    for size in window.logical_size {
        out.extend(size.to_le_bytes());
    }
    out.extend(raw);
    let scene = window.scene.encode()?;
    out.extend((scene.len() as u32).to_le_bytes());
    out.extend(scene);
    Ok(out)
}

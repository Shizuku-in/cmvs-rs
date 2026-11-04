use super::{Reader, SavedScene, SavedWindow};
use crate::{
    FormatError,
    message::{MessageWindow, WindowButton},
    scene::SceneObject,
    snapshot::SnapshotLayout,
};

pub(crate) fn decode(bytes: &[u8], nodes: &mut usize) -> Result<SavedWindow, FormatError> {
    decode_with_layout(bytes, nodes, SnapshotLayout::TextV3)
}
pub(crate) fn decode_with_layout(
    bytes: &[u8],
    nodes: &mut usize,
    layout: SnapshotLayout,
) -> Result<SavedWindow, FormatError> {
    let mut r = Reader::new(bytes);
    let text_v1_scene_v1 = layout.text_version() == 1;
    let text = usize::from(r.u16()?);
    if text >= if text_v1_scene_v1 { 8 } else { 12 }
        || r.u16()? != if text_v1_scene_v1 { 1 } else { 3 }
    {
        return Err(FormatError(
            "unsupported snapshot window version or text slot",
        ));
    }
    let logical_size = if text_v1_scene_v1 {
        [0; 2]
    } else {
        r.words::<2>()?
    };
    let raw = r.take(if text_v1_scene_v1 { 1280 } else { 2496 })?;
    let switch_at = if text_v1_scene_v1 { 1256 } else { 2472 };
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let half = |at: usize| u16::from_le_bytes(raw[at..at + 2].try_into().unwrap());
    if word(0) as usize != text || raw[34..40] != [0; 6] {
        return Err(FormatError("snapshot window text or padding disagrees"));
    }
    let mut buttons = [WindowButton::default(); 32];
    let mut button_words = [0; 32];
    for (i, button) in buttons
        .iter_mut()
        .enumerate()
        .take(if text_v1_scene_v1 { 16 } else { 32 })
    {
        let at = 40 + 76 * i;
        button.enabled = word(at) != 0;
        button.states =
            std::array::from_fn(|state| std::array::from_fn(|n| half(at + 4 + 12 * state + 2 * n)));
        button.hit = std::array::from_fn(|n| half(at + 64 + 2 * n));
        button.state_word = word(at + 72);
        button_words[i] = word(at + 72);
    }
    let scene = SavedScene::decode_with_layout(r.chunk()?, 0, nodes, layout)?;
    r.finish()?;
    Ok(SavedWindow {
        state: MessageWindow {
            text,
            visible: word(4) != 0,
            hidden: word(8) != 0,
            position: [word(12) as i32, word(16) as i32],
            size: [word(20) as i32, word(24) as i32],
            switch_positions: [
                [word(switch_at + 8) as i32, word(switch_at + 12) as i32],
                [word(switch_at) as i32, word(switch_at + 4) as i32],
            ],
            cursor_mode: half(28),
            cursor_offset: [half(30) as i16, half(32) as i16],
            position_switch: word(switch_at + 16) != 0,
            object: SceneObject::default(),
            buttons,
            pressed_button: -1,
            switch_state: word(switch_at + 20),
        },
        logical_size: if text_v1_scene_v1 {
            scene.logical_size
        } else {
            logical_size
        },
        button_words,
        switch_state: word(switch_at + 20),
        scene,
    })
}

/// Window writer 0x14005d050: the record decoded above, from a window and its scene.
pub(crate) fn encode_with_layout(
    window: &SavedWindow,
    layout: SnapshotLayout,
) -> Result<Vec<u8>, crate::FormatError> {
    let state = &window.state;
    let text_v1_scene_v1 = layout.text_version() == 1;
    if text_v1_scene_v1
        && (state.text >= 8
            || state.buttons[16..]
                .iter()
                .any(|b| *b != WindowButton::default())
            || window.button_words[16..] != [0; 16])
    {
        return Err(FormatError(
            "window v1 supports only eight texts and sixteen buttons",
        ));
    }
    let switch_at = if text_v1_scene_v1 { 1256 } else { 2472 };
    let count = if text_v1_scene_v1 { 16 } else { 32 };
    let mut raw = vec![0u8; if text_v1_scene_v1 { 1280 } else { 2496 }];
    let mut word = |at: usize, value: u32| raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
    word(0, state.text as u32);
    word(4, u32::from(state.visible));
    word(8, u32::from(state.hidden));
    word(12, state.position[0] as u32);
    word(16, state.position[1] as u32);
    word(20, state.size[0] as u32);
    word(24, state.size[1] as u32);
    word(switch_at, state.switch_positions[1][0] as u32);
    word(switch_at + 4, state.switch_positions[1][1] as u32);
    word(switch_at + 8, state.switch_positions[0][0] as u32);
    word(switch_at + 12, state.switch_positions[0][1] as u32);
    word(switch_at + 16, u32::from(state.position_switch));
    word(switch_at + 20, window.switch_state);
    for (i, button) in state.buttons.iter().enumerate().take(count) {
        let at = 40 + 76 * i;
        word(at, u32::from(button.enabled));
        word(at + 72, window.button_words[i]);
    }
    let mut half = |at: usize, value: u16| raw[at..at + 2].copy_from_slice(&value.to_le_bytes());
    half(28, state.cursor_mode);
    half(30, state.cursor_offset[0] as u16);
    half(32, state.cursor_offset[1] as u16);
    for (i, button) in state.buttons.iter().enumerate().take(count) {
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
    out.extend(if text_v1_scene_v1 { 1u16 } else { 3u16 }.to_le_bytes());
    if !text_v1_scene_v1 {
        for size in window.logical_size {
            out.extend(size.to_le_bytes());
        }
    }
    out.extend(raw);
    let scene = window.scene.encode_with_layout(layout)?;
    out.extend((scene.len() as u32).to_le_bytes());
    out.extend(scene);
    Ok(out)
}

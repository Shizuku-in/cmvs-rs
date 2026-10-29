use super::{Reader, SavedScene, SavedWindow};
use crate::{
    FormatError,
    message::{MessageWindow, WindowButton},
    scene::SceneObject,
};

pub(super) fn decode(bytes: &[u8], nodes: &mut usize) -> Result<SavedWindow, FormatError> {
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
        },
        logical_size,
        button_words,
        switch_state: word(2492),
        scene,
    })
}

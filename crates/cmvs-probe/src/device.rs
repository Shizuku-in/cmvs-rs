//! Abstract probe actions and the devices that translate them into engine input codes
//! Checks only issue `Action`s, so one check covers both devices.
use cmvs_core::input::{KeyBindings, MOUSE_CODES};

/// Engine action indices of the key.cfg bank (KEY_FUNCTION_01.. minus one). These are
/// engine semantics shared by all builds, not per-game data; builds whose
/// actions differ must be represented as a capability, never as a game branch.
pub mod engine_action {
    /// Decide: releasing it advances message waits and presses the current button.
    pub const DECIDE: usize = 0;
    pub const CANCEL: usize = 1;
    /// Button navigation: previous and next link of the current button.
    pub const PREVIOUS: usize = 2;
    pub const NEXT: usize = 3;
    /// Normal skip, held.
    pub const SKIP: usize = 8;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Mouse,
    Keyboard,
}

impl Device {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mouse => "mouse",
            Self::Keyboard => "keyboard",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mouse" => Some(Self::Mouse),
            "keyboard" => Some(Self::Keyboard),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Advance a message wait: decide pressed for one frame, released on the next.
    Advance,
    Cancel,
    /// Hold skip for the given number of frames.
    SkipHold(u32),
    /// Click button `button` of group slot `group`. The keyboard device walks the button
    /// links with the navigation keys and then decides; the session drives that walk.
    Press {
        group: usize,
        button: usize,
    },
    /// Mouse wheel up or down for one frame.
    WheelUp,
    WheelDown,
}

/// Input held during one VM frame. `pointer` keeps its previous value when `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    pub codes: Vec<usize>,
    pub pointer: Option<[i32; 2]>,
}

/// Pointer position outside every button, so a mouse advance is never taken by a button.
pub const OFF_SCREEN: [i32; 2] = [-1, -1];

/// One tap of a keyboard-bound engine action: pressed for a frame, then released.
pub fn key_tap(bindings: &KeyBindings, action: usize) -> Result<Vec<Frame>, String> {
    let code = bound_key(bindings, action)
        .ok_or_else(|| format!("key.cfg binds no key to engine action {action}"))?;
    Ok(vec![
        Frame {
            codes: vec![code],
            pointer: None,
        },
        Frame::default(),
    ])
}

/// A non-mouse code bound to normal skip, held by route exploration whatever the device.
pub fn skip_code(bindings: &KeyBindings) -> Option<usize> {
    bound_key(bindings, engine_action::SKIP)
}

fn bound_key(bindings: &KeyBindings, action: usize) -> Option<usize> {
    bindings.codes()[action]
        .iter()
        .map(|&c| usize::from(c))
        .find(|c| *c != 0 && !MOUSE_CODES.contains(c))
}

impl Device {
    /// Frames performing `action`, ending with all codes released. `centre` gives the
    /// pointer for a button press. Errors name actions this device cannot perform.
    pub fn frames(
        self,
        action: Action,
        bindings: &KeyBindings,
        centre: impl Fn(usize, usize) -> Option<[i32; 2]>,
    ) -> Result<Vec<Frame>, String> {
        let tap = |code: usize, pointer: Option<[i32; 2]>| {
            vec![
                Frame {
                    codes: vec![code],
                    pointer,
                },
                Frame {
                    codes: Vec::new(),
                    pointer,
                },
            ]
        };
        let key = |action: usize, label: &str| {
            bound_key(bindings, action).ok_or_else(|| format!("key.cfg binds no key to {label}"))
        };
        match (self, action) {
            (Self::Mouse, Action::Advance) => Ok(tap(MOUSE_CODES[0], Some(OFF_SCREEN))),
            (Self::Mouse, Action::Cancel) => Ok(tap(MOUSE_CODES[1], Some(OFF_SCREEN))),
            (Self::Mouse, Action::Press { group, button }) => {
                let at = centre(group, button)
                    .ok_or_else(|| format!("button {group}:{button} is not present"))?;
                // Hover first so the button host sees the pointer before the press edge.
                let mut frames = vec![Frame {
                    codes: Vec::new(),
                    pointer: Some(at),
                }];
                frames.extend(tap(MOUSE_CODES[0], Some(at)));
                // Leave the button so hover states do not outlive the press.
                frames.push(Frame {
                    codes: Vec::new(),
                    pointer: Some(OFF_SCREEN),
                });
                Ok(frames)
            }
            (Self::Mouse, Action::SkipHold(_)) => Err("the mouse device has no skip".into()),
            (Self::Keyboard, Action::Advance) => {
                Ok(tap(key(engine_action::DECIDE, "decide")?, None))
            }
            (Self::Keyboard, Action::Cancel) => {
                Ok(tap(key(engine_action::CANCEL, "cancel")?, None))
            }
            (Self::Keyboard, Action::SkipHold(frames)) => {
                let code = key(engine_action::SKIP, "skip")?;
                let mut out = vec![
                    Frame {
                        codes: vec![code],
                        pointer: None,
                    };
                    frames as usize
                ];
                out.push(Frame::default());
                Ok(out)
            }
            (Self::Keyboard, Action::Press { .. }) => {
                Err("keyboard presses are driven by the session".into())
            }
            (Self::Mouse, Action::WheelUp) => Ok(tap(MOUSE_CODES[2], None)),
            (Self::Mouse, Action::WheelDown) => Ok(tap(MOUSE_CODES[3], None)),
            (Self::Keyboard, Action::WheelUp | Action::WheelDown) => {
                Err("the keyboard device has no wheel".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindings() -> KeyBindings {
        KeyBindings::parse(
            "[KEY_FUNCTION_01]\nKEY_DATA_01=1\nKEY_DATA_02=12\n\
             [KEY_FUNCTION_02]\nKEY_DATA_01=3\n\
             [KEY_FUNCTION_09]\nKEY_DATA_01=24\n",
        )
        .unwrap()
    }

    #[test]
    fn keyboard_uses_the_first_non_mouse_binding() {
        let frames = Device::Keyboard
            .frames(Action::Advance, &bindings(), |_, _| None)
            .unwrap();
        assert_eq!(frames[0].codes, [12]);
        assert!(frames[1].codes.is_empty());
        // Cancel is bound only to the right mouse button here.
        assert!(
            Device::Keyboard
                .frames(Action::Cancel, &bindings(), |_, _| None)
                .is_err()
        );
        let skip = Device::Keyboard
            .frames(Action::SkipHold(3), &bindings(), |_, _| None)
            .unwrap();
        assert_eq!(skip.len(), 4);
        assert_eq!(skip[2].codes, [24]);
    }

    #[test]
    fn mouse_press_hovers_then_clicks_the_button_centre() {
        let frames = Device::Mouse
            .frames(
                Action::Press {
                    group: 2,
                    button: 1,
                },
                &bindings(),
                |g, b| (g == 2 && b == 1).then_some([10, 20]),
            )
            .unwrap();
        assert_eq!(frames.len(), 4);
        assert!(frames[0].codes.is_empty());
        assert_eq!(frames[1].codes, [MOUSE_CODES[0]]);
        assert!(frames[..3].iter().all(|f| f.pointer == Some([10, 20])));
        assert_eq!(frames[3].pointer, Some(OFF_SCREEN));
        assert!(
            Device::Mouse
                .frames(
                    Action::Press {
                        group: 0,
                        button: 0
                    },
                    &bindings(),
                    |_, _| None
                )
                .is_err()
        );
    }
}

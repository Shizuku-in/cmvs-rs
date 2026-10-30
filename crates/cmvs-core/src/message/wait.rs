//! Manual/automatic 0x2153 / 0x140076880 -> 0x14005e220 with an explicit read policy.
//! A pointer gesture result gates completion; the +12244 bypass (0x2340) completes like the
//! skip flag. Automatic advance uses explicit frame time and voice playback status.
use super::{MessageHost, RESEARCH_SCREEN, window_slot};
use crate::{
    audio::{AudioBackend, RecordingAudio},
    bytecode::DecodeError,
    evaluate::error,
    input::ActionInput,
    machine::{Machine, Step},
    message_flags::MessageFlags,
    settings::AutoAdvanceSettings,
    vm::SystemResult,
};

#[derive(Clone, Copy)]
pub struct AutoAdvanceContext<'a> {
    pub settings: AutoAdvanceSettings,
    pub delta_ms: u32,
    pub gesture_result: bool,
    pub read_eligible: bool,
    pub audio: &'a dyn AudioBackend,
}

impl MessageHost {
    /// Waits for a release of action 0 or 20 outside message buttons, or a skip flag/action.
    /// No glyphs are generated. Pending returns 0xa000; completion 0x400c does not yield.
    /// This convenience path uses a zero-time recording context; automatic runs supply
    /// their settings, frame delta and audio backend through `try_wait_with_auto`.
    pub fn try_wait(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, DecodeError> {
        self.try_wait_with_auto(
            vm,
            input,
            flags,
            AutoAdvanceContext {
                settings: AutoAdvanceSettings::default(),
                delta_ms: 0,
                gesture_result: false,
                read_eligible: true,
                audio: &RecordingAudio::default(),
            },
        )
    }
    pub fn try_wait_with_auto(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x153)) = vm.pending_system() else {
            return Ok(None);
        };
        let slot = window_slot(vm, pc, 0x2153)?;
        let _read = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x2153, e.0))?;
        let voice = vm
            .stack()
            .peek_u32(12)
            .map_err(|e| error(pc, 0x2153, e.0))? as i32;
        let mut window = self
            .window(slot)
            .ok_or_else(|| error(pc, 0x2153, "missing message window sets an unmodeled flag"))?
            .clone();
        let text = self
            .text(window.text)
            .ok_or_else(|| error(pc, 0x2153, "message window without its text object"))?;
        if window.hidden {
            return vm.yield_pending_system().map(Some);
        }
        let mut done = !context.gesture_result
            && (input.held(7)
                || ((flags.skip() || input.held(8)) && context.read_eligible)
                || flags.bypass());
        let mut consume = false;
        let mut advance = false;
        let mut elapsed = self.auto_elapsed;
        if !done {
            if input.pressed(0) {
                window.pressed_button = -1;
            }
            let [px, py] = input.pointer();
            let [px, py] = [px, py].map(|value| value as u32);
            let px = (px.wrapping_mul(text.logical_size[0] as u32) / RESEARCH_SCREEN[0]) as i32;
            let py = (py.wrapping_mul(text.logical_size[1] as u32) / RESEARCH_SCREEN[1]) as i32;
            let mut hovered = false;
            for (index, button) in window.buttons.iter().enumerate().filter(|(_, b)| b.enabled) {
                let [x, y, w, h] = button.hit.map(i32::from);
                let left = window.position[0].wrapping_add(x);
                let top = window.position[1].wrapping_add(y);
                // 0x14005e220 uses inclusive right/bottom bounds (unlike 0x215f).
                if left <= px
                    && top <= py
                    && px <= left.wrapping_add(w)
                    && py <= top.wrapping_add(h)
                {
                    hovered = true;
                    if input.pressed(0) {
                        window.pressed_button = index as i32;
                    }
                    consume |= input.released(0);
                }
            }
            if input.released(0) && window.pressed_button >= 0 {
                consume = true;
                window.pressed_button = -1;
            }
            if !input.held(0) {
                window.pressed_button = -1;
            }
            // 0x14005e48f: an action 0 release left after the button checks, or an action 20
            // release, advances and consumes both actions (0x140054d90, 0x140055160).
            if !hovered
                && !context.gesture_result
                && ((input.released(0) && !consume) || input.released(20))
            {
                done = true;
                consume = true;
                advance = true;
            }
            if !done && flags.auto_advance() && !context.gesture_result {
                elapsed = elapsed.wrapping_add(context.delta_ms);
                if voice >= 0 && context.settings.sync_voice {
                    let playing = context
                        .audio
                        .voice_position((voice % 4) as u8)
                        .map_err(|_| {
                            error(
                                pc,
                                0x2153,
                                "voice status query failed during automatic advance",
                            )
                        })?
                        .is_some();
                    if playing {
                        elapsed = 0;
                    } else {
                        done = elapsed
                            >= context
                                .settings
                                .voice_wait
                                .wrapping_mul(100)
                                .wrapping_add(250);
                    }
                } else {
                    let duration = context
                        .settings
                        .text_delay_ms(text.glyphs.len())
                        .map_err(|why| error(pc, 0x2153, why))?;
                    done = elapsed >= duration;
                }
            }
        }
        if !done && window.cursor_mode == 3 {
            let position = [
                text.cursor[0].wrapping_add(i32::from(window.cursor_offset[0])),
                text.cursor[1].wrapping_add(i32::from(window.cursor_offset[1])),
            ];
            for id in [2, 3] {
                if let Some(child) = window.object.children.get_mut(&id) {
                    let draw = child
                        .draw
                        .as_mut()
                        .ok_or_else(|| error(pc, 0x2153, "wait cursor child has no draw state"))?;
                    draw.words[8] = position[0] as u32;
                    draw.words[9] = position[1] as u32;
                }
            }
        }
        for id in [2, 3] {
            if let Some(child) = window.object.children.get_mut(&id) {
                child.shown = !done;
            }
        }
        let step = if done {
            vm.complete_system(SystemResult(0x400c))?
        } else {
            vm.yield_pending_system()?
        };
        self.windows[slot] = Some(window);
        self.auto_elapsed = if done { 0 } else { elapsed };
        if consume {
            input.consume(0);
        }
        if advance {
            input.consume(20);
        }
        Ok(Some(step))
    }
}

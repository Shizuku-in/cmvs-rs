//! Window services, dialogs, and frame capture.
use crate::{
    bytecode::DecodeError,
    dispatch::{Call, Flow, Hosts, RuntimeOperation},
    machine::Machine,
    platform::{TextDialogRequest, TextDialogResult},
};
use std::error::Error;

pub(in super::super) fn root_platform(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .platform
        .as_mut()
        .expect("platform capability checked");
    Ok(Flow::Platform(
        h.host.handle(call, h.backend, h.assets, h.cursors)?,
    ))
}

/// 0x213c returns a comment and a confirmation result.
pub(in super::super) fn variant_text_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let arguments = [
        call.argument(4, "text dialog title")?,
        call.argument(8, "text dialog prompt")?,
        call.argument(12, "text dialog initial text")?,
        call.argument(16, "text dialog byte limit")?,
        call.argument(20, "text dialog output")?,
    ];
    // The inspected old helper ignores this sixth flag after the wrapper normalizes it.
    call.argument(24, "text dialog flag")?;
    if hosts.in_game_text_dialogs {
        return Ok(call.defer(RuntimeOperation::TextDialog { arguments }, 24)?);
    }
    let dialog = VmTextDialog::new(call.vm, arguments, |why| call.error(why))?;
    let answer = hosts
        .device_frame
        .as_deref_mut()
        .expect("text dialog backend checked")
        .poll_text_dialog(&dialog.request)?;
    let text = match answer {
        TextDialogResult::Pending => return Ok(Flow::Wait),
        TextDialogResult::Cancelled => None,
        TextDialogResult::Accepted(text) => Some(text),
    };
    let result = dialog.result(call.vm, text, true, |why| call.error(why))?;
    call.complete(0x4018)?;
    dialog.store(call.vm, result);
    Ok(Flow::Done)
}

pub(crate) struct VmTextDialog {
    pub request: TextDialogRequest,
    index: u32,
}

impl VmTextDialog {
    pub fn new(
        vm: &Machine,
        [title_raw, prompt_raw, initial_raw, max_bytes, output]: [u32; 5],
        error: impl Fn(&'static str) -> DecodeError,
    ) -> Result<Self, DecodeError> {
        let crate::vm::StringRef::StringBank(index) = crate::vm::StringRef::decode(output) else {
            return Err(error("text dialog output requires a writable string slot"));
        };
        let title = vm.resolve_string(title_raw)?;
        let prompt = vm.resolve_string(prompt_raw)?;
        let initial = match vm.resolve_string(initial_raw) {
            Ok(bytes) => bytes,
            Err(_)
                if matches!(
                    crate::vm::StringRef::decode(initial_raw),
                    crate::vm::StringRef::Frame(offset) if offset >= 0x3fff_0000
                ) =>
            {
                // Negative string-bank indices can wrap into a large frame offset.
                // The original lstrcpyA swallows the fault and leaves the field empty.
                Vec::new()
            }
            Err(err) => return Err(err),
        };
        let index = if index >= vm.state().string_slots() {
            0
        } else {
            index
        };
        if title.len() >= 144 || prompt.len() >= 256 || initial.len() >= 256 {
            return Err(error("text dialog strings exceed native capacity"));
        }
        Ok(Self {
            request: TextDialogRequest {
                title,
                prompt,
                initial,
                max_bytes,
            },
            index,
        })
    }

    pub fn result(
        &self,
        vm: &Machine,
        text: Option<Vec<u8>>,
        legacy_full_width_only: bool,
        error: impl Fn(&'static str) -> DecodeError,
    ) -> Result<(bool, Vec<u8>), DecodeError> {
        let accepted = text.is_some();
        let text = if let Some(bytes) = text {
            if bytes.len() >= 256 || bytes.contains(&0) {
                return Err(error("text dialog result exceeds native capacity"));
            }
            let mut comment = Vec::with_capacity(bytes.len());
            let mut rest = bytes.as_slice();
            while let Some(&byte) = rest.first() {
                let wide = vm.text_encoding().wide_len(byte);
                let (sequence, tail) = rest
                    .split_at_checked(wide.unwrap_or(1))
                    .ok_or_else(|| error("text dialog has a truncated double-byte character"))?;
                // The original host strips single-byte characters before the empty fallback.
                if !legacy_full_width_only || wide.is_some() {
                    comment.extend_from_slice(sequence);
                }
                rest = tail;
            }
            if comment.is_empty() {
                self.request.initial.clone()
            } else {
                comment
            }
        } else {
            self.request.initial.clone()
        };
        if text.contains(&0) {
            return Err(error("text dialog result exceeds native capacity"));
        }
        // Validate the write before consuming the arguments or changing VM state.
        let mut next = vm.state().clone();
        next.set_string(self.index, &text).map_err(error)?;
        Ok((accepted, text))
    }

    pub fn store(&self, vm: &mut Machine, (accepted, text): (bool, Vec<u8>)) {
        vm.state_mut()
            .set_string(self.index, &text)
            .expect("validated string slot");
        vm.state_mut()
            .set_system_integer(0, u32::from(accepted))
            .expect("slot zero exists");
    }
}

/// CVNS 0x21b0 (0x4652a0 -> exit confirmation dialog) and 0x21b1 (0x4652f0 -> MessageBoxA).
pub(in super::super) fn cvns_message_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    use crate::platform::MessageDialogResult;
    let arguments = if call.command == 0x5b0 {
        None
    } else {
        Some([
            call.argument(4, "dialog mode")?,
            call.argument(8, "dialog caption")?,
            call.argument(12, "dialog text")?,
        ])
    };
    let bytes = if arguments.is_some() { 12 } else { 0 };
    if hosts.in_game_message_dialogs {
        return Ok(call.defer(RuntimeOperation::CvnsMessageDialog { arguments }, bytes)?);
    }
    let request = cvns_message_request(call.vm, arguments)?;
    let answer = hosts
        .device_frame
        .as_deref_mut()
        .expect("message dialog backend checked")
        .poll_message_dialog(&request)?;
    let accepted = match answer {
        MessageDialogResult::Pending => return Ok(Flow::Wait),
        MessageDialogResult::Cancelled => false,
        MessageDialogResult::Accepted => true,
    };
    call.complete(0x4000 | bytes as u16)?;
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(accepted))
        .expect("slot zero exists");
    Ok(Flow::Done)
}

pub(crate) fn cvns_message_request(
    vm: &crate::machine::Machine,
    arguments: Option<[u32; 3]>,
) -> Result<crate::platform::MessageDialogRequest, crate::bytecode::DecodeError> {
    use crate::platform::{MessageDialogKind, MessageDialogRequest};
    if let Some([mode, caption, text]) = arguments {
        return Ok(MessageDialogRequest {
            kind: if mode == 0 {
                MessageDialogKind::Alert
            } else {
                MessageDialogKind::Confirm
            },
            caption: vm.resolve_string(caption)?,
            text: vm.resolve_string(text)?,
        });
    }
    let (caption, text): (&[u8], &[u8]) = match vm.text_encoding() {
        crate::TextEncoding::Utf8 => (
            "ゲーム終了の確認".as_bytes(),
            "ゲームを終了させますか？".as_bytes(),
        ),
        // 退出确认 / 确定退出游戏吗？
        crate::TextEncoding::Gbk => (
            b"\xcd\xcb\xb3\xf6\xc8\xb7\xc8\xcf",
            b"\xc8\xb7\xb6\xa8\xcd\xcb\xb3\xf6\xd3\xce\xcf\xb7\xc2\xf0\xa3\xbf",
        ),
        crate::TextEncoding::ShiftJis => (
            b"\x83\x51\x81\x5b\x83\x80\x8f\x49\x97\xb9\x82\xcc\x8a\x6d\x94\x46",
            b"\x83\x51\x81\x5b\x83\x80\x82\xf0\x8f\x49\x97\xb9\x82\xb3\x82\xb9\x82\xdc\x82\xb7\x82\xa9\x81\x48",
        ),
    };
    Ok(MessageDialogRequest {
        kind: MessageDialogKind::Confirm,
        caption: caption.to_vec(),
        text: text.to_vec(),
    })
}

/// CVNS 0x21b7 (cvns-20040730 0x430820 -> ChooseColorA): the two RGB words seed
/// custom swatches, and only confirmation replaces system integer 4.
pub(in super::super) fn cvns_color_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let swatches = [
        call.argument(4, "first custom color")? & 0xff_ffff,
        call.argument(8, "second custom color")? & 0xff_ffff,
    ];
    if hosts.runtime {
        return Ok(call.defer(RuntimeOperation::CvnsColorPicker { swatches }, 8)?);
    }
    let color = hosts
        .device_frame
        .as_deref_mut()
        .expect("color dialog backend checked")
        .choose_color(swatches)?;
    call.complete(0x4008)?;
    store_color_result(call.vm.state_mut(), color);
    Ok(Flow::Done)
}

pub(crate) fn store_color_result(state: &mut crate::evaluate::State, color: Option<u32>) {
    state
        .set_system_integer(0, u32::from(color.is_some()))
        .expect("slot zero exists");
    if let Some(color) = color {
        state
            .set_system_integer(4, color & 0xff_ffff)
            .expect("slot four exists");
    }
}

/// 0x213d / 0x140081520 seeds the modal dialog with the global face (+3160).
/// An accepted face updates the setting and reselects only texts using the default;
/// cancellation still completes with no arguments or result writes.
pub(in super::super) fn root_font_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let settings = hosts
        .settings
        .as_deref_mut()
        .expect("font settings checked");
    let messages = hosts
        .messages
        .as_deref_mut()
        .expect("font messages checked");
    let chosen = hosts
        .device_frame
        .as_deref_mut()
        .expect("font backend checked")
        .poll_font_dialog(settings.font_face())?;
    let chosen = match chosen {
        crate::platform::FontDialogResult::Pending => return Ok(Flow::Wait),
        crate::platform::FontDialogResult::Cancelled => None,
        crate::platform::FontDialogResult::Selected(face) => Some(face),
    };
    let mut next = settings.clone();
    if let Some(face) = &chosen {
        next.set_font_face(face).map_err(|why| call.error(why))?;
    }
    call.complete(0x4000)?;
    if chosen.is_some() {
        *settings = next;
        messages.set_default_face(settings.font_face());
        messages.refresh_default_fonts();
    }
    Ok(Flow::Done)
}

pub(in super::super) fn root_capture(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .capture
        .as_deref_mut()
        .expect("capture capability checked")
        .handle(
            call,
            hosts.scene.as_deref_mut().expect("capture scene checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_capture_device(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .capture
        .as_deref_mut()
        .expect("capture capability checked")
        .handle_immediate(
            call,
            hosts.scene.as_deref_mut().expect("capture scene checked"),
            hosts
                .device_frame
                .as_deref_mut()
                .expect("capture device checked"),
        )
}

pub(in super::super) fn window_full_move(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .platform
        .as_mut()
        .expect("platform capability checked")
        .host
        .handle_window_full_move(call)?;
    Ok(Flow::Done)
}

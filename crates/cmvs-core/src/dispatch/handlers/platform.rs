//! Window services, dialogs, and frame capture.
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in crate::dispatch) fn root_platform(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .platform
        .as_mut()
        .expect("platform capability checked");
    Ok(Flow::Platform(
        h.host.handle(call, h.backend, h.assets, h.cursors)?,
    ))
}

/// Old 0x213c returns a full-width comment and a confirmation result.
pub(in crate::dispatch) fn variant_text_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    use crate::platform::{TextDialogRequest, TextDialogResult};
    let mut strings = Vec::new();
    for distance in [4, 8, 12, 20] {
        let raw = call.argument(distance, "text dialog string")?;
        strings.push(call.vm.resolve_string(raw)?);
    }
    let output = call.argument(20, "text dialog output")?;
    let crate::vm::StringRef::StringBank(index) = crate::vm::StringRef::decode(output) else {
        return Err(call
            .error("text dialog output requires a writable string slot")
            .into());
    };
    let index = if index > 127 { 0 } else { index };
    let request = TextDialogRequest {
        title: strings[0].clone(),
        prompt: strings[1].clone(),
        initial: strings[2].clone(),
        max_bytes: call.argument(16, "text dialog character limit")?,
    };
    // The inspected old helper ignores this sixth flag after the wrapper normalizes it.
    call.argument(24, "text dialog flag")?;
    if request.title.len() >= 144 || request.prompt.len() >= 256 || request.initial.len() >= 256 {
        return Err(call
            .error("text dialog strings exceed native capacity")
            .into());
    }
    let answer = hosts
        .device_frame
        .as_deref_mut()
        .expect("text dialog backend checked")
        .poll_text_dialog(&request)?;
    let (accepted, text) = match answer {
        TextDialogResult::Pending => return Ok(Flow::Wait),
        TextDialogResult::Cancelled => (false, request.initial.clone()),
        TextDialogResult::Accepted(text) => (true, text),
    };
    if text.len() >= 256 || text.contains(&0) {
        return Err(call
            .error("text dialog result exceeds native capacity")
            .into());
    }
    // 0x403b10 keeps only encoded double-byte characters before the empty fallback.
    let text = if accepted {
        let mut full_width = Vec::new();
        let mut i = 0;
        while i < text.len() {
            if let Some(len) = call.vm.text_encoding().wide_len(text[i]) {
                let pair = text.get(i..i + len).ok_or_else(|| {
                    call.error("text dialog has a truncated double-byte character")
                })?;
                full_width.extend_from_slice(pair);
                i += len;
            } else {
                i += 1;
            }
        }
        if full_width.is_empty() {
            request.initial.clone()
        } else {
            full_width
        }
    } else {
        text
    };
    // Validate the write before consuming arguments or committing any VM state.
    let mut next = call.vm.state().clone();
    next.set_string(index, &text)
        .map_err(|why| call.error(why))?;
    call.complete(0x4018)?;
    call.vm
        .state_mut()
        .set_string(index, &text)
        .expect("validated string slot");
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(accepted))
        .expect("slot zero exists");
    Ok(Flow::Done)
}

/// 0x213d / 0x140081520 seeds the modal dialog with the global face (+3160).
/// An accepted face updates the setting and reselects only texts using the default;
/// cancellation still completes with no arguments or result writes.
pub(in crate::dispatch) fn root_font_dialog(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
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

pub(in crate::dispatch) fn root_capture(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
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

pub(in crate::dispatch) fn root_capture_device(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
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
        )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn window_full_move(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .platform
        .as_mut()
        .expect("platform capability checked")
        .host
        .handle_window_full_move(call)?;
    Ok(Flow::Done)
}

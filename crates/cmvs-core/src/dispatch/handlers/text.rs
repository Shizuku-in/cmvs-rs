//! Text, message windows, and message waits.
use crate::dispatch::{Call, Flow, Hosts};
use crate::message::font_cells;
use std::error::Error;

pub(in super::super) fn root_text(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_handle(call.vm)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}

pub(in super::super) fn root_output(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(
        match hosts
            .messages
            .try_record_output(call.vm, hosts.input, hosts.flags)?
        {
            Some(step) => Flow::from_step(step, call.vm.pending_system().is_some()),
            None => Flow::Unhandled,
        },
    )
}

pub(in super::super) fn root_poll(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_record_poll(call.vm, hosts.flags)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}

pub(in super::super) fn root_text_state(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("message capability checked")
        .handle_text_state(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_auxiliary_scene(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(hosts
        .messages
        .as_deref_mut()
        .expect("message capability checked")
        .handle_auxiliary_scene(call, hosts.scene.as_deref_mut())?)
}

pub(in super::super) fn root_windows(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_window_pointer(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_pointer(
            call,
            hosts
                .input
                .as_ref()
                .expect("input capability checked")
                .input,
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_window_buttons(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_buttons(
            call,
            hosts
                .input
                .as_mut()
                .expect("input capability checked")
                .input,
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_window_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_window_image(
            call,
            hosts
                .window_images
                .as_deref_mut()
                .expect("image source capability checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_message_wait(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .message_wait
        .as_mut()
        .expect("message wait capability checked");
    Ok(h.messages.handle_wait(call, h.input, h.flags, h.context)?)
}

pub(in super::super) fn root_cvns_messages(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_cvns(call)?)
}

pub(in super::super) fn root_cvns_message_buttons(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_cvns_input(
            call,
            hosts
                .input
                .as_mut()
                .expect("input capability checked")
                .input,
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_cvns_message_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .messages
        .as_deref_mut()
        .expect("window capability checked")
        .handle_cvns_image(
            call,
            hosts
                .window_images
                .as_deref_mut()
                .expect("image source capability checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_cvns_message_wait(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .message_wait
        .as_mut()
        .expect("message wait capability checked");
    Ok(h.messages
        .handle_cvns_wait(call, h.input, h.flags, h.context)?)
}

pub(in super::super) fn auxiliary_measure(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::auxiliary_measure(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_direct_text(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::direct_text(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_measure_text(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::measure_text(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_append_window(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::append_window(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_direct_window(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::direct_window(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_poll_window(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::poll_window(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_auxiliary_face(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::auxiliary_face(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_auxiliary_text(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::auxiliary_text(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_set_mode(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::set_mode(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

pub(in super::super) fn font_auxiliary_line_break(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(font_cells::auxiliary_line_break(
        hosts.text.as_mut().expect("text capability checked"),
        call,
    )?)
}

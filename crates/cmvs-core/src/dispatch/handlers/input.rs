//! Input, gestures, buttons, and function-key policy.
use crate::dispatch::FunctionKeys;
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in super::super) fn root_input(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    if matches!(call.command, 0x2ef | 0x447) {
        crate::input::handle_skipping(call, h.input, h.skip)?;
    } else {
        crate::input::handle(call, h.input)?;
    }
    Ok(Flow::Done)
}

pub(in super::super) fn root_buttons(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_button_input(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle_input(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
            h.input,
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_list_input(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle_list_input(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
            h.input,
            hosts
                .gesture
                .as_deref()
                .expect("gesture capability checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_drag_press(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .gesture
        .as_deref_mut()
        .expect("gesture capability checked")
        .handle_drag_press(call, h.input, h.clock.expect("clock capability checked"))?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_gesture(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .gesture
        .as_deref_mut()
        .expect("gesture capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_function_keys(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    match hosts
        .function_keys
        .as_mut()
        .expect("function key capability checked")
    {
        FunctionKeys::Runtime(host) => Ok(host.handle(call)?),
        FunctionKeys::Input(input) => {
            crate::input::handle(call, input)?;
            Ok(Flow::Done)
        }
    }
}

/// Static variant tables used when provisioning shared host capabilities.
pub(in super::super) fn variant_input_press(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    crate::input::handle_press(call, h.input)?;
    Ok(Flow::Done)
}

pub(in super::super) fn select_list(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.input.as_mut().expect("input capability checked");
    hosts
        .buttons
        .as_deref_mut()
        .expect("buttons capability checked")
        .handle_list_select(
            call,
            hosts
                .scene
                .as_deref_mut()
                .expect("scene capability checked"),
            h.input,
        )?;
    Ok(Flow::Done)
}

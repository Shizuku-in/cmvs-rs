//! Startup configuration, settings, history, and scalar services.
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in crate::dispatch) fn root_timers(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .timers
        .as_deref_mut()
        .expect("timers capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_flags(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .flags
        .as_deref_mut()
        .expect("flags capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_random(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .random
        .as_deref_mut()
        .expect("random capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_settings(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let settings = hosts
        .settings
        .as_deref_mut()
        .expect("settings capability checked");
    let display_name = if call.command == 0x2de && call.argument(4, "mode")? == 0 {
        hosts
            .messages
            .as_deref()
            .and_then(|messages| messages.display_face_name(settings.font_face()))
    } else {
        None
    };
    settings.handle_with_font_name(call, display_name.as_deref())?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_history(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .history
        .as_deref_mut()
        .expect("history capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_strings(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    crate::text::handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_math(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    crate::math::handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_bootstrap(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    Ok(hosts
        .bootstrap
        .as_deref_mut()
        .expect("bootstrap capability checked")
        .handle(call)?)
}

pub(in crate::dispatch) fn polar_point(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    crate::math::handle_polar_point(call)?;
    Ok(Flow::Done)
}

/// cmvs-20070119 0x22c0 (0x43d310) / 0x22c1 (0x43d3a0) read SP-4 without other
/// arguments, return 0x4004 and set the system result to `SP-4 == 1 && check`, where the
/// check verifies the retail activation file `comap.dat` (0x22c0 first runs `reg.exe`).
/// The activation and disc checks are deliberately not reimplemented: the check is
/// reported as passed, with no file, process, registry or drive access.
pub(in crate::dispatch) fn activation_pass(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let mode = call.argument(4, "check mode")?;
    call.complete(0x4004)?;
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(mode == 1))
        .expect("slot zero exists");
    Ok(Flow::Done)
}

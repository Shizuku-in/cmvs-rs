//! Startup configuration, settings, history, and scalar services.
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in super::super) fn root_timers(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .timers
        .as_deref_mut()
        .expect("timers capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_flags(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .flags
        .as_deref_mut()
        .expect("flags capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_random(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .random
        .as_deref_mut()
        .expect("random capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_settings(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
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

pub(in super::super) fn root_history(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .history
        .as_deref_mut()
        .expect("history capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_strings(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    crate::text::handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_math(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    crate::math::handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_bootstrap(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    Ok(hosts
        .bootstrap
        .as_deref_mut()
        .expect("bootstrap capability checked")
        .handle(call)?)
}

pub(in super::super) fn polar_point(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    crate::math::handle_polar_point(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn cartesian_to_polar(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    crate::math::handle_cartesian_to_polar(call)?;
    Ok(Flow::Done)
}

/// CVNS timing and wall-clock queries/switches (`0x403`, `0x404`, `0x407`, `0x41c`, `0x41f`).
pub(in super::super) fn cvns_timers(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    match call.command {
        0x403 => {
            let interval = hosts
                .settings
                .as_deref()
                .expect("settings checked")
                .cvns()
                .frame_interval_ms;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, interval)
                .expect("slot 0 exists");
        }
        0x404 => {
            let fixed = call.argument(4, "wait mode")? != 0;
            call.complete(0x4004)?;
            hosts
                .settings
                .as_deref_mut()
                .expect("settings checked")
                .cvns_mut()
                .fixed_frame_timing = fixed;
        }
        0x407 => {
            let delta = hosts.timers.as_deref().expect("timers checked").delta_ms();
            let fps = 1000u32.checked_div(delta).map_or(60, |v| v.max(1));
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, fps)
                .expect("slot 0 exists");
        }
        0x41c => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let [year, month, day, hour, minute, second] =
                crate::runtime::save_time_from_unix(secs);
            let date = u32::from(year) * 10_000 + u32::from(month) * 100 + u32::from(day);
            let time = u32::from(hour) * 10_000 + u32::from(minute) * 100 + u32::from(second);
            let weekday = ((secs / 86_400 + 4) % 7) as u32;
            call.complete(0x4000)?;
            let state = call.vm.state_mut();
            state.set_system_integer(4, date).expect("slot 4 exists");
            state.set_system_integer(5, time).expect("slot 5 exists");
            state.set_system_integer(6, weekday).expect("slot 6 exists");
        }
        0x41f => {
            let enabled = call.argument(4, "frame sync")? != 0;
            call.complete(0x4004)?;
            hosts
                .settings
                .as_deref_mut()
                .expect("settings checked")
                .cvns_mut()
                .sync_redraw = enabled;
        }
        _ => unreachable!("registered CVNS timer command"),
    }
    Ok(Flow::Done)
}

/// cmvs-20070119 0x22c0 (0x43d310) / 0x22c1 (0x43d3a0) and CVNS 0x2017 (0x4696e0) /
/// 0x202e (0x469570) report activation and disc protection checks as passed without
/// file, process, registry, or drive access.
pub(in super::super) fn activation_pass(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let mode = call.argument(4, "check mode")?;
    let passed = match call.command {
        0x417 => matches!(mode, 2 | 3 | 5 | 6),
        0x42e => matches!(mode, 1 | 2 | 3 | 5 | 6),
        _ => mode == 1,
    };
    call.complete(0x4004)?;
    call.vm
        .state_mut()
        .set_system_integer(0, u32::from(passed))
        .expect("slot zero exists");
    Ok(Flow::Done)
}

/// CVNS unified configuration read (`0x5ec` = `0x21ec`) and write (`0x5ed` = `0x21ed`).
pub(in super::super) fn cvns_config(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    if call.command == crate::cvns::cmd::GET_CONFIG {
        cvns_get_config(hosts, call)?;
    } else {
        cvns_set_config(hosts, call)?;
    }
    Ok(Flow::Done)
}

fn cvns_get_config(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let id = call.argument(4, "option id")?;
    if id == 15 {
        let str_ref = call.argument(8, "font buffer")?;
        let crate::vm::StringRef::StringBank(index) = crate::vm::StringRef::decode(str_ref) else {
            return Err(call
                .error("font destination outside the string bank is not modeled")
                .into());
        };
        let slot = if index >= 256 { 0 } else { index };
        let settings = hosts.settings.as_deref().expect("settings checked");
        let face = hosts
            .messages
            .as_deref()
            .and_then(|messages| messages.display_face_name(settings.font_face()))
            .unwrap_or_else(|| settings.font_face().to_vec());
        let mut next = call.vm.state().clone();
        next.set_string(slot, &face)
            .map_err(|why| call.error(why))?;
        call.complete(0x4008)?;
        call.vm
            .state_mut()
            .set_string(slot, &face)
            .expect("validated string slot");
        call.vm
            .state_mut()
            .set_system_integer(0, 1)
            .expect("slot zero exists");
        return Ok(());
    }

    call.ignore_argument(8, "unused font buffer")?;
    let settings = hosts.settings.as_deref().expect("settings checked");
    let audio = hosts.audio.as_ref().expect("audio checked");
    let popup = hosts.popup.as_deref().expect("popup checked");
    let flags = hosts.flags.as_deref().expect("flags checked");
    let effects = *hosts.effects.as_deref().expect("effects checked");
    let value = match id {
        1 => Some(u32::from(effects)),
        2 => Some(u32::from(settings.cvns().wait_timing)),
        3 => Some(u32::from(popup.confirmations()[0])),
        4 => Some(u32::from(popup.confirmations()[1])),
        5 => Some(u32::from(settings.active_draw())),
        6 => Some(settings.cvns().cpu_priority),
        7 => Some(u32::from(audio.audio.settings().bgm_enabled)),
        8 => Some(u32::from(audio.audio.settings().se_enabled)),
        9 => Some(u32::from(audio.audio.settings().voice_enabled)),
        10..=14 => Some(audio.audio.cvns_volume((id - 10) as usize)),
        16 => Some(u32::from(settings.cvns().text_shadow)),
        17 => Some(settings.text_output().speed),
        18 => Some(settings.text_output().reveal_mode),
        19 => Some(settings.text_output().reveal_duration),
        20 => Some(flags.mode_1548()),
        21 => Some(u32::from(settings.read_skip())),
        22 => Some(u32::from(settings.cvns().auto_skip)),
        23 => Some(u32::from(flags.auto_advance())),
        24 => Some(u32::from(settings.auto_advance().sync_voice)),
        25 => Some(settings.auto_advance().base),
        26 => Some(settings.auto_advance().per_glyph),
        27 => Some(u32::from(settings.cvns().mouse_right_mode)),
        28 => Some(settings.cvns().wheel_mode),
        29 => Some(settings.cursor_auto_clear()),
        30 => Some(u32::from(settings.cvns().auto_move)),
        31 => Some(settings.cvns().frame_ctrl),
        999 => Some(u32::from(flags.cvns_read_skipping())),
        _ => None,
    };
    call.complete(0x4008)?;
    if let Some(value) = value {
        call.vm
            .state_mut()
            .set_system_integer(0, value)
            .expect("slot zero exists");
    }
    Ok(())
}

fn cvns_set_config(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let id = call.argument(4, "option id")?;
    if id == 15 {
        let str_ref = call.argument(8, "font face")?;
        call.ignore_argument(12, "unused value")?;
        call.ignore_argument(16, "reserved")?;
        let face = call.vm.resolve_string(str_ref)?;
        let settings = hosts.settings.as_deref_mut().expect("settings checked");
        let mut next = settings.clone();
        next.set_font_face(&face).map_err(|why| call.error(why))?;
        call.complete(0x4010)?;
        *settings = next;
        if let Some(messages) = hosts.messages.as_deref_mut() {
            messages.set_default_face(settings.font_face());
            messages.refresh_default_fonts();
        }
        return Ok(());
    }

    call.ignore_argument(8, "unused font")?;
    let raw = call.argument(12, "option value")?;
    call.ignore_argument(16, "reserved")?;
    let val = raw as i32;
    if matches!(id, 7..=9) {
        let cmd = match id {
            7 => 0x2d6,
            8 => 0x2d7,
            _ => 0x2d8,
        };
        let audio = hosts.audio.as_mut().expect("audio checked");
        audio
            .audio
            .cvns_set_enabled(cmd, raw != 0, call.pc, audio.backend)?;
        call.complete(0x4010)?;
        return Ok(());
    }
    if matches!(id, 10..=14) {
        let kind = (id - 10) as usize;
        let audio = hosts.audio.as_mut().expect("audio checked");
        let (percent, next_audio) = audio.audio.stage_cvns_volume(kind, val, audio.backend)?;
        call.complete(0x4010)?;
        audio.audio.commit_cvns_volume(kind, percent, next_audio);
        return Ok(());
    }

    call.complete(0x4010)?;
    let settings = hosts.settings.as_deref_mut().expect("settings checked");
    let popup = hosts.popup.as_deref_mut().expect("popup checked");
    let flags = hosts.flags.as_deref_mut().expect("flags checked");
    let effects = hosts.effects.as_deref_mut().expect("effects checked");
    match id {
        1 => *effects = raw != 0,
        2 => settings.cvns_mut().wait_timing = val >= 0 && (val & 1) != 0,
        3 => {
            let [_, save] = popup.confirmations();
            popup.set_confirmations(raw != 0, save);
        }
        4 => {
            let [load, _] = popup.confirmations();
            popup.set_confirmations(load, raw != 0);
        }
        5 => settings.set_active_draw(raw != 0),
        6 => settings.cvns_mut().cpu_priority = (val.max(0) as u32) % 3,
        16 => settings.cvns_mut().text_shadow = raw != 0,
        17 => settings.set_text_speed(val.clamp(0, 4) as u32),
        18 => settings.set_reveal_mode(val.clamp(0, 2) as u32),
        19 => settings.set_reveal_duration(val.clamp(0, 4) as u32),
        20 => flags.set_mode_1548(val.clamp(0, 2) as u32),
        21 => settings.set_read_skip(raw != 0),
        22 => {
            settings.cvns_mut().auto_skip = raw != 0;
            flags.set_cvns_read_skipping(false);
        }
        23 => flags.set_auto_advance(raw != 0),
        24 => settings.set_auto_voice_sync(raw != 0),
        25 => settings.set_auto_base(raw),
        26 => settings.set_auto_per_glyph(raw),
        27 => settings.cvns_mut().mouse_right_mode = raw != 0,
        28 => settings.cvns_mut().wheel_mode = raw,
        29 => settings.set_cvns_cursor_auto_clear(raw),
        30 => settings.cvns_mut().auto_move = raw != 0,
        31 => settings.cvns_mut().frame_ctrl = val.clamp(0, 2) as u32,
        999 => flags.set_cvns_read_skipping(raw != 0),
        _ => {}
    }
    Ok(())
}

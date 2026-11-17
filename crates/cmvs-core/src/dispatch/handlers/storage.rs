//! Script control, snapshots, save data, and deferred runtime operations.
use crate::dispatch::RuntimeOperation;
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in super::super) fn root_runtime(
    _hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    use RuntimeOperation::*;
    let (operation, bytes) = match call.command {
        1 => (Quit, 0),
        3 => (Reset, 0),
        0x2b5 => (
            LoadSlot {
                slot: call.argument(4, "save slot")?,
            },
            4,
        ),
        0x2b6 => (
            SaveSlot {
                slot: call.argument(4, "save slot")?,
            },
            4,
        ),
        0x28c | 0x28e => (
            EditSlots {
                first: call.argument(4, "first slot")?,
                second: call.argument(8, "second slot")?,
            },
            8,
        ),
        0x2bc | 0x5ab => (
            SetSlotComment {
                slot: call.argument(4, "save slot")?,
                text: call.argument(8, "comment")?,
            },
            8,
        ),
        0x2b8 => (Thumbnail, 0),
        0x2be => (
            SetSlotInfo {
                slot: call.argument(4, "save slot")?,
                values: [
                    call.argument(8, "first metadata word")?,
                    call.argument(12, "second metadata word")?,
                ],
            },
            12,
        ),
        0x2b7 => (
            LatestSlot {
                start: call.argument(4, "first slot")? as i32,
                count: call.argument(8, "slot count")? as i32,
            },
            8,
        ),
        0x2bd => {
            let (slot, object, child) = (
                call.argument(4, "save slot")? as i32,
                call.argument(8, "object slot")?,
                call.argument(12, "child slot")? as i32,
            );
            let label_slot = crate::text::bank_slot(call.argument(16, "label destination")?)
                .ok_or_else(|| {
                    call.error("label destination outside the string bank is not modeled")
                })?;
            let text_slot = crate::text::bank_slot(call.argument(20, "text destination")?)
                .ok_or_else(|| {
                    call.error("text destination outside the string bank is not modeled")
                })?;
            (
                SlotInfo {
                    slot,
                    object,
                    child,
                    label_slot,
                    text_slot,
                },
                20,
            )
        }
        crate::cvns::cmd::SLOT_INFO => {
            let slot = call.argument(4, "save slot")? as i32;
            let img_slot = call.argument(8, "CVNS image slot")?;
            let label_arg = call.argument(12, "label destination")?;
            let text_arg = call.argument(16, "text destination")?;
            let state = call.vm.state();
            let label_slot = crate::text::bank_slot_for(state, label_arg).ok_or_else(|| {
                call.error("label destination outside the string bank is not modeled")
            })?;
            let text_slot = crate::text::bank_slot_for(state, text_arg).ok_or_else(|| {
                call.error("text destination outside the string bank is not modeled")
            })?;
            (
                CvnsSlotInfo {
                    slot,
                    img_slot,
                    label_slot,
                    text_slot,
                },
                16,
            )
        }
        _ => unreachable!("registered runtime command"),
    };
    Ok(call.defer(operation, bytes)?)
}

pub(in super::super) fn root_snapshot(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .snapshot
        .as_mut()
        .expect("snapshot capability checked");
    if h.capture {
        return Ok(if hosts.runtime {
            call.defer(RuntimeOperation::CaptureSnapshot, 0)?
        } else {
            Flow::Unavailable
        });
    }
    if !h.continue_without_image {
        return Ok(Flow::Unavailable);
    }
    h.image.handle_unavailable(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_script_control(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts.scripts.as_mut().expect("script capability checked");
    Ok(Flow::Control(h.host.handle(call, h.source)?))
}

pub(in super::super) fn root_system_read(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .system_read
        .as_mut()
        .expect("system read capability checked");
    Ok(Flow::SystemSave(if call.command == 0x5c0 {
        h.host.handle_css0_read(call, h.source, h.persistent)?
    } else {
        h.host.handle_read(call, h.source, h.game_id)?
    }))
}

pub(in super::super) fn root_system_write(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    let h = hosts
        .system_write
        .as_mut()
        .expect("system write capability checked");
    h.host.handle_write(call, h.sink, h.random, h.game_id)?;
    Ok(Flow::SystemWritten)
}

pub(in super::super) fn root_save_menu(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .popup
        .as_deref_mut()
        .expect("save menu checked")
        .handle(
            call,
            hosts
                .save_source
                .as_deref_mut()
                .expect("save source checked"),
        )?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_popup(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .popup
        .as_deref_mut()
        .expect("popup capability checked")
        .handle_popup(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_save_title(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .save_title
        .as_deref_mut()
        .expect("save_title capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in super::super) fn root_save_info(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_>,
) -> Result<Flow, Box<dyn Error + Send + Sync>> {
    hosts
        .save_info
        .as_deref_mut()
        .expect("save_info capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

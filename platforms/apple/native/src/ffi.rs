//! The C header defines the pointer lifetimes and serialized ownership contract.
use crate::{Config, Player, Result, inspect, renderer::Frame};
use cmvs_core::{
    host::platform::{FontDialogResult, MessageDialogResult},
    render::raster::RgbaImage,
};
use cmvs_font::{font::FontRenderer, text_codec};
use cmvs_input::{TouchEvent, TouchPhase};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr,
};

#[derive(Default)]
struct Handle {
    player: Option<Player>,
    text: CString,
    snapshot: Option<RgbaImage>,
}
impl Handle {
    fn player(&mut self) -> Result<&mut Player> {
        self.player.as_mut().ok_or_else(|| "no open game".into())
    }
    fn output(&mut self, v: Value) -> Result<i32> {
        self.text = CString::new(v.to_string())?;
        Ok(0)
    }
}
fn string(p: *const c_char) -> Result<String> {
    if p.is_null() {
        return Err("missing UTF-8 argument".into());
    }
    // SAFETY: the caller supplies a terminated string valid for this call.
    Ok(unsafe { CStr::from_ptr(p) }.to_str()?.into())
}
fn call(h: *mut Handle, f: impl FnOnce(&mut Handle) -> Result<i32>) -> i32 {
    // SAFETY: caller owns the live handle exclusively until this call returns.
    let Some(h) = (unsafe { h.as_mut() }) else {
        return -1;
    };
    match catch_unwind(AssertUnwindSafe(|| f(h))) {
        Ok(Ok(n)) => n,
        result => {
            let error = match result {
                Ok(Err(e)) => e.to_string(),
                _ => "panic in Apple bridge".into(),
            };
            h.text = CString::new(error.replace('\0', "�")).unwrap_or_default();
            -1
        }
    }
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_create() -> *mut Handle {
    Box::into_raw(Box::default())
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_destroy(h: *mut Handle) {
    if !h.is_null() {
        // SAFETY: caller transfers the unique allocation returned by create exactly once.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(h) })));
    }
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_text(h: *const Handle) -> *const c_char {
    // SAFETY: the handle remains live and the caller copies before mutating it.
    unsafe { h.as_ref() }.map_or(ptr::null(), |h| h.text.as_ptr())
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_list_profiles(h: *mut Handle) -> i32 {
    call(h, |h| h.output(inspect::profiles()))
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_inspect_game(
    h: *mut Handle,
    path: *const c_char,
    profile: *const c_char,
    encoding: *const c_char,
) -> i32 {
    call(h, |h| {
        let profile = (!profile.is_null()).then(|| string(profile)).transpose()?;
        let encoding = if encoding.is_null() {
            "auto".to_owned()
        } else {
            string(encoding)?
        };
        h.output(inspect::inspect(
            Path::new(&string(path)?),
            profile.as_deref(),
            &encoding,
        )?)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_validate_font(h: *mut Handle, path: *const c_char, face: u32) -> i32 {
    call(h, |_| {
        FontRenderer::read(Path::new(&string(path)?), face)?;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_open(h: *mut Handle, config: *const c_char) -> i32 {
    call(h, |h| {
        if h.player.is_some() {
            return Err("game already open".into());
        }
        let p = Player::open(serde_json::from_str::<Config>(&string(config)?)?)?;
        let info = json!({"size":p.backend.size,"encoding":text_codec::name(p.session.encoding),"mode":p.backend.mode(),"fullscreen":p.backend.display.fullscreen,"linear":p.backend.display.render_mode==1});
        h.player = Some(p);
        h.output(info)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_attach_metal_layer(
    h: *mut Handle,
    layer: *mut c_void,
    width: u32,
    height: u32,
) -> i32 {
    call(h, |h| {
        if layer.is_null() {
            return Err("missing Metal layer".into());
        }
        let p = h.player()?;
        p.output = [width, height];
        #[cfg(target_vendor = "apple")]
        {
            use cmvs_wgpu::wgpu::{Backends, Instance, InstanceDescriptor, SurfaceTargetUnsafe};
            let instance = Instance::new(InstanceDescriptor {
                backends: Backends::METAL,
                ..InstanceDescriptor::new_without_display_handle()
            });
            // SAFETY: the Swift view retains its CAMetalLayer until synchronous detach.
            let surface = unsafe {
                instance.create_surface_unsafe(SurfaceTargetUnsafe::CoreAnimationLayer(layer))
            }?;
            p.metal = Some(crate::renderer::Metal::new(surface, &instance)?);
            Ok(0)
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            Err("Metal requires Apple hardware".into())
        }
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_resize(h: *mut Handle, width: u32, height: u32) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if p.output != [width, height] {
            p.output = [width, height];
            p.touch.cancel(p.session.runtime.input_mut());
            p.session.input.clear(p.session.runtime.input_mut());
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_fullscreen(h: *mut Handle, fullscreen: bool) -> i32 {
    use cmvs_core::host::platform::{PlatformBackend, PlatformRequest};
    call(h, |h| {
        let backend = &mut h.player()?.backend;
        if backend.display.fullscreen != fullscreen {
            backend.apply(&PlatformRequest::Fullscreen(fullscreen))?;
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_detach(h: *mut Handle) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        p.backend.frame = None;
        p.metal = None;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_tick(h: *mut Handle, delta: u32) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        p.tick(delta).inspect_err(|error| {
            let vm = p.session.runtime.machine();
            p.backend.events.push_front(json!({
                "kind": "stopped",
                "script": vm.current_name().escape_ascii().to_string(),
                "pc": vm.pc(),
                "steps": p.session.runtime.steps(),
                "reason": error.to_string()
            }));
        })
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_next_event(h: *mut Handle) -> i32 {
    call(h, |h| {
        if let Some(event) = h.player()?.backend.events.pop_front() {
            h.output(event)?;
            Ok(1)
        } else {
            Ok(0)
        }
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_key(h: *mut Handle, code: u32, pressed: bool) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if !p.paused {
            p.session.input.set_key(code as usize, pressed)?;
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_pointer(h: *mut Handle, x: f64, y: f64) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if p.touching {
            p.touch.reset_contacts();
            p.session.input.cancel_primary_button();
            p.touching = false;
        }
        let [sx, sy] = p
            .viewport()
            .and_then(|v| v.to_script([x, y]))
            .unwrap_or([-1, -1]);
        p.session.input.set_pointer(sx, sy);
        Ok(i32::from(sx >= 0))
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_touch(h: *mut Handle, id: u64, phase: u32, x: f64, y: f64) -> i32 {
    call(h, |h| {
        let phase = match phase {
            0 => TouchPhase::Started,
            1 => TouchPhase::Moved,
            2 => TouchPhase::Ended,
            3 => TouchPhase::Cancelled,
            _ => return Err("invalid touch phase".into()),
        };
        let p = h.player()?;
        p.touching = true;
        p.fresh_touch = true;
        if let Some(v) = p.viewport()
            && p.touch.pointer_event(
                TouchEvent {
                    id,
                    phase,
                    position: [x, y],
                },
                v,
            )
        {
            p.session.input.cancel_primary_button();
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
#[cfg(target_os = "ios")]
extern "C" fn cmvs_apple_restart_audio(h: *mut Handle) -> i32 {
    call(h, |h| {
        if let Some(output) = &mut h.player()?.output_audio {
            output.restart()?;
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_pause(h: *mut Handle, paused: bool) -> i32 {
    call(h, |h| {
        h.player()?.pause(paused);
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_save_preferences(h: *mut Handle) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        p.session.runtime.save_preferences(p.backend.display)?;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_comment(h: *mut Handle, id: u32, text: *const c_char) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        let answer = if text.is_null() {
            None
        } else {
            let t = string(text)?;
            text_codec::encode(p.session.encoding, &t)?;
            Some(t)
        };
        p.backend.comment.submit(id, answer);
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_font(h: *mut Handle, path: *const c_char, face: u32) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        p.backend.font = Some(if path.is_null() {
            FontDialogResult::Cancelled
        } else {
            FontDialogResult::Selected(
                p.session
                    .font
                    .import_font(Path::new(&string(path)?), face)?,
            )
        });
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_message(h: *mut Handle, accept: bool) -> i32 {
    call(h, |h| {
        h.player()?.backend.message = Some(if accept {
            MessageDialogResult::Accepted
        } else {
            MessageDialogResult::Cancelled
        });
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_read_frame(h: *mut Handle, pixels: *mut *const u8) -> i32 {
    call(h, |h| {
        if pixels.is_null() {
            return Err("missing pixel destination".into());
        }
        let frame = h
            .player
            .as_ref()
            .and_then(|p| p.backend.frame.as_ref())
            .ok_or("no completed frame")?;
        let image = match frame {
            Frame::Cpu(image) => image,
            Frame::Metal(frame) => h.snapshot.insert(frame.read()?),
        };
        // SAFETY: caller supplies writable pointer storage and copies pixels before mutation.
        unsafe { *pixels = image.pixels().as_ptr() };
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_backup(
    h: *mut Handle,
    game: *const c_char,
    user: *const c_char,
    local: bool,
    path: *const c_char,
    import: bool,
) -> i32 {
    call(h, |_| {
        crate::backup::run(
            Path::new(&string(game)?),
            Path::new(&string(user)?),
            local,
            Path::new(&string(path)?),
            import,
        )?;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_comment_valid(
    h: *mut Handle,
    text: *const c_char,
    chars: u32,
    bytes: u32,
) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        let text = string(text)?;
        Ok(i32::from(
            !text.chars().any(char::is_control)
                && text.chars().count() <= chars as usize
                && text_codec::encode(p.session.encoding, &text)
                    .is_ok_and(|encoded| encoded.len() <= bytes as usize),
        ))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_inspection_keeps_error_on_its_own_handle() {
        let h = cmvs_apple_create();
        assert_eq!(
            cmvs_apple_inspect_game(h, c".".as_ptr(), c"".as_ptr(), c"latin1".as_ptr()),
            -1
        );
        assert!(string(cmvs_apple_text(h)).unwrap().contains("encoding"));
        assert_eq!(cmvs_apple_list_profiles(h), 0);
        let profiles: Value = serde_json::from_str(&string(cmvs_apple_text(h)).unwrap()).unwrap();
        assert!(
            profiles
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == "cmvs-20160201")
        );
        cmvs_apple_destroy(h);
    }
}

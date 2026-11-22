//! The C header defines the pointer lifetimes and serialized ownership contract.
use crate::{Config, Player, Result, TickInput, inspect, renderer::Frame};
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
extern "C" fn cmvs_apple_inspect_font(h: *mut Handle, path: *const c_char) -> i32 {
    call(h, |h| h.output(inspect::font(Path::new(&string(path)?))?))
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_open(h: *mut Handle, config: *const c_char) -> i32 {
    call(h, |h| {
        if h.player.is_some() {
            return Err("game already open".into());
        }
        let p = Player::open(serde_json::from_str::<Config>(&string(config)?)?)?;
        let scaled_cpu_canvas = !p.use_metal
            && p.backend
                .scale_filter
                .is_some_and(|filter| filter.linear_sampler().is_none());
        let info = json!({"size":p.backend.size,"encoding":text_codec::name(p.session.encoding),"mode":p.backend.mode(),"fullscreen":p.backend.display.fullscreen,"linear":p.backend.layer_linear(),"scaledCpuCanvas":scaled_cpu_canvas});
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
        p.backend.output = [width, height];
        #[cfg(target_vendor = "apple")]
        {
            use cmvs_wgpu::wgpu::{Backends, Instance, InstanceDescriptor, SurfaceTargetUnsafe};
            let instance = Instance::new(InstanceDescriptor {
                backends: Backends::METAL,
                ..InstanceDescriptor::new_without_display_handle()
            });
            let surface_instance = instance.clone();
            p.metal = Some(crate::renderer::Metal::new(&instance, move || {
                // SAFETY: Swift retains the CAMetalLayer until synchronous detach.
                Ok(unsafe {
                    surface_instance
                        .create_surface_unsafe(SurfaceTargetUnsafe::CoreAnimationLayer(layer))
                }?)
            })?);
            Ok(0)
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            Err("Metal requires Apple hardware".into())
        }
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_resize(
    h: *mut Handle,
    width: u32,
    height: u32,
    windowed: bool,
    resizable: bool,
    rect: *mut [i32; 4],
) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        let output = [width, height];
        if p.backend.output != output {
            p.backend.output = output;
            p.touch.reset_contacts();
            p.touching = false;
            p.fresh_touch = false;
            p.reader.cancel();
            p.session.input.cancel_pointer();
        }
        p.backend.resizable = resizable;
        if windowed {
            p.backend.windowed = output;
        }
        // SAFETY: the caller supplies storage for four consecutive signed coordinates.
        let rect = unsafe { rect.as_mut() }.ok_or("missing viewport destination")?;
        if output.contains(&0) {
            *rect = [0; 4];
        } else {
            let v = cmvs_core::render::viewport::Viewport::new(
                p.backend.size,
                output,
                p.backend.mode(),
            )?;
            *rect = [v.left as i32, v.top as i32, v.width as i32, v.height as i32];
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
    tick(h, delta, None)
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_tick_traced(h: *mut Handle, delta: u32, input: *mut TickInput) -> i32 {
    let mut trace = TickInput::default();
    let status = tick(h, delta, Some(&mut trace));
    if !input.is_null() {
        // SAFETY: the caller supplies writable storage for the duration of this call.
        unsafe { input.write(trace) };
    }
    status
}
fn tick(h: *mut Handle, delta: u32, trace: Option<&mut TickInput>) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        p.tick(delta, trace).inspect_err(|error| {
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
        let player = h.player()?;
        if let Some(mut event) = player.backend.events.pop_front() {
            if event["kind"] == "font"
                && let Some((name, id)) = event["face"]
                    .as_str()
                    .and_then(|face| player.fonts.label(face))
            {
                event["face"] = json!(name);
                event["fontID"] = json!(id);
            }
            h.output(event)?;
            Ok(1)
        } else {
            Ok(0)
        }
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_accessibility(h: *mut Handle, known_revision: u64) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if !p.session.driver.ready() {
            return Ok(0);
        }
        p.reader.update(crate::accessibility::Page::read(
            &p.session.runtime,
            p.session.encoding,
        ));
        if known_revision == p.reader.revision {
            return Ok(0);
        }
        let page = json!({"revision":p.reader.revision, "content":p.reader.page});
        h.output(page)?;
        Ok(1)
    })
}

#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_choose(h: *mut Handle, revision: u64, index: u32, slider: i32) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if !p.paused {
            p.reader
                .choose(revision, index as usize, u8::try_from(slider).ok());
        }
        Ok(0)
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
            .backend
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
        if let Some(v) = p.backend.viewport()
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
extern "C" fn cmvs_apple_font(h: *mut Handle, id: *const c_char) -> i32 {
    call(h, |h| {
        let p = h.player()?;
        if id.is_null() {
            p.backend.font = Some(FontDialogResult::Cancelled);
            return Ok(0);
        }
        let key = p.fonts.select(&p.session.font, &string(id)?)?;
        let text = CString::new(key.as_bytes())?;
        p.backend.font = Some(FontDialogResult::Selected(key.into_bytes()));
        h.text = text;
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
extern "C" fn cmvs_apple_read_presentation(
    h: *mut Handle,
    pixels: *mut *const u8,
    size: *mut [u32; 2],
) -> i32 {
    call(h, |h| {
        if pixels.is_null() || size.is_null() {
            return Err("missing presentation destination".into());
        }
        let p = h.player.as_ref().ok_or("no open game")?;
        let Some(Frame::Cpu(frame)) = &p.backend.frame else {
            return Err("no completed software frame".into());
        };
        let filter = p
            .backend
            .scale_filter
            .ok_or("no software scaling override")?;
        let display = frame.presentation_canvas(p.backend.output, p.backend.mode(), filter)?;
        let display = h.snapshot.insert(display);
        // SAFETY: caller supplies writable pointer and size storage and copies pixels before mutation.
        unsafe {
            *pixels = display.pixels().as_ptr();
            *size = [display.width(), display.height()];
        }
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_export_saves(
    h: *mut Handle,
    local_root: *const c_char,
    user: *const c_char,
    path: *const c_char,
    identity: *const c_char,
) -> i32 {
    call(h, |_| {
        let root = (!local_root.is_null())
            .then(|| string(local_root))
            .transpose()?;
        crate::backup::export(
            root.as_deref().map(Path::new),
            Path::new(&string(user)?),
            Path::new(&string(path)?),
            &string(identity)?,
        )?;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_restore_saves(
    h: *mut Handle,
    game: *const c_char,
    profile: *const c_char,
    encoding: *const c_char,
    user: *const c_char,
    local: bool,
    path: *const c_char,
) -> i32 {
    call(h, |_| {
        let boot = inspect::bootstrap(
            Path::new(&string(game)?),
            Some(&string(profile)?),
            &string(encoding)?,
        )?;
        crate::backup::import(
            &boot,
            Path::new(&string(user)?),
            local,
            Path::new(&string(path)?),
        )?;
        Ok(0)
    })
}
#[unsafe(no_mangle)]
extern "C" fn cmvs_apple_comment_bytes(h: *mut Handle, text: *const c_char) -> i32 {
    call(h, |h| {
        let encoded = text_codec::encode(h.player()?.session.encoding, &string(text)?)?;
        Ok(i32::try_from(encoded.len())?)
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

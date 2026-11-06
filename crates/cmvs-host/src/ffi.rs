//! C callers serialize every call, keep buffers valid for the call and destroy each
//! handle once. Returned frame/error pointers last until the next mutating call.
use crate::Player;
use std::{
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
};

pub struct Handle {
    player: Option<Player>,
    error: CString,
}
impl Handle {
    fn run(
        &mut self,
        f: impl FnOnce(&mut Player) -> Result<i32, Box<dyn std::error::Error>>,
    ) -> i32 {
        let result = catch_unwind(AssertUnwindSafe(|| match self.player.as_mut() {
            Some(player) => f(player),
            None => Err("CMVS is not open".into()),
        }));
        match result {
            Ok(Ok(value)) => {
                self.error = CString::default();
                value
            }
            error => {
                let message = match error {
                    Ok(Err(e)) => e.to_string(),
                    _ => "CMVS runtime panicked".into(),
                };
                self.error = CString::new(message.replace('\0', " ")).unwrap_or_default();
                -1
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn cmvs_host_api_version() -> u32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn cmvs_host_create() -> *mut Handle {
    Box::into_raw(Box::new(Handle {
        player: None,
        error: CString::default(),
    }))
}
// SAFETY: the caller owns this handle, serializes calls, and uses live C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_open(
    handle: *mut Handle,
    game: *const c_char,
    user: *const c_char,
    font: *const c_char,
    audio: i32,
) -> i32 {
    let Some(h) = (unsafe { handle.as_mut() }) else {
        return -1;
    };
    h.player = None;
    let result = catch_unwind(AssertUnwindSafe(|| {
        if game.is_null() || user.is_null() || font.is_null() {
            return Err("null CMVS path".into());
        }
        let game = unsafe { CStr::from_ptr(game) }.to_str()?;
        let user = unsafe { CStr::from_ptr(user) }.to_str()?;
        let font = unsafe { CStr::from_ptr(font) }.to_str()?;
        Player::open(
            Path::new(game),
            Path::new(user),
            Path::new(font),
            audio != 0,
        )
    }));
    match result {
        Ok(Ok(player)) => {
            h.player = Some(player);
            h.error = CString::default();
            0
        }
        error => {
            let message = match error {
                Ok(Err(e)) => e.to_string(),
                _ => "CMVS initialization panicked".into(),
            };
            h.error = CString::new(message.replace('\0', " ")).unwrap_or_default();
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_destroy(handle: *mut Handle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_tick(handle: *mut Handle, delta_ms: u32) -> i32 {
    unsafe { handle.as_mut() }.map_or(-1, |h| h.run(|p| p.tick(delta_ms)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_key(handle: *mut Handle, code: u32, pressed: i32) -> i32 {
    unsafe { handle.as_mut() }.map_or(-1, |h| {
        h.run(|p| {
            p.key(code as usize, pressed != 0)?;
            Ok(0)
        })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_pointer(handle: *mut Handle, x: i32, y: i32) -> i32 {
    unsafe { handle.as_mut() }.map_or(-1, |h| {
        h.run(|p| {
            p.pointer(x, y);
            Ok(0)
        })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_pause(handle: *mut Handle, paused: i32) -> i32 {
    unsafe { handle.as_mut() }.map_or(-1, |h| {
        h.run(|p| {
            p.pause(paused != 0);
            Ok(0)
        })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_save_preferences(handle: *mut Handle) -> i32 {
    unsafe { handle.as_mut() }.map_or(-1, |h| {
        h.run(|p| {
            p.save_preferences()?;
            Ok(0)
        })
    })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_error(handle: *const Handle) -> *const c_char {
    unsafe { handle.as_ref() }.map_or(c"invalid CMVS handle".as_ptr(), |h| h.error.as_ptr())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_width(handle: *const Handle) -> u32 {
    unsafe { handle.as_ref() }
        .and_then(|h| h.player.as_ref())
        .map_or(0, |p| p.size()[0])
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_height(handle: *const Handle) -> u32 {
    unsafe { handle.as_ref() }
        .and_then(|h| h.player.as_ref())
        .map_or(0, |p| p.size()[1])
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_frame_serial(handle: *const Handle) -> u64 {
    unsafe { handle.as_ref() }
        .and_then(|h| h.player.as_ref())
        .map_or(0, Player::serial)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmvs_host_pixels(handle: *const Handle) -> *const u8 {
    unsafe { handle.as_ref() }
        .and_then(|h| h.player.as_ref())
        .and_then(Player::frame)
        .map_or(std::ptr::null(), |f| f.pixels().as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unopened_handles_and_null_paths_fail_without_panicking() {
        unsafe {
            assert_eq!(cmvs_host_tick(std::ptr::null_mut(), 16), -1);
            let h = cmvs_host_create();
            assert_eq!(cmvs_host_tick(h, 16), -1);
            assert_eq!(cmvs_host_width(h), 0);
            assert!(cmvs_host_pixels(h).is_null());
            assert_eq!(
                cmvs_host_open(h, std::ptr::null(), c"x".as_ptr(), c"x".as_ptr(), 0),
                -1
            );
            assert!(
                CStr::from_ptr(cmvs_host_error(h))
                    .to_str()
                    .unwrap()
                    .contains("null CMVS path")
            );
            cmvs_host_destroy(h);
        }
    }
}

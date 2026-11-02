//! Color cursors retaining Win32's (background AND mask) XOR color operation.
use crate::NativeError;
use raw_window_handle::RawWindowHandle;
use std::{cell::Cell, marker::PhantomData, ptr, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{CreateBitmap, DeleteObject},
    System::Threading::GetCurrentThreadId,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateIconIndirect, DestroyCursor, GetCursor, GetWindowThreadProcessId, HCURSOR,
            HTCLIENT, ICONINFO, IDC_ARROW, LoadCursorW, SetCursor, WM_NCDESTROY, WM_SETCURSOR,
        },
    },
};

struct State {
    window: Cell<HWND>,
    cursor: HCURSOR,
}

/// Owns a cursor and a client-area WM_SETCURSOR override. Create and drop on the
/// window's thread. Other areas retain their normal resize/title-bar cursors.
pub struct MaskedCursor {
    state: Box<State>,
    _thread: PhantomData<Rc<()>>,
}

impl MaskedCursor {
    pub fn attach(
        owner: RawWindowHandle,
        size: [usize; 2],
        hotspot: [u16; 2],
        xor_rgb: &[[u8; 3]],
        and_mask: &[bool],
    ) -> Result<Self, NativeError> {
        let RawWindowHandle::Win32(owner) = owner else {
            return Err(NativeError::Unsupported(
                "a masked cursor for a non-Win32 window",
            ));
        };
        let [width, height] = size;
        if !(1..=256).contains(&width)
            || !(1..=256).contains(&height)
            || usize::from(hotspot[0]) >= width
            || usize::from(hotspot[1]) >= height
            || xor_rgb.len() != width * height
            || and_mask.len() != xor_rgb.len()
        {
            return Err(NativeError::Failed(
                "invalid masked cursor dimensions".into(),
            ));
        }
        let window = owner.hwnd.get() as HWND;
        // SAFETY: only queries the supplied handle and the current thread; no pointers
        // are dereferenced. Reject foreign/destroyed windows before subclassing.
        #[allow(unsafe_code)]
        if unsafe { GetWindowThreadProcessId(window, ptr::null_mut()) != GetCurrentThreadId() } {
            return Err(NativeError::Failed(
                "masked cursor requires the window's thread".into(),
            ));
        }
        let cursor = create(size, hotspot, xor_rgb, and_mask)?;
        let state = Box::new(State {
            window: Cell::new(window),
            cursor,
        });
        let id = (&*state as *const State) as usize;
        // SAFETY: the stable Box remains owned until Drop removes the callback. Each
        // live box has its own ID; the !Send/!Sync guard keeps it on the window thread.
        #[allow(unsafe_code)]
        if unsafe { SetWindowSubclass(window, Some(subclass), id, id) } == 0 {
            let error = last_error("SetWindowSubclass");
            // SAFETY: this cursor is owned and has not been installed.
            #[allow(unsafe_code)]
            unsafe {
                DestroyCursor(cursor);
            }
            return Err(error);
        }
        // SAFETY: installs a live cursor owned by the returned guard. Subsequent
        // WM_SETCURSOR messages restrict the override to this window's client area.
        #[allow(unsafe_code)]
        unsafe {
            SetCursor(cursor);
        }
        Ok(Self {
            state,
            _thread: PhantomData,
        })
    }
}

fn last_error(call: &str) -> NativeError {
    NativeError::Failed(format!("{call}: {}", std::io::Error::last_os_error()))
}

fn create(
    [width, height]: [usize; 2],
    [hx, hy]: [u16; 2],
    xor_rgb: &[[u8; 3]],
    and_mask: &[bool],
) -> Result<HCURSOR, NativeError> {
    let color: Vec<u8> = xor_rgb.iter().flat_map(|&[r, g, b]| [b, g, r, 0]).collect();
    // CreateBitmap uses WORD-aligned top-down rows, unlike a CUR's DWORD-aligned DIB.
    let stride = width.div_ceil(16) * 2;
    let mut mask = vec![0u8; stride * height];
    for (i, &and) in and_mask.iter().enumerate() {
        if and {
            mask[i / width * stride + i % width / 8] |= 0x80 >> (i % width % 8);
        }
    }
    // SAFETY: validated dimensions, full top-down buffers with the required strides;
    // CreateBitmap copies them. Both bitmaps are deleted after CreateIconIndirect copies.
    #[allow(unsafe_code)]
    unsafe {
        let color = CreateBitmap(width as i32, height as i32, 1, 32, color.as_ptr().cast());
        if color.is_null() {
            return Err(last_error("CreateBitmap color"));
        }
        let mask = CreateBitmap(width as i32, height as i32, 1, 1, mask.as_ptr().cast());
        if mask.is_null() {
            let error = last_error("CreateBitmap mask");
            DeleteObject(color);
            return Err(error);
        }
        let info = ICONINFO {
            fIcon: 0,
            xHotspot: u32::from(hx),
            yHotspot: u32::from(hy),
            hbmMask: mask,
            hbmColor: color,
        };
        let cursor = CreateIconIndirect(&info);
        let error = cursor.is_null().then(|| last_error("CreateIconIndirect"));
        DeleteObject(mask);
        DeleteObject(color);
        match error {
            Some(error) => Err(error),
            None => Ok(cursor),
        }
    }
}

#[allow(unsafe_code)]
unsafe extern "system" fn subclass(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the stable Box registered by attach, removed before freeing.
    let state = unsafe { &*(data as *const State) };
    if message == WM_SETCURSOR && wparam == window as usize && lparam as u16 == HTCLIENT as u16 {
        // SAFETY: owned cursor remains live for the callback's lifetime.
        unsafe {
            SetCursor(state.cursor);
        }
        return 1;
    }
    if message == WM_NCDESTROY {
        // SAFETY: removing this callback on the owning thread before HWND reuse.
        unsafe {
            RemoveWindowSubclass(window, Some(subclass), id);
        }
        state.window.set(ptr::null_mut());
    }
    // SAFETY: forwards the unmodified message through the existing subclass chain.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

impl Drop for MaskedCursor {
    fn drop(&mut self) {
        // SAFETY: same thread as attach. A destroyed window cleared its handle in
        // WM_NCDESTROY; otherwise unregister before freeing the Box. Never destroy a
        // selected cursor, and do not replace a newer guard's cursor during swaps.
        #[allow(unsafe_code)]
        unsafe {
            let window = self.state.window.get();
            if !window.is_null() {
                RemoveWindowSubclass(
                    window,
                    Some(subclass),
                    (&*self.state as *const State) as usize,
                );
            }
            if GetCursor() == self.state.cursor {
                SetCursor(LoadCursorW(ptr::null_mut(), IDC_ARROW));
            }
            DestroyCursor(self.state.cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Graphics::Gdi::{
            BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
            DeleteDC, GdiFlush, SelectObject,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DI_NORMAL, DestroyWindow, DrawIconEx, HTLEFT, SendMessageW,
        },
    };

    #[test]
    fn masked_cursor_draws_transparent_opaque_and_inverted_pixels() {
        // Two rows and a width crossing the mask's WORD boundary exercise row packing.
        let (width, height) = (17, 2);
        let colors: Vec<_> = (0..width * height)
            .map(|i| match i % 4 {
                0 => [0, 0, 0],
                1 => [255, 255, 255],
                _ => [23, 81, i as u8],
            })
            .collect();
        let mask: Vec<_> = (0..colors.len()).map(|i| i % 4 != 2).collect();
        let cursor = create([width, height], [3, 1], &colors, &mask).unwrap();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: private memory DC and correctly sized top-down DIB, used synchronously
        // on this thread. Restore the selected bitmap before releasing GDI resources.
        #[allow(unsafe_code)]
        let actual = unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            assert!(!dc.is_null());
            let mut bits = ptr::null_mut();
            let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            assert!(!dib.is_null());
            let previous = SelectObject(dc, dib);
            let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), width * height);
            pixels.fill(0x123456);
            let drawn = DrawIconEx(
                dc,
                0,
                0,
                cursor,
                width as i32,
                height as i32,
                0,
                ptr::null_mut(),
                DI_NORMAL,
            );
            GdiFlush();
            let actual: Vec<_> = pixels.iter().map(|p| p & 0xffffff).collect();
            SelectObject(dc, previous);
            DeleteObject(dib);
            DeleteDC(dc);
            DestroyCursor(cursor);
            assert_ne!(drawn, 0);
            actual
        };
        let expected: Vec<_> = colors
            .iter()
            .zip(mask)
            .map(|(&[r, g, b], and)| {
                (if and { 0x123456 } else { 0 })
                    ^ ((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn masked_cursor_switches_and_detaches_on_window_destruction() {
        // SAFETY: creates a private hidden system-class window on this test thread;
        // messages and destruction remain on that thread, with no user input injection.
        #[allow(unsafe_code)]
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                32,
                32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!window.is_null());
            let owner = RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(
                std::num::NonZeroIsize::new(window as isize).unwrap(),
            ));
            let first = MaskedCursor::attach(owner, [1, 1], [0, 0], &[[255; 3]], &[true]).unwrap();
            let second = MaskedCursor::attach(owner, [1, 1], [0, 0], &[[128; 3]], &[true]).unwrap();
            drop(first);
            SetCursor(ptr::null_mut());
            assert_eq!(
                SendMessageW(window, WM_SETCURSOR, window as usize, HTCLIENT as isize),
                1
            );
            assert_eq!(GetCursor(), second.state.cursor);
            SendMessageW(window, WM_SETCURSOR, window as usize, HTLEFT as isize);
            assert_ne!(GetCursor(), second.state.cursor);
            drop(second);
            let third = MaskedCursor::attach(owner, [1, 1], [0, 0], &[[255; 3]], &[true]).unwrap();
            DestroyWindow(window);
            assert!(third.state.window.get().is_null());
            drop(third);
        }
    }
}

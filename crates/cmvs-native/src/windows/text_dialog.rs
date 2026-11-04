//! Small owned modal dialog; the template and callback state live until it closes.
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{
        Controls::{EM_SETLIMITTEXT, EM_SETSEL},
        WindowsAndMessaging::*,
    },
};

struct State {
    title: Vec<u16>,
    prompt: Vec<u16>,
    initial: Vec<u16>,
    limit: u32,
    answer: Option<String>,
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[allow(unsafe_code)]
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    // SAFETY: WM_INITDIALOG supplies the State pointer passed to the synchronous
    // DialogBoxIndirectParamW call. Later messages use only that same live pointer.
    unsafe {
        if message == WM_INITDIALOG {
            let state = &mut *(lparam as *mut State);
            SetWindowLongPtrW(window, GWLP_USERDATA, lparam);
            SetWindowTextW(window, state.title.as_ptr());
            SetDlgItemTextW(window, 1000, state.prompt.as_ptr());
            SetDlgItemTextW(window, 1001, state.initial.as_ptr());
            SendDlgItemMessageW(window, 1001, EM_SETLIMITTEXT, state.limit as usize, 0);
            SendDlgItemMessageW(window, 1001, EM_SETSEL, 0, -1);
            return 1;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && (wparam & 0xffff) == IDCANCEL as usize)
        {
            EndDialog(window, 0);
            return 1;
        }
        if message == WM_COMMAND && (wparam & 0xffff) == IDOK as usize {
            let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
            if !pointer.is_null() {
                let mut text = [0u16; 256];
                let len = GetDlgItemTextW(window, 1001, text.as_mut_ptr(), text.len() as i32);
                (*pointer).answer = Some(String::from_utf16_lossy(&text[..len as usize]));
                EndDialog(window, 1);
            }
            return 1;
        }
    }
    0
}

fn dword(words: &mut Vec<u16>, value: u32) {
    words.extend([value as u16, (value >> 16) as u16]);
}
fn control(words: &mut Vec<u16>, style: u32, rect: [u16; 4], id: u16, class: u16, title: &str) {
    if !words.len().is_multiple_of(2) {
        words.push(0);
    }
    dword(words, style);
    dword(words, 0);
    words.extend(rect);
    words.push(id);
    words.extend([0xffff, class]);
    words.extend(wide(title));
    words.push(0);
}
fn template() -> Vec<u32> {
    let mut words = Vec::new();
    dword(
        &mut words,
        WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | DS_MODALFRAME as u32
            | DS_SETFONT as u32
            | DS_CENTER as u32,
    );
    dword(&mut words, 0);
    words.extend([4, 0, 0, 260, 95, 0, 0, 0, 9]);
    words.extend(wide("MS Shell Dlg"));
    control(
        &mut words,
        WS_CHILD | WS_VISIBLE,
        [8, 8, 244, 24],
        1000,
        0x82,
        "",
    );
    control(
        &mut words,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        [8, 36, 244, 15],
        1001,
        0x81,
        "",
    );
    control(
        &mut words,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        [142, 66, 52, 16],
        IDOK as u16,
        0x80,
        "OK",
    );
    control(
        &mut words,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        [200, 66, 52, 16],
        IDCANCEL as u16,
        0x80,
        "Cancel",
    );
    if !words.len().is_multiple_of(2) {
        words.push(0);
    }
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u32::from(v[0]) | (u32::from(v[1]) << 16))
        .collect()
}

#[allow(unsafe_code)]
pub fn edit_text(
    owner: HWND,
    title: &str,
    prompt: &str,
    initial: &str,
    max_chars: u32,
) -> Result<Option<String>, String> {
    let template = template();
    let mut state = State {
        title: wide(title),
        prompt: wide(prompt),
        initial: wide(initial),
        limit: max_chars.min(255),
        answer: None,
    };
    // SAFETY: template is a DWORD-aligned standard dialog template with bounded,
    // NUL-terminated strings. The callback borrows state only during this modal call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            std::ptr::null_mut(),
            template.as_ptr().cast(),
            owner,
            Some(procedure),
            &mut state as *mut State as isize,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(state.answer)
}

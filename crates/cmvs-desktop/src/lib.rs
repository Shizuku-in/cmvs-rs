//! Optional native window/input adapter. Static scene rendering lives in `scene_render`.
mod app;
pub use app::run_cli;
use cmvs_core::{
    FormatError,
    cursor::CursorBitmap,
    platform::{MAX_PLATFORM_ASSET, PlatformBackend, PlatformRequest},
};
use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    io::Cursor,
};
use winit::{
    event_loop::ActiveEventLoop,
    window::{CursorIcon, CustomCursor, Icon, Window},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    ShiftJis,
    Gbk,
    Utf8,
}
impl TextEncoding {
    pub fn parse(name: &str) -> Result<Self, FormatError> {
        match name {
            "shift-jis" => Ok(Self::ShiftJis),
            "gbk" => Ok(Self::Gbk),
            "utf-8" => Ok(Self::Utf8),
            _ => Err(FormatError("encoding must be shift-jis, gbk or utf-8")),
        }
    }
    pub fn decode(self, bytes: &[u8]) -> Result<String, FormatError> {
        let encoding = match self {
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::Gbk => encoding_rs::GBK,
            Self::Utf8 => encoding_rs::UTF_8,
        };
        encoding
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(|s| s.into_owned())
            .ok_or(FormatError("invalid title bytes for selected encoding"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub fn decode_icon(bytes: &[u8]) -> Result<IconPixels, Box<dyn Error>> {
    if bytes.len() < 6 || bytes.len() > MAX_PLATFORM_ASSET || bytes[..4] != [0, 0, 1, 0] {
        return Err("invalid or oversized ICO".into());
    }
    let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let end = 6 + count * 16;
    if count == 0 || count > 256 || end > bytes.len() {
        return Err("invalid ICO directory size".into());
    }
    // Preflight embedded dimensions before invoking a raster decoder or allocating RGBA.
    for e in bytes[6..end].chunks_exact(16) {
        let w = if e[0] == 0 { 256 } else { u32::from(e[0]) };
        let h = if e[1] == 0 { 256 } else { u32::from(e[1]) };
        let n = u64::from(u32::from_le_bytes(e[8..12].try_into()?));
        let at = u64::from(u32::from_le_bytes(e[12..16].try_into()?));
        if e[3] != 0 || at < end as u64 || at + n > bytes.len() as u64 {
            return Err("ICO raster range outside input".into());
        }
        let image = &bytes[at as usize..(at + n) as usize];
        if image.starts_with(b"\x89PNG\r\n\x1a\n") {
            if image.len() < 24
                || u32::from_be_bytes(image[16..20].try_into()?) != w
                || u32::from_be_bytes(image[20..24].try_into()?) != h
            {
                return Err("ICO PNG dimensions disagree".into());
            }
        } else if image.len() < 40
            || u32::from_le_bytes(image[..4].try_into()?) != 40
            || u32::from_le_bytes(image[4..8].try_into()?) != w
            || u32::from_le_bytes(image[8..12].try_into()?) != 2 * h
        {
            return Err("unsupported ICO DIB dimensions".into());
        }
    }
    let dir = ico::IconDir::read(Cursor::new(bytes))?;
    let entry = dir
        .entries()
        .iter()
        .min_by_key(|e| {
            (
                e.width().abs_diff(32) + e.height().abs_diff(32),
                std::cmp::Reverse(e.bits_per_pixel()),
            )
        })
        .ok_or("ICO has no images")?;
    let image = entry.decode()?;
    Ok(IconPixels {
        width: image.width(),
        height: image.height(),
        rgba: image.rgba_data().to_vec(),
    })
}

pub fn cursor_rgba(bitmap: &CursorBitmap) -> Result<Vec<u8>, FormatError> {
    let mut rgba = Vec::with_capacity(bitmap.width() * bitmap.height() * 4);
    for (rgb, and) in bitmap.xor_rgb().iter().zip(bitmap.and_mask()) {
        if *and && *rgb != [0, 0, 0] {
            return Err(FormatError(
                "native RGBA cursor cannot represent AND/XOR inversion",
            ));
        }
        rgba.extend_from_slice(rgb);
        rgba.push(if *and { 0 } else { 255 });
    }
    Ok(rgba)
}

pub struct NativeBackend<'a> {
    pub window: &'a Window,
    pub event_loop: &'a ActiveEventLoop,
    pub encoding: TextEncoding,
}
impl PlatformBackend for NativeBackend<'_> {
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        match request {
            PlatformRequest::Title(bytes) => {
                let title = self.encoding.decode(bytes)?;
                self.window.set_title(&title);
                println!("# native title={title}");
            }
            PlatformRequest::Icon { bytes, .. } => {
                let image = decode_icon(bytes)?;
                let icon = Icon::from_rgba(image.rgba, image.width, image.height)?;
                self.window.set_window_icon(Some(icon));
                println!("# native icon={}x{}", image.width, image.height);
            }
            PlatformRequest::Cursor { id, bitmap } => {
                if let Some(bitmap) = bitmap {
                    let rgba = cursor_rgba(bitmap)?;
                    let (hx, hy) = bitmap.hotspot();
                    let source = CustomCursor::from_rgba(
                        rgba,
                        bitmap.width() as u16,
                        bitmap.height() as u16,
                        hx,
                        hy,
                    )?;
                    let cursor = self.event_loop.create_custom_cursor(source);
                    self.window.set_cursor(cursor);
                } else {
                    self.window.set_cursor(CursorIcon::Default);
                }
                println!("# native cursor={id}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Key {
        physical: String,
        pressed: bool,
        repeat: bool,
    },
    Pointer {
        x: f64,
        y: f64,
    },
    Button {
        button: String,
        pressed: bool,
    },
    Focus(bool),
    Wheel {
        x: f64,
        y: f64,
    },
}
#[derive(Debug, Default)]
pub struct DesktopInput {
    events: VecDeque<InputEvent>,
    keys: BTreeSet<String>,
    buttons: BTreeSet<String>,
    dropped: u64,
    counts: InputCounts,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputCounts {
    pub keys: u64,
    pub pointer: u64,
    pub buttons: u64,
    pub focus: u64,
    pub wheel: u64,
}
impl DesktopInput {
    pub fn push(&mut self, event: InputEvent) {
        match &event {
            InputEvent::Key {
                physical, pressed, ..
            } => {
                self.counts.keys += 1;
                if *pressed {
                    self.keys.insert(physical.clone());
                } else {
                    self.keys.remove(physical);
                }
            }
            InputEvent::Button { button, pressed } => {
                self.counts.buttons += 1;
                if *pressed {
                    self.buttons.insert(button.clone());
                } else {
                    self.buttons.remove(button);
                }
            }
            InputEvent::Focus(focused) => {
                self.counts.focus += 1;
                if !focused {
                    self.keys.clear();
                    self.buttons.clear();
                }
            }
            InputEvent::Pointer { .. } => self.counts.pointer += 1,
            InputEvent::Wheel { .. } => self.counts.wheel += 1,
        }
        if self.events.len() == 256 {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }
    pub fn events(&self) -> &VecDeque<InputEvent> {
        &self.events
    }
    pub fn held_keys(&self) -> &BTreeSet<String> {
        &self.keys
    }
    pub fn held_buttons(&self) -> &BTreeSet<String> {
        &self.buttons
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn counts(&self) -> InputCounts {
        self.counts
    }
}
